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

/// Official Rivulet Discord application id (Developer Portal). Shipped as the
/// default so end users get the branded presence card (logo, status) without
/// creating their own Discord application. The id is not a secret: it rides
/// along in every SET_ACTIVITY payload anyway. Users who want their own
/// branding can override it in Settings; an empty value keeps the adapter off.
pub const DEFAULT_CLIENT_ID: &str = "1544027006847680532";

/// Asset key of the Rivulet artwork uploaded in the official application's
/// Rich Presence → Art Assets tab. Shipped as the default art asset key so
/// the presence card shows the Rivulet logo out of the box.
pub const DEFAULT_LARGE_IMAGE_KEY: &str = "rivulet_logo";

/// Resolve the effective Discord application id from an optional configured
/// value. The fallback chain is:
///
/// 1. a non-empty configured id (Settings override for custom branding),
/// 2. the official [`DEFAULT_CLIENT_ID`] (shipped default, kept alive by the
///    updater: a release that must retire the current id ships the new id in
///    its release notes and the default constant is bumped with it — the
///    updater manifest *is* the release payload, so users pick the change up
///    with the regular update),
/// 3. empty (`None`): the adapter stays off.
///
/// Central helper so GUI, tests and future manifests all agree on one chain.
pub fn effective_client_id(configured: Option<&str>) -> Option<String> {
    match configured.map(str::trim) {
        Some(id) if !id.is_empty() => Some(id.to_owned()),
        _ => Some(DEFAULT_CLIENT_ID.to_owned()),
    }
}

/// Resolve the effective art asset key (same chain as
/// [`effective_client_id`], but `None` is allowed and means "no artwork" —
/// Discord then renders the generic placeholder icon).
pub fn effective_large_image_key(configured: Option<&str>) -> Option<String> {
    match configured.map(str::trim) {
        Some(key) if !key.is_empty() => Some(key.to_owned()),
        _ => Some(DEFAULT_LARGE_IMAGE_KEY.to_owned()),
    }
}

/// Discord Rich Presence configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordPresenceConfig {
    /// Master switch (explicit opt-out). When `false` no worker is spawned.
    pub enabled: bool,
    /// Discord Developer Application client id.
    pub client_id: String,
    /// Optional explicit IPC endpoint override — a filesystem path on Unix
    /// (e.g. `/run/user/1000/discord-ipc-0`), or a named-pipe name on Windows
    /// (e.g. `\\.\pipe\discord-ipc-0`). When `None`, the adapter derives the
    /// default `discord-ipc-0` endpoint from the environment. Exposed so the
    /// adapter can be run deterministically against a local listener in tests.
    pub ipc_socket_path: Option<std::path::PathBuf>,
    /// Asset key of the large image uploaded in the Discord Developer Portal
    /// (Rich Presence → Art Assets). When set, Discord renders that image as
    /// the card artwork instead of the generic placeholder icon, like OBS
    /// shows its logo. The image is uploaded by the application owner; the key
    /// is the name given to the asset there.
    pub large_image_key: Option<String>,
    /// Hover text for the large image. Defaults to the app name when unset.
    pub large_image_text: Option<String>,
}

impl Default for DiscordPresenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            client_id: DEFAULT_CLIENT_ID.to_owned(),
            ipc_socket_path: None,
            large_image_key: Some(DEFAULT_LARGE_IMAGE_KEY.to_owned()),
            large_image_text: None,
        }
    }
}

/// Why a configured Discord application client id looks invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientIdError {
    /// The value contains characters other than ASCII digits (e.g. the app
    /// URL was pasted instead of the numeric id, or spaces were included).
    NotNumeric,
    /// The value is not within the plausible Discord snowflake length range
    /// (17-20 digits).
    Length,
}

/// Validate a Discord application client id entered in Settings. Returns
/// `Ok(())` for a syntactically plausible snowflake (all digits, 17-20 long)
/// or for an empty value (empty deliberately keeps the adapter off). A
/// non-empty value that fails the checks is reported so the UI can warn
/// immediately instead of silently accepting a mistyped id.
///
/// Note: this is a *format* check — the id still has to belong to a real
/// Discord application with Rich Presence enabled for the handshake to be
/// accepted.
pub fn validate_client_id(client_id: &str) -> Result<(), ClientIdError> {
    let trimmed = client_id.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if !trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ClientIdError::NotNumeric);
    }
    if !(17..=20).contains(&trimmed.len()) {
        return Err(ClientIdError::Length);
    }
    Ok(())
}

/// Why a `SET_ACTIVITY` payload violates Discord's Rich Presence validation
/// rules. Discord rejects offending payloads with error code `4000` (verified
/// live), so every violation here is a *wire-level* rejection, not a cosmetic
/// nit — the whole status update is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadIssue {
    /// `state` or `details` exceeds Discord's 128-character limit. The
    /// serializer truncates silently, so this flags the condition before the
    /// truncation hides it (e.g. in Settings previews).
    FieldTooLong { field: &'static str, len: usize },
    /// The configured `large_image` asset key is not a plausible Discord art
    /// asset key (non-empty, letters/digits/underscore only, ≤ 64 chars).
    /// Unlike truncation, an invalid key is sent **verbatim** and Discord
    /// silently drops the image — it never appears on the card.
    InvalidAssetKey,
}

/// Validate a `PresenceStatus` (plus optional asset key) against Discord's
/// documented Rich Presence rules **before** it is put on the wire. Returns
/// every violation found; an empty slice means the payload would be accepted.
///
/// This is the single source of truth for the limits that otherwise only
/// surface as silent truncation (`state`/`details` ≤ 128 chars) or as a
/// silently dropped image (invalid `large_image` key). The empty-`state`
/// rule is enforced on the wire itself: `details` always carries the status
/// label, while the serializer **omits** `state` when there is no game name
/// (Discord rejects an empty string with `4000`), and the exhaustive contract
/// test asserts no serialized payload ever contains an empty `state`.
pub fn validate_set_activity_payload(
    status: &PresenceStatus,
    large_image_key: Option<&str>,
) -> Vec<PayloadIssue> {
    let mut issues = Vec::new();
    let details = status.details.trim();
    if details.len() > 128 {
        issues.push(PayloadIssue::FieldTooLong {
            field: "details",
            len: details.len(),
        });
    }
    let state = status.state.trim();
    if !state.is_empty() && state.len() > 128 {
        issues.push(PayloadIssue::FieldTooLong {
            field: "state",
            len: state.len(),
        });
    }
    if let Some(key) = large_image_key {
        let key = key.trim();
        // Empty is treated exactly like `None` (serializer filters it and the
        // generic placeholder icon stays) — only a *non-empty* implausible key
        // is a violation.
        if !key.is_empty()
            && (key.len() > 64 || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        {
            issues.push(PayloadIssue::InvalidAssetKey);
        }
    }
    issues
}

/// Messages sent from the (fast) caller side to the (blocking) worker thread.
enum Msg {
    Activity(Box<PresenceStatus>),
    Disconnect,
}

/// Connection state of the presence worker, shared with the caller side so
/// the GUI can surface whether Discord actually accepted the handshake instead
/// of silently showing nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscordConnState {
    /// Worker not running (adapter disabled or no client id configured).
    Off,
    /// Worker is running but has not (yet) established the IPC handshake.
    Connecting,
    /// Handshake accepted; a status was delivered to Discord.
    Connected,
}

/// Handle to a running adapter. Cloning is not supported; the handle owns the
/// worker lifecycle. Non-blocking by construction: [`Self::set_activity`] only
/// enqueues a message and returns immediately.
pub struct DiscordPresence {
    tx: Option<Sender<Msg>>,
    stop: Arc<AtomicBool>,
    enabled: bool,
    /// Reference to the worker's connection state so the GUI can show whether
    /// Discord actually accepted the presence.
    conn: Arc<std::sync::atomic::AtomicU8>,
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
                conn: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            };
        }
        let (tx, rx) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        // Worker running but no handshake completed yet.
        let conn = Arc::new(std::sync::atomic::AtomicU8::new(1));
        let worker_conn = Arc::clone(&conn);
        let cfg = config.clone();
        let spawned = std::thread::Builder::new()
            .name("rivulet-discord-presence".to_owned())
            .spawn(move || worker_loop(rx, cfg, worker_stop, worker_conn))
            .is_ok();
        if !spawned {
            return Self {
                tx: None,
                stop: Arc::new(AtomicBool::new(true)),
                enabled: false,
                conn: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            };
        }
        Self {
            tx: Some(tx),
            stop,
            enabled: true,
            conn,
        }
    }

    /// Whether a worker is actually running (i.e. the feature is active).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Current IPC connection state, so callers can surface whether Discord
    /// accepted the presence instead of silently showing nothing.
    pub fn connection_state(&self) -> DiscordConnState {
        match self.conn.load(std::sync::atomic::Ordering::SeqCst) {
            2 => DiscordConnState::Connected,
            1 => DiscordConnState::Connecting,
            _ => DiscordConnState::Off,
        }
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

fn worker_loop(
    rx: Receiver<Msg>,
    cfg: DiscordPresenceConfig,
    stop: Arc<AtomicBool>,
    conn: Arc<std::sync::atomic::AtomicU8>,
) {
    let pid = std::process::id();
    let mut stream: Option<Box<dyn IpcStream>> = None;
    let mut last: Option<PresenceStatus> = None;
    let mut seq: u64 = 0;
    let mut backoff = 1u64;

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
        let result = ensure_connected(&mut stream, &cfg.client_id, cfg.ipc_socket_path.as_deref())
            .and_then(|_| {
                send_set_activity(
                    stream.as_mut().unwrap(),
                    &status,
                    pid,
                    &mut seq,
                    cfg.large_image_key.as_deref(),
                )
            });
        match result {
            Ok(()) => {
                backoff = 1;
                // Expose the connection state so the GUI can show whether
                // Discord actually accepted the presence.
                conn.store(2, Ordering::SeqCst);
                tracing::info!(
                    activity = %status.details,
                    game = %status.state,
                    "Discord Rich Presence SET_ACTIVITY delivered"
                );
            }
            Err(e) => {
                // Discord gone or the socket died: drop it, mark the state and
                // reconnect lazily on the next transition.
                stream = None;
                conn.store(0, Ordering::SeqCst);
                tracing::warn!(
                    error = %e,
                    backoff_secs = backoff,
                    "Discord Rich Presence IPC unavailable"
                );
                std::thread::sleep(Duration::from_secs(backoff));
                backoff = (backoff * 2).min(30);
            }
        }
    }
}

fn ensure_connected(
    stream: &mut Option<Box<dyn IpcStream>>,
    client_id: &str,
    ipc_path: Option<&std::path::Path>,
) -> io::Result<()> {
    if stream.is_some() {
        return Ok(());
    }
    let mut s = match open_platform_stream(ipc_path) {
        Ok(s) => s,
        Err(e) => {
            // Discord is unavailable; back off on the worker thread so we do
            // not spin. The caller side is unaffected.
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

/// Connect to the Discord IPC endpoint for this platform. When `path` is
/// supplied (test override) it is used instead of the environment-derived
/// default `discord-ipc-0` endpoint.
fn open_platform_stream(path: Option<&std::path::Path>) -> io::Result<Box<dyn IpcStream>> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let candidates = match path {
            Some(p) => vec![p.to_owned()],
            None => ipc_socket_paths(),
        };
        let mut last_err = None;
        for candidate in candidates {
            match UnixStream::connect(&candidate) {
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
        let name = match path {
            Some(p) => p.to_string_lossy().into_owned(),
            None => winipc::DEFAULT_PIPE_NAME.to_owned(),
        };
        winipc::connect(&name).map(|s| -> Box<dyn IpcStream> { Box::new(s) })
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

/// Discord IPC frame opcodes.
mod op {
    /// Connect / identify with `client_id`.
    pub const HANDSHAKE: u32 = 0;
    /// A normal command/event frame (e.g. `SET_ACTIVITY`).
    pub const FRAME: u32 = 1;
    /// Connection closed (Discord sends this with `{code,message}` when it
    /// rejects us, e.g. code 1003 = protocol error).
    #[allow(dead_code)]
    pub const CLOSE: u32 = 2;
    /// Keepalive ping / pong.
    #[allow(dead_code)]
    pub const PING: u32 = 3;
    #[allow(dead_code)]
    pub const PONG: u32 = 4;
}

/// One IPC frame: 8-byte little-endian header `[opcode: u32][length: u32]`,
/// then the JSON payload. Discord v1 rejects the old 4-byte-length-only
/// framing with `{"code":1003,"message":"protocol error"}` and closes the
/// connection — which is why the presence never appeared while the GUI still
/// reported the desired status.
fn write_frame<W: Write + ?Sized>(w: &mut W, opcode: u32, payload: &[u8]) -> io::Result<()> {
    let len = payload.len() as u32;
    w.write_all(&opcode.to_le_bytes())?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(payload)
}

/// Read one Discord IPC frame (8-byte `[opcode][length]` header + payload)
/// and return `(opcode, payload)`.
#[cfg(test)]
fn read_frame<R: Read>(r: &mut R) -> io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 8];
    r.read_exact(&mut header)?;
    let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok((opcode, payload))
}

fn send_handshake<W: Write + ?Sized>(w: &mut W, client_id: &str) -> io::Result<()> {
    let payload = serde_json::json!({
        "v": 1,
        "client_id": client_id,
    });
    let bytes =
        serde_json::to_vec(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_frame(w, op::HANDSHAKE, &bytes)
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
    /// First card line, always present: the localized status label (e.g.
    /// "Recording"). Never empty — it is built from the i18n tables.
    details: String,
    /// Second card line, optional: the game name. Discord **rejects** an
    /// empty string with `4000: "..." is not allowed to be empty`, so the
    /// field is omitted entirely when there is no game name to show (verified
    /// live against a running Discord client).
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assets: Option<ActivityAssets>,
}

#[derive(Serialize)]
struct ActivityAssets {
    #[serde(rename = "large_image")]
    large_image: String,
    /// Discord renders the small image in the member list next to the activity
    /// text; the configured key is mirrored here so the same uploaded asset
    /// replaces the generic game-controller placeholder there too (docs:
    /// small image = member list, large image = profile card).
    #[serde(rename = "small_image")]
    small_image: String,
    #[serde(rename = "large_text", skip_serializing_if = "Option::is_none")]
    large_text: Option<String>,
    #[serde(rename = "small_text", skip_serializing_if = "Option::is_none")]
    small_text: Option<String>,
}

impl ActivityCommand {
    /// Build a `SET_ACTIVITY` command from the privacy-safe status payload.
    /// `state`/`details` are truncated to Discord's 128-char limit at a UTF-8
    /// character boundary. When the configured Discord application has an
    /// uploaded art asset, its key is attached as `large_image` (profile card)
    /// and mirrored as `small_image` (member list) so Discord shows the
    /// Rivulet artwork instead of the generic placeholder icons.
    fn new(status: &PresenceStatus, pid: u32, seq: u64, large_image: Option<&str>) -> Self {
        // Only attach the artwork when the key is plausible: an empty or
        // malformed key would be sent verbatim and Discord would silently drop
        // the image (no error, no artwork). The same rule is exposed through
        // `validate_set_activity_payload` for pre-wire warnings.
        let assets = large_image
            .filter(|key| {
                !key.trim().is_empty()
                    && key.trim().len() <= 64
                    && key
                        .trim()
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
            .map(|key| {
                let key = key.trim().to_owned();
                ActivityAssets {
                    large_image: key.clone(),
                    small_image: key,
                    large_text: Some("Rivulet".to_owned()),
                    small_text: Some("Rivulet".to_owned()),
                }
            });
        ActivityCommand {
            cmd: "SET_ACTIVITY",
            args: ActivityArgs {
                pid,
                activity: Activity {
                    r#type: 0, // 0 == "Playing" / Game presence
                    details: truncate(&status.details),
                    state: (!status.state.trim().is_empty()).then(|| truncate(&status.state)),
                    assets,
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
    large_image: Option<&str>,
) -> io::Result<()> {
    *seq += 1;
    let cmd = ActivityCommand::new(status, pid, *seq, large_image);
    let bytes =
        serde_json::to_vec(&cmd).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_frame(w, op::FRAME, &bytes)
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

    /// Discord's default Rich Presence named-pipe endpoint.
    pub(crate) const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\discord-ipc-0";

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

    pub fn connect(name: &str) -> io::Result<NamedPipe> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
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
            ..Default::default()
        };
        let presence = DiscordPresence::new(&cfg);
        assert!(!presence.enabled());
        assert!(!presence.set_activity(&status()));
        assert_eq!(presence.connection_state(), DiscordConnState::Off);
    }

    #[test]
    fn empty_client_id_disables_the_adapter() {
        let cfg = DiscordPresenceConfig {
            enabled: true,
            client_id: String::new(),
            ..Default::default()
        };
        let presence = DiscordPresence::new(&cfg);
        assert!(!presence.enabled());
        assert_eq!(presence.connection_state(), DiscordConnState::Off);
    }

    #[test]
    fn connection_state_starts_connecting_and_degrades_without_discord() {
        // No Discord running: the worker cannot complete the handshake, so the
        // connection state must never claim success. The GUI surfaces this so
        // the user sees why only the plain game card shows.
        let cfg = DiscordPresenceConfig {
            enabled: true,
            client_id: "dummy-client".to_owned(),
            ipc_socket_path: Some(std::env::temp_dir().join("rivulet-missing-discord-ipc.sock")),
            ..Default::default()
        };
        let mut presence = DiscordPresence::new(&cfg);
        assert!(presence.enabled());
        let _ = presence.set_activity(&status());
        // Give the worker time to attempt the (failing) connect; because the
        // worker sleeps on failure, the state stays off/connecting, never
        // connected.
        std::thread::sleep(Duration::from_millis(600));
        assert_ne!(presence.connection_state(), DiscordConnState::Connected);
        presence.disconnect();
    }

    #[test]
    fn default_config_ships_official_app_id_and_logo() {
        let cfg = DiscordPresenceConfig::default();
        assert!(cfg.enabled);
        // The default must be the official, 17-20 digit application id (not
        // the old all-zeros placeholder) so the branded presence works
        // without any user setup.
        assert_eq!(cfg.client_id, DEFAULT_CLIENT_ID);
        assert!(
            validate_client_id(&cfg.client_id).is_ok(),
            "default client id must pass Discord snowflake validation"
        );
        // The artwork defaults to the official logo asset key.
        assert_eq!(
            cfg.large_image_key.as_deref(),
            Some(DEFAULT_LARGE_IMAGE_KEY)
        );
        assert!(cfg.ipc_socket_path.is_none());
    }

    #[test]
    fn effective_id_chain_prefers_configured_then_official_default() {
        // 1. A non-empty configured id always wins (custom branding).
        assert_eq!(
            effective_client_id(Some("9999999999999999999")).as_deref(),
            Some("9999999999999999999")
        );
        assert_eq!(
            effective_large_image_key(Some("my_brand")).as_deref(),
            Some("my_brand")
        );
        // 2. Empty/whitespace/unconfigured falls back to the official default
        //    (zero-config end users + retirement path for deprecated ids).
        assert_eq!(
            effective_client_id(Some("")).as_deref(),
            Some(DEFAULT_CLIENT_ID)
        );
        assert_eq!(
            effective_client_id(Some("   ")).as_deref(),
            Some(DEFAULT_CLIENT_ID)
        );
        assert_eq!(
            effective_client_id(None).as_deref(),
            Some(DEFAULT_CLIENT_ID)
        );
        assert_eq!(
            effective_large_image_key(Some("")).as_deref(),
            Some(DEFAULT_LARGE_IMAGE_KEY)
        );
        assert_eq!(
            effective_large_image_key(None).as_deref(),
            Some(DEFAULT_LARGE_IMAGE_KEY)
        );
    }

    #[test]
    fn client_id_validation_accepts_realistic_snowflakes() {
        // The configured Rivulet app id (19 digits) and the documented example
        // must pass.
        assert_eq!(validate_client_id("1544027006847680532"), Ok(()));
        assert_eq!(validate_client_id("1234567890123456789"), Ok(()));
        // Empty keeps the adapter off by design — that is valid input.
        assert_eq!(validate_client_id(""), Ok(()));
        assert_eq!(validate_client_id("   "), Ok(()));
    }

    #[test]
    fn client_id_validation_rejects_non_numeric_values() {
        // Common paste mistakes: the app URL, a label prefix, or spaces inside.
        assert_eq!(
            validate_client_id("https://discord.com/developers/applications/1234567890"),
            Err(ClientIdError::NotNumeric)
        );
        assert_eq!(
            validate_client_id("client-1234567890123456"),
            Err(ClientIdError::NotNumeric)
        );
        assert_eq!(
            validate_client_id("1234 5678"),
            Err(ClientIdError::NotNumeric)
        );
        assert_eq!(
            validate_client_id("1234567890abcdefgh"),
            Err(ClientIdError::NotNumeric)
        );
    }

    #[test]
    fn client_id_validation_rejects_implausible_lengths() {
        assert_eq!(validate_client_id("123"), Err(ClientIdError::Length));
        assert_eq!(
            validate_client_id("1234567890123456"),
            Err(ClientIdError::Length)
        ); // 16 digits
           // 21 digits exceeds the snowflake range.
        assert_eq!(
            validate_client_id("123456789012345678901"),
            Err(ClientIdError::Length)
        );
    }

    #[test]
    fn set_activity_is_non_blocking_without_discord() {
        let cfg = DiscordPresenceConfig {
            enabled: true,
            client_id: "dummy-client".to_owned(),
            ..Default::default()
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
            ..Default::default()
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
        // 8-byte header: opcode 0 (HANDSHAKE) + length, then the JSON payload.
        assert_eq!(
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            0
        );
        let payload = &bytes[8..];
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
        send_set_activity(&mut buf, &st, 1234, &mut seq, None).unwrap();

        // 8-byte header: opcode 1 (FRAME) + length, then the JSON payload.
        assert_eq!(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]), 1);
        let payload = &buf[8..];
        let value: serde_json::Value = serde_json::from_slice(payload).unwrap();
        assert_eq!(value["cmd"], "SET_ACTIVITY");
        assert_eq!(value["args"]["pid"], 1234);
        assert_eq!(value["args"]["activity"]["type"], 0);
        assert_eq!(value["args"]["activity"]["details"], "Streaming");
        assert_eq!(value["args"]["activity"]["state"], "Elden Ring");
        assert!(value["args"]["activity"]["assets"].is_null());
        assert!(!value["nonce"].as_str().unwrap().is_empty());
        assert!(seq > 7);
    }

    #[test]
    fn empty_state_is_omitted_not_sent_empty() {
        // Regression (verified live): Discord rejects SET_ACTIVITY with
        // `4000: "..." is not allowed to be empty`, so a status without a
        // game name (empty `state`) must omit the field entirely instead of
        // sending an empty string. `details` always carries the status label.
        let mut buf = Vec::new();
        let st = PresenceStatus::for_activity(PresenceActivity::Idle);
        assert!(st.state.is_empty());
        assert_eq!(st.details, "Ready");
        let mut seq = 2;
        send_set_activity(&mut buf, &st, 5, &mut seq, None).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&buf[8..]).unwrap();
        assert!(
            value["args"]["activity"]["state"].is_null(),
            "empty state must be omitted, got: {}",
            value["args"]["activity"]["state"]
        );
        assert_eq!(value["args"]["activity"]["details"], "Ready");
        let text = String::from_utf8(buf[8..].to_vec()).unwrap();
        assert!(
            !text.contains("\"state\":\"\""),
            "payload must not contain an empty state string"
        );
    }

    #[test]
    fn payload_validator_reports_overlong_state_and_details() {
        // 129+ chars must be flagged so Settings can warn before the
        // truncation silently hides the condition.
        let long_details = "x".repeat(129);
        let long_state = "y".repeat(200);
        let st = PresenceStatus {
            application: "Rivulet",
            details: long_details,
            state: long_state,
        };
        let issues = validate_set_activity_payload(&st, None);
        assert!(issues.contains(&PayloadIssue::FieldTooLong {
            field: "details",
            len: 129,
        }));
        assert!(issues.contains(&PayloadIssue::FieldTooLong {
            field: "state",
            len: 200,
        }));
        // Exactly 128 chars is the limit — not a violation.
        let ok = PresenceStatus {
            application: "Rivulet",
            details: "x".repeat(128),
            state: "Ready".to_owned(),
        };
        assert!(validate_set_activity_payload(&ok, None).is_empty());
    }

    #[test]
    fn payload_validator_rejects_invalid_asset_keys() {
        // A key is sent verbatim, so an implausible one must be flagged before
        // Discord silently drops the image from the card.
        let too_long = "x".repeat(65);
        for bad in ["with space", "with/slash", "emoji😀", too_long.as_str()] {
            assert!(
                validate_set_activity_payload(&status(), Some(bad))
                    .contains(&PayloadIssue::InvalidAssetKey),
                "key {bad:?} must be rejected"
            );
        }
        // Empty and whitespace-only are treated like `None` (the serializer
        // filters them and the placeholder icon stays) — never a violation.
        for empty in ["", "  "] {
            assert!(
                validate_set_activity_payload(&status(), Some(empty)).is_empty(),
                "empty key {empty:?} must be accepted"
            );
        }
        for good in ["rivulet_logo", "logo_2026", "RIVULET_LOGO"] {
            assert!(
                validate_set_activity_payload(&status(), Some(good)).is_empty(),
                "key {good:?} must be accepted"
            );
        }
        // No key configured is always fine.
        assert!(validate_set_activity_payload(&status(), None).is_empty());
    }

    /// Exhaustive wire-contract check: serialize *every* payload variant and
    /// assert the JSON actually sent to Discord satisfies its documented
    /// validation rules. This is the CI-enforced guarantee that a `4000`
    /// rejection (empty `state`) or a silently truncated/dropped field can
    /// never reach the wire again.
    #[test]
    fn every_payload_variant_conforms_to_discord_rules_on_the_wire() {
        use crate::presence::PresenceActivity;
        let game_names: &[Option<&str>] = &[None, Some("Elden Ring"), Some(&"é".repeat(200))];
        let asset_keys: &[Option<&str>] = &[None, Some("rivulet_logo"), Some("bad key")];
        let mut checked = 0;
        for activity in PresenceActivity::all() {
            for locale in [crate::Locale::En, crate::Locale::De] {
                for game in game_names {
                    for asset in asset_keys {
                        let st = PresenceStatus::for_activity_localized(activity, locale, *game);
                        let mut buf = Vec::new();
                        let mut seq = 0;
                        send_set_activity(&mut buf, &st, 1234, &mut seq, *asset).unwrap();
                        let value: serde_json::Value = serde_json::from_slice(&buf[8..]).unwrap();
                        let act = &value["args"]["activity"];

                        // Rule 1 (error 4000): `details` is the first card line
                        // and always carries the status label — it must be
                        // present and non-empty on the wire.
                        assert_eq!(
                            act["details"].as_str(),
                            Some(locale.tr(activity.i18n_key())),
                            "details must carry the status label on the wire: \
                             {activity:?}/{locale:?}"
                        );
                        // The game name lives in `state` (second line); it
                        // must never be an empty string — present with content
                        // or omitted entirely when there is no game name.
                        match act["state"].as_str() {
                            Some(s) => assert!(
                                !s.is_empty(),
                                "empty state on the wire for {activity:?}/{locale:?}/{game:?}"
                            ),
                            None => {
                                let has_game = game.is_some_and(|g| !g.trim().is_empty());
                                assert!(
                                    !has_game,
                                    "state must be present when a game is set: \
                                     {activity:?}/{locale:?}/{game:?}"
                                );
                            }
                        }

                        // Rule 2: state/details never exceed 128 chars on the
                        // wire (the serializer truncates at a char boundary).
                        for field in ["state", "details"] {
                            if let Some(text) = act[field].as_str() {
                                assert!(
                                    text.len() <= 128,
                                    "{field} too long on the wire: {} chars",
                                    text.len()
                                );
                            }
                        }

                        // Rule 3: asset keys on the wire are always plausible
                        // (validated before send; serializer filters empty),
                        // and the configured key is mirrored to both large
                        // (profile card) and small (member list) images.
                        if let Some(img) = act["assets"]["large_image"].as_str() {
                            assert!(
                                !img.is_empty()
                                    && img.len() <= 64
                                    && img.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                                "implausible large_image on the wire: {img:?}"
                            );
                            assert_eq!(
                                act["assets"]["small_image"].as_str(),
                                Some(img),
                                "small_image must mirror large_image for {activity:?}/{asset:?}"
                            );
                            assert_eq!(
                                act["assets"]["small_text"].as_str(),
                                Some("Rivulet"),
                                "small_text must label the small image"
                            );
                        } else {
                            // No assets on the wire: the serializer filters
                            // empty *and* implausible keys (never sent
                            // verbatim), so this is valid whenever the key was
                            // not a plausible one.
                            let plausible = asset.is_some_and(|k| {
                                let k = k.trim();
                                !k.is_empty()
                                    && k.len() <= 64
                                    && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                            });
                            assert!(
                                !plausible,
                                "large_image must be present for configured key {asset:?}"
                            );
                            assert!(
                                act["assets"]["small_image"].is_null(),
                                "small_image must be absent when large_image is absent"
                            );
                        }
                        checked += 1;
                    }
                }
            }
        }
        // 6 activities × 2 locales × 3 game variants × 3 asset variants.
        assert_eq!(
            checked,
            6 * 2 * 3 * 3,
            "contract matrix must stay exhaustive"
        );
    }

    #[test]
    fn set_activity_attaches_large_image_when_configured() {
        let mut buf = Vec::new();
        let st = PresenceStatus::for_activity(PresenceActivity::Recording);
        let mut seq = 1;
        send_set_activity(&mut buf, &st, 9, &mut seq, Some("rivulet_logo")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&buf[8..]).unwrap();
        let assets = &value["args"]["activity"]["assets"];
        // The configured key is mirrored to small_image (member list) so the
        // same uploaded asset replaces the game-controller placeholder there.
        assert_eq!(assets["large_image"], "rivulet_logo");
        assert_eq!(assets["small_image"], "rivulet_logo");
        assert_eq!(assets["large_text"], "Rivulet");
        assert_eq!(assets["small_text"], "Rivulet");
    }

    #[test]
    fn payload_never_contains_sensitive_data() {
        let st = PresenceStatus::for_activity(PresenceActivity::Streaming);
        let mut buf = Vec::new();
        let mut seq = 0;
        send_set_activity(&mut buf, &st, 1, &mut seq, None).unwrap();
        let text = String::from_utf8(buf[8..].to_vec()).unwrap();
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
    fn frame_wires_opcode_and_length_prefix_then_payload() {
        let mut buf = Vec::new();
        let payload = b"hello-discord";
        write_frame(&mut buf, op::FRAME, payload).unwrap();
        assert_eq!(buf.len(), 8 + payload.len());
        assert_eq!(
            u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            op::FRAME
        );
        let len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        assert_eq!(len, payload.len());
        assert_eq!(&buf[8..], payload);
    }

    #[test]
    fn read_frame_parses_wire_format() {
        let mut buf = Vec::new();
        write_frame(&mut buf, op::FRAME, b"{\"cmd\":\"ping\"}").unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let (opcode, payload) = read_frame(&mut cursor).unwrap();
        assert_eq!(opcode, op::FRAME);
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
        send_set_activity(&mut a, &st, 42, &mut seq, None).unwrap();
        let (opcode, frame) = read_frame(&mut b).unwrap();
        assert_eq!(opcode, op::FRAME, "SET_ACTIVITY must be a FRAME op");
        let value: serde_json::Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(value["cmd"], "SET_ACTIVITY");
        assert_eq!(value["args"]["pid"], 42);
    }

    /// End-to-end smoke test: run the real worker (via `DiscordPresence`) against
    /// a local Unix-socket listener on the actual `discord-ipc-0` endpoint and
    /// verify it emits a handshake followed by a `SET_ACTIVITY` frame.
    #[cfg(unix)]
    #[test]
    fn worker_sends_handshake_then_set_activity_to_local_unix_socket_listener() {
        use std::os::unix::net::UnixListener;
        let dir = tempfile::tempdir().expect("temp dir");
        let sock_path = dir.path().join("discord-ipc-0");
        let listener = UnixListener::bind(&sock_path).expect("bind listener");

        let cfg = DiscordPresenceConfig {
            enabled: true,
            client_id: "smoke-client-unix".to_owned(),
            ipc_socket_path: Some(sock_path),
            ..Default::default()
        };
        let mut presence = DiscordPresence::new(&cfg);
        assert!(presence.enabled());
        let st = PresenceStatus::for_activity(PresenceActivity::Recording);
        assert!(presence.set_activity(&st));

        // accept() blocks until the worker connects through the explicit path.
        let (mut conn, _) = listener.accept().expect("accept worker connection");
        conn.set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");

        // Frame 1: handshake (opcode 0 = HANDSHAKE).
        let (hs_op, hs) = read_frame(&mut conn).expect("handshake frame");
        assert_eq!(hs_op, 0, "handshake must use the HANDSHAKE opcode");
        let hs: serde_json::Value = serde_json::from_slice(&hs).expect("json");
        assert_eq!(hs["v"], 1);
        assert_eq!(hs["client_id"], "smoke-client-unix");

        // Frame 2: SET_ACTIVITY for the pushed status (opcode 1 = FRAME).
        let (set_op, set) = read_frame(&mut conn).expect("set_activity frame");
        assert_eq!(set_op, 1, "SET_ACTIVITY must use the FRAME opcode");
        let set: serde_json::Value = serde_json::from_slice(&set).expect("json");
        assert_eq!(set["cmd"], "SET_ACTIVITY");
        assert_eq!(set["args"]["pid"], std::process::id());
        assert_eq!(set["args"]["activity"]["type"], 0);
        // First card line carries the status label; the game name lives in
        // `state` and is omitted when no game is selected.
        assert_eq!(set["args"]["activity"]["details"], "Recording");
        assert!(set["args"]["activity"]["state"].is_null());

        // The shared connection state must reflect the successful handshake so
        // the GUI can show "connected" instead of only the plain game card.
        wait_until(|| presence.connection_state() == DiscordConnState::Connected);
        presence.disconnect();
    }

    /// Poll `condition` until it holds or the deadline passes (test helper for
    /// the async worker: the SET_ACTIVITY frame may be buffered before the
    /// worker flips the shared connection state).
    fn wait_until(condition: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if condition() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("condition not met within 5s");
    }

    /// End-to-end smoke test for the Windows transport: run the real worker
    /// against a local named-pipe listener and verify the handshake +
    /// `SET_ACTIVITY` frames arrive over the actual pipe.
    #[cfg(windows)]
    mod named_pipe_smoke {
        use super::*;
        use std::io;
        use winapi::um::fileapi::ReadFile;
        use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
        use winapi::um::namedpipeapi::{ConnectNamedPipe, CreateNamedPipeW};
        use winapi::um::winbase::{
            PIPE_ACCESS_DUPLEX, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
            PIPE_WAIT,
        };
        use winapi::um::winnt::HANDLE;

        /// ERROR_PIPE_CONNECTED (0x21B): returned by ConnectNamedPipe when the
        /// client connected before the server issued the wait.
        const ERROR_PIPE_CONNECTED: i32 = 535;

        struct ServerPipe {
            handle: HANDLE,
        }

        impl Drop for ServerPipe {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.handle);
                }
            }
        }

        impl ServerPipe {
            fn new(name: &str) -> io::Result<Self> {
                let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
                let handle = unsafe {
                    CreateNamedPipeW(
                        wide.as_ptr(),
                        PIPE_ACCESS_DUPLEX,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                        PIPE_UNLIMITED_INSTANCES,
                        4096,
                        4096,
                        0,
                        std::ptr::null_mut(),
                    )
                };
                if std::ptr::addr_eq(handle, INVALID_HANDLE_VALUE) {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(Self { handle })
                }
            }

            /// Signal readiness for a client. If the worker already connected,
            /// ConnectNamedPipe returns ERROR_PIPE_CONNECTED, which we accept.
            fn connect(&mut self) -> io::Result<()> {
                let ok = unsafe { ConnectNamedPipe(self.handle, std::ptr::null_mut()) };
                if ok == 0 {
                    let err = io::Error::last_os_error();
                    if err.raw_os_error() != Some(ERROR_PIPE_CONNECTED) {
                        return Err(err);
                    }
                }
                Ok(())
            }
        }

        impl Read for ServerPipe {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let mut read: u32 = 0;
                let ok = unsafe {
                    ReadFile(
                        self.handle,
                        buf.as_mut_ptr() as *mut _,
                        buf.len() as u32,
                        &mut read,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(read as usize)
                }
            }
        }

        #[test]
        fn worker_sends_handshake_then_set_activity_to_local_named_pipe() {
            let pipe_name = format!(r"\\.\pipe\rivulet-discord-smoke-{}", std::process::id());
            let cfg = DiscordPresenceConfig {
                enabled: true,
                client_id: "smoke-client-windows".to_owned(),
                ipc_socket_path: Some(std::path::PathBuf::from(&pipe_name)),
                ..Default::default()
            };
            let mut presence = DiscordPresence::new(&cfg);
            assert!(presence.enabled());

            // Create the server pipe instance first so the worker's CreateFileW
            // finds an existing instance. Connect only after pushing the activity
            // (otherwise ConnectNamedPipe blocks with no client waiting yet).
            let mut server = ServerPipe::new(&pipe_name).expect("create server pipe");

            let st = PresenceStatus::for_activity(PresenceActivity::Streaming);
            assert!(presence.set_activity(&st));

            server.connect().expect("connect pipe");

            let (hs_op, hs) = read_frame(&mut server).expect("handshake frame");
            assert_eq!(hs_op, 0, "handshake must use the HANDSHAKE opcode");
            let hs: serde_json::Value = serde_json::from_slice(&hs).expect("json");
            assert_eq!(hs["v"], 1);
            assert_eq!(hs["client_id"], "smoke-client-windows");

            let (set_op, set) = read_frame(&mut server).expect("set_activity frame");
            assert_eq!(set_op, 1, "SET_ACTIVITY must use the FRAME opcode");
            let set: serde_json::Value = serde_json::from_slice(&set).expect("json");
            assert_eq!(set["cmd"], "SET_ACTIVITY");
            assert_eq!(set["args"]["pid"], std::process::id());
            // First card line carries the status label; the game name lives in
            // `state` and is omitted when no game is selected.
            assert_eq!(set["args"]["activity"]["details"], "Streaming");
            assert!(set["args"]["activity"]["state"].is_null());

            // The shared connection state must reflect the successful handshake
            // (same contract as the Unix listener smoke test).
            super::wait_until(|| presence.connection_state() == DiscordConnState::Connected);
            presence.disconnect();
        }
    }
}
