//! Optional non-blocking Discord Rich Presence adapter.
//!
//! This module consumes the privacy-safe [`PresenceStatus`] payload from
//! [`crate::presence`] and forwards it to the locally-installed Discord desktop
//! client over its RPC IPC socket (Unix domain socket on Linux/macOS, named
//! pipe on Windows).
//!
//! Design guarantees:
//! 1. **Non-blocking** — the UI/capture/encode paths only push a message into
//!    an unbounded channel; all IPC I/O happens on a dedicated worker thread.
//! 2. **Explicit opt-out** — the master switch in [`DiscordPresenceConfig`]
//!    defaults to on (so nothing is transmitted until a status change), but a
//!    clear toggle in Settings disables the whole adapter and no thread is
//!    spawned.
//! 3. **Privacy-safe** — only the application name, a localized activity label
//!    and an optional user-selected game name are sent; stream keys, ingest
//!    URLs, paths and window titles never leave the process.
//! 4. **Graceful degradation** — if Discord is not running, the worker simply
//!    retries on the next activity transition; it never blocks nor panics the
//!    application.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use serde::Serialize;

use crate::presence::PresenceStatus;

/// Placeholder application id. Replace with the real Rivulet Discord Developer
/// Application id before shipping to end users.
pub const DEFAULT_CLIENT_ID: &str = "0000000000000000000";

/// Discord Rich Presence configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordPresenceConfig {
    /// Master switch (explicit opt-out). When `false` no worker is spawned.
    pub enabled: bool,
    /// Discord Developer Application client id.
    pub client_id: String,
}

impl Default for DiscordPresenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            client_id: DEFAULT_CLIENT_ID.to_owned(),
        }
    }
}

/// Messages sent from the (fast) caller side to the (blocking) worker thread.
enum Msg {
    Activity(Box<PresenceStatus>),
    Disconnect,
}

/// Handle to a running adapter. Cloning is not supported; the handle owns the
/// worker lifecycle. Non-blocking by construction: [`Self::set_activity`] only
/// enqueues a message and returns immediately.
pub struct DiscordPresence {
    tx: Option<Sender<Msg>>,
    stop: Arc<AtomicBool>,
    enabled: bool,
}

impl DiscordPresence {
    /// Spawn the worker thread when the configuration is valid. Returns a
    /// disabled handle (no thread) when `enabled` is `false` or the client id
    /// is empty.
    pub fn new(config: &DiscordPresenceConfig) -> Self {
        if !config.enabled || config.client_id.trim().is_empty() {
            return Self {
                tx: None,
                stop: Arc::new(AtomicBool::new(true)),
                enabled: false,
            };
        }
        let (tx, rx) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let cfg = config.clone();
        let spawned = std::thread::Builder::new()
            .name("rivulet-discord-presence".to_owned())
            .spawn(move || worker_loop(rx, cfg, worker_stop))
            .is_ok();
        if !spawned {
            return Self {
                tx: None,
                stop: Arc::new(AtomicBool::new(true)),
                enabled: false,
            };
        }
        Self {
            tx: Some(tx),
            stop,
            enabled: true,
        }
    }

    /// Whether a worker is actually running (i.e. the feature is active).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Push a status update to the worker. Returns `false` when the adapter is
    /// disabled. Never blocks.
    pub fn set_activity(&self, status: &PresenceStatus) -> bool {
        match &self.tx {
            Some(tx) => {
                let _ = tx.try_send(Msg::Activity(Box::new(status.clone())));
                true
            }
            None => false,
        }
    }

    /// Stop the worker and clear any active presence. Safe to call repeatedly.
    pub fn disconnect(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Disconnect);
        }
        self.enabled = false;
        self.tx = None;
    }
}

impl Drop for DiscordPresence {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Disconnect);
        }
    }
}

fn worker_loop(rx: Receiver<Msg>, cfg: DiscordPresenceConfig, stop: Arc<AtomicBool>) {
    let pid = std::process::id();
    let mut stream: Option<Box<dyn IpcStream>> = None;
    let mut last: Option<PresenceStatus> = None;
    let mut seq: u64 = 0;

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let pending = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Msg::Activity(status)) if Some(status.as_ref()) != last.as_ref() => {
                last = Some(status.as_ref().clone());
                Some(status)
            }
            Ok(Msg::Activity(_)) => None, // duplicate transition, already sent
            Ok(Msg::Disconnect) => break,
            Err(_) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                None
            }
        };
        let Some(status) = pending else { continue };

        // Non-blocking for the caller: this I/O happens on the worker thread and
        // only when an activity transition is pending.
        let result = ensure_connected(&mut stream, &cfg.client_id)
            .and_then(|_| send_set_activity(stream.as_mut().unwrap(), &status, pid, &mut seq));
        if result.is_err() {
            // Discord gone or the socket died: drop it and reconnect lazily on
            // the next transition.
            stream = None;
        }
    }
}

fn ensure_connected(stream: &mut Option<Box<dyn IpcStream>>, client_id: &str) -> io::Result<()> {
    if stream.is_some() {
        return Ok(());
    }
    let mut s = match open_platform_stream() {
        Ok(s) => s,
        Err(e) => {
            // Discord is unavailable; back off on the worker thread so we do
            // not spin. The caller side is unaffected.
            std::thread::sleep(Duration::from_secs(5));
            return Err(e);
        }
    };
    send_handshake(&mut *s, client_id)?;
    *stream = Some(s);
    Ok(())
}

// ---------------------------------------------------------------------------
// Wire format
// ---------------------------------------------------------------------------

/// Friending trait so we can hold either a `UnixStream` or a Windows named pipe
/// behind one type-erased handle.
pub(crate) trait IpcStream: Read + Write + Send {}
impl<T: Read + Write + Send> IpcStream for T {}

/// Connect to the Discord IPC endpoint for this platform.
fn open_platform_stream() -> io::Result<Box<dyn IpcStream>> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let mut last_err = None;
        for path in ipc_socket_paths() {
            match UnixStream::connect(path) {
                Ok(s) => {
                    let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
                    return Ok(Box::new(s));
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err
            .unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no discord ipc socket")))
    }
    #[cfg(windows)]
    {
        winipc::connect().map(|s| -> Box<dyn IpcStream> { Box::new(s) })
    }
}

/// Candidate unix socket paths, in preference order.
#[cfg(unix)]
fn ipc_socket_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        paths.push(std::path::Path::new(&dir).join("discord-ipc-0"));
    }
    if let Ok(dir) = std::env::var("TMPDIR") {
        paths.push(std::path::Path::new(&dir).join("discord-ipc-0"));
    }
    paths.push(std::path::PathBuf::from("/tmp/discord-ipc-0"));
    paths
}

/// One IPC frame: 4-byte little-endian length prefix, then the JSON payload.
fn write_frame<W: Write + ?Sized>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    let len = payload.len() as u32;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(payload)
}

/// Read one Discord IPC frame (length prefix + JSON payload).
#[cfg(test)]
fn read_frame<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut header = [0u8; 4];
    r.read_exact(&mut header)?;
    let len = u32::from_le_bytes(header) as usize;
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok(payload)
}

fn send_handshake<W: Write + ?Sized>(w: &mut W, client_id: &str) -> io::Result<()> {
    let payload = serde_json::json!({
        "v": 1,
        "client_id": client_id,
    });
    let bytes =
        serde_json::to_vec(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_frame(w, &bytes)
}

#[derive(Serialize)]
struct ActivityCommand {
    cmd: &'static str,
    args: ActivityArgs,
    nonce: String,
}

#[derive(Serialize)]
struct ActivityArgs {
    pid: u32,
    activity: Activity,
}

#[derive(Serialize)]
struct Activity {
    #[serde(rename = "type")]
    r#type: u32,
    state: String,
    details: String,
}

impl ActivityCommand {
    /// Build a `SET_ACTIVITY` command from the privacy-safe status payload.
    /// `state`/`details` are truncated to Discord's 128-char limit at a UTF-8
    /// character boundary.
    fn new(status: &PresenceStatus, pid: u32, seq: u64) -> Self {
        ActivityCommand {
            cmd: "SET_ACTIVITY",
            args: ActivityArgs {
                pid,
                activity: Activity {
                    r#type: 0, // 0 == "Playing" / Game presence
                    state: truncate(&status.state),
                    details: truncate(&status.details),
                },
            },
            nonce: format!("rivulet-{seq}"),
        }
    }
}

/// Truncate to at most 128 characters, honoring UTF-8 boundaries.
fn truncate(text: &str) -> String {
    let mut len = text.len().min(128);
    while !text.is_char_boundary(len) {
        len -= 1;
    }
    text[..len].to_owned()
}

fn send_set_activity<W: Write + ?Sized>(
    w: &mut W,
    status: &PresenceStatus,
    pid: u32,
    seq: &mut u64,
) -> io::Result<()> {
    *seq += 1;
    let cmd = ActivityCommand::new(status, pid, *seq);
    let bytes =
        serde_json::to_vec(&cmd).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_frame(w, &bytes)
}

// ---------------------------------------------------------------------------
// Windows named-pipe transport
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod winipc {
    use std::io::{self, Read, Write};
    use winapi::um::fileapi::{CreateFileW, WriteFile, OPEN_EXISTING};
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::namedpipeapi::WaitNamedPipeW;
    use winapi::um::winnt::{FILE_ATTRIBUTE_NORMAL, GENERIC_READ, GENERIC_WRITE, HANDLE};

    const PIPE_NAME: &str = r"\\.\pipe\discord-ipc-0";

    pub struct NamedPipe {
        handle: HANDLE,
    }

    // The handle is owned exclusively by this value and only ever touched on
    // the thread that constructs it (the presence worker), so it is safe to
    // treat as Send so the boxed stream can live on the worker thread.
    unsafe impl Send for NamedPipe {}

    impl Drop for NamedPipe {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }

    pub fn connect() -> io::Result<NamedPipe> {
        let wide: Vec<u16> = PIPE_NAME.encode_utf16().chain(Some(0)).collect();
        let first = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if std::ptr::addr_eq(first, INVALID_HANDLE_VALUE) {
            // Discord may still be creating the server pipe: wait up to ~3s.
            let mut handle = std::ptr::null_mut();
            for _ in 0..6 {
                let available = unsafe { WaitNamedPipeW(wide.as_ptr(), 500) };
                if available == 0 {
                    break;
                }
                let h = unsafe {
                    CreateFileW(
                        wide.as_ptr(),
                        GENERIC_READ | GENERIC_WRITE,
                        0,
                        std::ptr::null_mut(),
                        OPEN_EXISTING,
                        FILE_ATTRIBUTE_NORMAL,
                        std::ptr::null_mut(),
                    )
                };
                if !std::ptr::addr_eq(h, INVALID_HANDLE_VALUE) {
                    handle = h;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            return Ok(NamedPipe { handle });
        }
        Ok(NamedPipe { handle: first })
    }

    impl Read for NamedPipe {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            // The adapter never reads from Discord's pipe (handshake ack is
            // intentionally skipped to keep the worker non-blocking).
            Ok(0)
        }
    }

    impl Write for NamedPipe {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut written: u32 = 0;
            let ok = unsafe {
                WriteFile(
                    self.handle,
                    buf.as_ptr() as *const _,
                    buf.len() as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(written as usize)
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presence::{PresenceActivity, PresenceStatus};

    fn status() -> PresenceStatus {
        PresenceStatus::for_activity(PresenceActivity::Recording)
    }

    #[test]
    fn disabled_config_spawns_no_worker() {
        let cfg = DiscordPresenceConfig {
            enabled: false,
            client_id: "abc".to_owned(),
        };
        let presence = DiscordPresence::new(&cfg);
        assert!(!presence.enabled());
        assert!(!presence.set_activity(&status()));
    }

    #[test]
    fn empty_client_id_disables_the_adapter() {
        let cfg = DiscordPresenceConfig {
            enabled: true,
            client_id: String::new(),
        };
        let presence = DiscordPresence::new(&cfg);
        assert!(!presence.enabled());
    }

    #[test]
    fn default_config_is_opt_out_with_placeholder_client_id() {
        let cfg = DiscordPresenceConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.client_id, DEFAULT_CLIENT_ID);
    }

    #[test]
    fn set_activity_is_non_blocking_without_discord() {
        let cfg = DiscordPresenceConfig {
            enabled: true,
            client_id: "dummy-client".to_owned(),
        };
        let mut presence = DiscordPresence::new(&cfg);
        assert!(presence.enabled());
        // Many rapid updates must return immediately (worker sleeps on connect)
        // and never block the caller.
        for _ in 0..500 {
            assert!(presence.set_activity(&status()));
        }
        presence.disconnect();
        assert!(!presence.enabled());
    }

    #[test]
    fn disconnect_is_idempotent() {
        let cfg = DiscordPresenceConfig {
            enabled: true,
            client_id: "dummy-client".to_owned(),
        };
        let mut presence = DiscordPresence::new(&cfg);
        presence.disconnect();
        presence.disconnect();
        assert!(!presence.set_activity(&status()));
    }

    #[test]
    fn handshake_payload_matches_discord_schema() {
        let bytes = {
            let mut buf = Vec::new();
            send_handshake(&mut buf, "1111111111111111111").unwrap();
            buf
        };
        // Strip the 4-byte length prefix for serialization checking.
        let payload = &bytes[4..];
        let value: serde_json::Value = serde_json::from_slice(payload).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["client_id"], "1111111111111111111");
    }

    #[test]
    fn set_activity_payload_carries_privacy_safe_fields() {
        let mut buf = Vec::new();
        let st = PresenceStatus::for_activity_localized(
            PresenceActivity::Streaming,
            crate::Locale::En,
            Some("Elden Ring"),
        );
        let mut seq = 7;
        send_set_activity(&mut buf, &st, 1234, &mut seq).unwrap();

        let payload = &buf[4..];
        let value: serde_json::Value = serde_json::from_slice(payload).unwrap();
        assert_eq!(value["cmd"], "SET_ACTIVITY");
        assert_eq!(value["args"]["pid"], 1234);
        assert_eq!(value["args"]["activity"]["type"], 0);
        assert_eq!(value["args"]["activity"]["details"], "Rivulet · Streaming");
        assert_eq!(value["args"]["activity"]["state"], "Streaming · Elden Ring");
        assert!(!value["nonce"].as_str().unwrap().is_empty());
        assert!(seq > 7);
    }

    #[test]
    fn payload_never_contains_sensitive_data() {
        let st = PresenceStatus::for_activity(PresenceActivity::Streaming);
        let mut buf = Vec::new();
        let mut seq = 0;
        send_set_activity(&mut buf, &st, 1, &mut seq).unwrap();
        let text = String::from_utf8(buf[4..].to_vec()).unwrap();
        assert!(!text.to_lowercase().contains("rtmp"));
        assert!(!text.to_lowercase().contains("key"));
        assert!(!text.contains('/') && !text.contains('\\'));
    }

    #[test]
    fn truncate_respects_utf8_boundaries() {
        let long = "éééééééééé".repeat(200); // multi-byte chars past 128 chars
        let truncated = truncate(&long);
        assert!(truncated.len() <= 128);
        assert!(truncated.is_char_boundary(truncated.len()));
        assert_eq!(truncated.len(), 128);
    }

    #[test]
    fn frame_wires_length_prefix_then_payload() {
        let mut buf = Vec::new();
        let payload = b"hello-discord";
        write_frame(&mut buf, payload).unwrap();
        assert_eq!(buf.len(), 4 + payload.len());
        let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        assert_eq!(len, payload.len());
        assert_eq!(&buf[4..], payload);
    }

    #[test]
    fn read_frame_parses_wire_format() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"{\"cmd\":\"ping\"}").unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let payload = read_frame(&mut cursor).unwrap();
        assert_eq!(payload, b"{\"cmd\":\"ping\"}");
    }

    #[cfg(unix)]
    #[test]
    fn write_and_read_round_trip_on_unix_socket() {
        use std::os::unix::net::UnixStream;
        let (mut a, mut b) = UnixStream::pair().unwrap();
        let st = status();
        let mut seq = 1;
        let _ = a.set_write_timeout(Some(Duration::from_secs(2)));
        let _ = b.set_read_timeout(Some(Duration::from_secs(2)));
        send_set_activity(&mut a, &st, 42, &mut seq).unwrap();
        let frame = read_frame(&mut b).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(value["cmd"], "SET_ACTIVITY");
        assert_eq!(value["args"]["pid"], 42);
    }
}
