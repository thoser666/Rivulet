//! Minimal HTTP companion server (M6 remote companion, issue #99).
//!
//! Serves the mobile-friendly remote page and a tiny `/config` endpoint so a
//! phone or browser on the LAN can drive scenes, recording, and streaming
//! through the OBS WebSocket v5 server (the same authenticated surface
//! Stream Deck / TouchPortal use — no second protocol).
//!
//! The page is dependency-free (pure HTML/CSS/JS, no CDN): it loads the scene
//! list, switches scenes, and toggles recording/streaming over a WebSocket
//! connection to the obs-websocket server, including the SHA-256 challenge
//! handshake. Because LAN pages are typically served over plain `http://`
//! (not a secure context where the Web Crypto API is guaranteed), the page
//! ships a compact pure-JS SHA-256 as fallback and self-tests it against the
//! FIPS 180-4 vector on load.
//!
//! Security model (enforced together with `crate::server`):
//! - Loopback by default; LAN binding is explicit ([`CompanionConfig`]) and
//!   the obs-websocket server refuses a LAN bind without a password.
//! - Stream start/stop requires the explicit `allow_remote_stream_control`
//!   permission (enforced by the obs server's LAN permission gate, not by the
//!   page).
//! - No access logging: passwords, identifiers, and request bodies are never
//!   written to logs.

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Default HTTP port for the companion page (thematic sibling of the
/// obs-websocket 4455 default; avoids the common 8080 collision).
pub const DEFAULT_PORT: u16 = 4456;

/// Upper bound for a request head (method line + headers). Bounded so a
/// misbehaving client cannot make us buffer without limit.
const MAX_REQUEST_HEAD_BYTES: usize = 8 * 1024;

/// Idle read timeout while waiting for a request head.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// The mobile-friendly page served at `GET /`. Kept as a separate include so
/// the HTML/CSS/JS is readable and reviewable as a real document.
const PAGE_HTML: &str = include_str!("remote_companion.html");

/// Configuration for the companion HTTP server.
#[derive(Debug, Clone)]
pub struct CompanionConfig {
    /// Bind address. The GUI resolves its loopback/LAN setting to this.
    pub bind_address: IpAddr,
    /// HTTP port for the page. `0` picks an ephemeral port (useful in tests).
    pub port: u16,
    /// Port of the OBS WebSocket v5 server the page will connect to
    /// (`ws://<same-host>:<ws_port>`).
    pub ws_port: u16,
    /// Whether the obs-websocket server requires a password. Drives the
    /// page's login prompt (the challenge/response itself runs in the page).
    pub ws_auth_required: bool,
}

impl Default for CompanionConfig {
    fn default() -> Self {
        Self {
            bind_address: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            port: DEFAULT_PORT,
            ws_port: crate::server::DEFAULT_PORT,
            ws_auth_required: false,
        }
    }
}

/// A running companion server. Dropping or calling [`CompanionServerHandle::shutdown`]
/// stops the listener.
pub struct CompanionServerHandle {
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    addr: SocketAddr,
}

impl CompanionServerHandle {
    /// A handle that is not bound to any listener. Useful for tests that only
    /// exercise its type (e.g. serde round-trips of a GUI state) — nothing
    /// will be shut down on drop.
    pub fn unused() -> Self {
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            thread: None,
            addr: SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        }
    }

    /// The address the listener is bound to (useful with port 0 in tests).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Request the server to stop accepting and release the port.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for CompanionServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct CompanionState {
    shutdown: Arc<AtomicBool>,
    config: CompanionConfig,
}

/// Start the companion HTTP server.
pub fn start(config: CompanionConfig) -> io::Result<CompanionServerHandle> {
    let listener = TcpListener::bind((config.bind_address, config.port))?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let state = Arc::new(CompanionState {
        shutdown: shutdown.clone(),
        config,
    });

    tracing::info!(
        %addr,
        ws_port = state.config.ws_port,
        auth = state.config.ws_auth_required,
        "remote companion page listening"
    );

    let thread_state = state.clone();
    let thread = thread::Builder::new()
        .name("remote-companion-accept".into())
        .spawn(move || accept_loop(listener, thread_state))
        .map_err(io::Error::other)?;

    Ok(CompanionServerHandle {
        shutdown,
        thread: Some(thread),
        addr,
    })
}

fn accept_loop(listener: TcpListener, state: Arc<CompanionState>) {
    loop {
        if state.shutdown.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nodelay(true);
                let state = state.clone();
                let _ = thread::Builder::new()
                    .name("remote-companion-conn".into())
                    .spawn(move || {
                        if let Err(err) = handle_connection(stream, &state.config) {
                            tracing::debug!(error = %err, "remote companion connection ended");
                        }
                    });
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // No pending connection: sleep briefly so a shutdown request
                // can be observed (the listener is non-blocking).
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(_) => continue,
        }
    }
}

fn handle_connection(mut stream: TcpStream, config: &CompanionConfig) -> io::Result<()> {
    // The accept loop uses a non-blocking listener; on Windows the accepted
    // socket inherits that mode. Restore blocking **before** reading.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));

    let head = read_request_head(&mut stream)?;
    let Some(path) = request_path(&head) else {
        return write_response(&mut stream, "400 Bad Request", "text/plain", b"Bad request");
    };

    if path == b"/" {
        write_response(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE_HTML.as_bytes(),
        )
    } else if path == b"/config" {
        let body = format!(
            "{{\"wsPort\":{},\"authRequired\":{}}}",
            config.ws_port, config.ws_auth_required
        );
        write_response(
            &mut stream,
            "200 OK",
            "application/json; charset=utf-8",
            body.as_bytes(),
        )
    } else {
        write_response(&mut stream, "404 Not Found", "text/plain", b"Not found")
    }
}

/// Read until the CRLFCRLF that terminates a request head, bounded.
fn read_request_head(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 512];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "client closed before request head",
            ));
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_REQUEST_HEAD_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request head too large",
            ));
        }
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(buf);
        }
    }
}

/// Extract the request path from a request head. Returns `None` unless the
/// method is GET.
fn request_path(head: &[u8]) -> Option<&[u8]> {
    let head = std::str::from_utf8(head).ok()?;
    let mut lines = head.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    if method != "GET" {
        return None;
    }
    Some(path.as_bytes())
}

/// Write a minimal HTTP/1.1 response with security headers and close the
/// connection.
fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    // The page is deliberately sandboxed: scripts and styles are inline (no
    // external fetch), only same-origin fetches and WebSocket connections are
    // allowed, and no forms or base URLs are permitted.
    let csp = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
               connect-src 'self' ws: wss:; img-src 'self' data:; base-uri 'self'; form-action 'none'";
    write!(
        stream,
        "HTTP/1.1 {status}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Cache-Control: no-store\r\n\
         Content-Security-Policy: {csp}\r\n\
         \r\n",
        body.len()
    )?;
    stream.write_all(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_path_parses_get_only() {
        assert_eq!(
            request_path(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n"),
            Some(b"/".as_slice())
        );
        assert_eq!(
            request_path(b"GET /config HTTP/1.1\r\nHost: x\r\n\r\n"),
            Some(b"/config".as_slice())
        );
        assert_eq!(request_path(b"POST / HTTP/1.1\r\n\r\n"), None);
        assert_eq!(request_path(b"not a request"), None);
    }

    #[test]
    fn default_config_is_loopback_4456() {
        let config = CompanionConfig::default();
        assert!(config.bind_address.is_loopback());
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.ws_port, crate::server::DEFAULT_PORT);
        assert!(!config.ws_auth_required);
    }
}
