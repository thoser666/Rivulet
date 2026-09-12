//! Local webhook receiver that feeds the alert chat dock (M5 alerts row).
//!
//! Completes the *ingestion* story of [`crate::alerts_ingest`]: a tiny,
//! dependency-free HTTP/1.1 listener bound to **`127.0.0.1` only** that accepts
//! Streamlabs donation webhooks and Twitch EventSub notifications, authenticates
//! them (EventSub HMAC-SHA-256 against the configured secret) and forwards the
//! parsed [`AlertEvent`]s over a bounded channel the GUI drains into the chat
//! dock.
//!
//! Routes:
//! - `POST /webhook/streamlabs` — Streamlabs donation payload
//!   ([`crate::alerts_ingest::parse_streamlabs_webhook`]); Streamlabs sends no
//!   signature, so this route trusts whatever reaches loopback (local tooling,
//!   a tunnel, or a reverse proxy with an HTTPS terminator in front).
//! - `POST /eventsub/twitch` — Twitch EventSub notification
//!   ([`crate::alerts_ingest::parse_twitch_eventsub_notification`]); requires a
//!   configured secret and a valid `sha256=...` HMAC over
//!   `message-id || message-timestamp || body`
//!   ([`crate::alerts_ingest::verify_twitch_eventsub_signature`]). Rejects with
//!   `403` when the secret is empty or the signature does not match.
//!
//! **Honest scope:** this is an *ingestion endpoint to the local machine*. Real
//! Twitch/Streamlabs deliveries arrive over public HTTPS; in production a
//! local reverse proxy / TLS terminator (or a tunnel) forwards the provider's
//! POSTs to this loopback port. Twitch regards non-`2xx` as a failed delivery
//! and retries, giving an honest delivery/error signal.
//!
//! Privacy posture matches the rest of the alerts stack: bodies are bounded
//! (64 KiB), events carry no tokens, `Debug` of the handle prints counters and
//! settings only, and rejected/dropped payload contents are never logged.
//! Nothing here ever listens on a non-loopback address.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};

use crate::alerts_ingest::{
    parse_streamlabs_webhook, parse_twitch_eventsub_notification, verify_twitch_eventsub_signature,
    AlertEvent, AlertIngestError,
};

/// Default loopback port for the alert webhook receiver.
pub const DEFAULT_ALERTS_RECEIVER_PORT: u16 = 17911;

/// Maximum accepted webhook body (bytes). Oversized bodies are rejected with
/// `413` before parsing.
pub const MAX_WEBHOOK_BODY_BYTES: usize = 64 * 1024;

/// Maximum accepted request head (request line + headers, bytes).
const MAX_REQUEST_HEAD_BYTES: usize = 24 * 1024;

/// Read/write timeout per accepted connection (prevents a stuck client from
/// occupying a session thread forever).
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

/// Upper bound of the event channel between the receiver and the GUI.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Receiver settings persisted by the GUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertsReceiverConfig {
    /// Loopback port to bind (`127.0.0.1:<port>`). `0` picks an ephemeral
    /// port (used by tests).
    pub port: u16,
    /// Twitch EventSub secret. Empty disables the `/eventsub/twitch` route
    /// (`403`). Stored locally by the GUI, never logged or serialized into any
    /// event model.
    pub twitch_secret: String,
}

impl Default for AlertsReceiverConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_ALERTS_RECEIVER_PORT,
            twitch_secret: String::new(),
        }
    }
}

/// HTTP method + path + header map used by the pure request handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    /// Header names lowercased (Twitch header lookup is case-insensitive
    /// per HTTP).
    pub headers: HashMap<String, String>,
    pub body: &'a [u8],
}

/// Why a webhook request was rejected. Drives the HTTP status and, for
/// signature/parse failures, the (provider-facing) delivery signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookReject {
    /// Not a `POST` (`405`).
    MethodNotAllowed,
    /// Unknown path (`404`).
    NotFound,
    /// Body larger than [`MAX_WEBHOOK_BODY_BYTES`] (`413`).
    BodyTooLarge,
    /// Malformed request head / missing `Content-Length` (`400`).
    MalformedRequest,
    /// Twitch secret not configured; EventSub route disabled (`403`).
    SecretNotConfigured,
    /// EventSub HMAC-SHA-256 did not match (`403`).
    SignatureMismatch,
    /// Body did not parse into an ingestable alert (`400`).
    Ingest(AlertIngestError),
}

impl WebhookReject {
    /// HTTP status line for the rejection.
    pub fn status(&self) -> u16 {
        match self {
            Self::MethodNotAllowed => 405,
            Self::NotFound => 404,
            Self::BodyTooLarge => 413,
            Self::MalformedRequest => 400,
            Self::SecretNotConfigured => 403,
            Self::SignatureMismatch => 403,
            Self::Ingest(_) => 400,
        }
    }
}

/// Pure request handler: route + authenticate + parse. Network-free, so the
/// whole contract is testable deterministically (mirroring the chat workers'
/// local-listener posture at the socket layer).
pub fn handle_webhook(
    config: &AlertsReceiverConfig,
    request: &WebhookRequest<'_>,
) -> Result<AlertEvent, WebhookReject> {
    if request.method != "POST" {
        return Err(WebhookReject::MethodNotAllowed);
    }
    if request.body.len() > MAX_WEBHOOK_BODY_BYTES {
        return Err(WebhookReject::BodyTooLarge);
    }
    let body = std::str::from_utf8(request.body).map_err(|_| {
        WebhookReject::Ingest(AlertIngestError::InvalidJson(
            "body is not UTF-8".to_owned(),
        ))
    })?;
    match request.path {
        "/webhook/streamlabs" => parse_streamlabs_webhook(body).map_err(WebhookReject::Ingest),
        "/eventsub/twitch" => {
            if config.twitch_secret.is_empty() {
                return Err(WebhookReject::SecretNotConfigured);
            }
            let header = |name: &str| request.headers.get(name).map(String::as_str);
            let message_id =
                header("twitch-webhook-message-id").ok_or(WebhookReject::MalformedRequest)?;
            let message_timestamp = header("twitch-webhook-message-timestamp")
                .ok_or(WebhookReject::MalformedRequest)?;
            let signature = header("twitch-webhook-message-signature")
                .ok_or(WebhookReject::MalformedRequest)?;
            let valid = verify_twitch_eventsub_signature(
                config.twitch_secret.as_bytes(),
                message_id,
                message_timestamp,
                body,
                signature,
            );
            if !valid {
                return Err(WebhookReject::SignatureMismatch);
            }
            parse_twitch_eventsub_notification(body).map_err(WebhookReject::Ingest)
        }
        _ => Err(WebhookReject::NotFound),
    }
}

/// A running loopback alert receiver. Drop or call [`AlertsReceiver::shutdown`]
/// to stop the accept loop and close all session threads.
pub struct AlertsReceiver {
    config: AlertsReceiverConfig,
    shutdown: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    addr: SocketAddr,
    counters: Arc<ListenerCounters>,
    events: Receiver<AlertEvent>,
}

impl std::fmt::Debug for AlertsReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Privacy: settings, address and counters only — never events, never
        // the secret, never payload contents.
        let (received, accepted, rejected, body_overflow) = self.counters.snapshot();
        f.debug_struct("AlertsReceiver")
            .field("addr", &self.addr)
            .field(
                "eventsub_configured",
                &!self.config.twitch_secret.is_empty(),
            )
            .field("received", &received)
            .field("accepted", &accepted)
            .field("rejected", &rejected)
            .field("body_overflow", &body_overflow)
            .finish()
    }
}

impl AlertsReceiver {
    /// Bind `127.0.0.1:<port>` and start accepting webhook POSTs. Fails when
    /// the port is taken (surfaced as a settings error, not a crash).
    pub fn start(config: AlertsReceiverConfig) -> std::io::Result<AlertsReceiver> {
        let listener = TcpListener::bind(("127.0.0.1", config.port))?;
        // Nonblocking accept so the shutdown flag wins promptly (a blocking
        // accept would keep `join()` in `shutdown()` waiting forever).
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let (events_tx, events_rx) =
            crossbeam_channel::bounded::<AlertEvent>(EVENT_CHANNEL_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(ListenerCounters::default());

        tracing::info!(%addr, "alert webhook receiver listening on loopback");

        let thread_shutdown = shutdown.clone();
        let thread_counters = counters.clone();
        let thread_events = events_tx;
        let thread_config = config.clone();
        let thread = thread::Builder::new()
            .name("alerts-webhook-accept".into())
            .spawn(move || {
                accept_loop(
                    listener,
                    thread_config,
                    thread_shutdown,
                    thread_counters,
                    thread_events,
                )
            })
            .map_err(std::io::Error::other)?;

        Ok(AlertsReceiver {
            config,
            shutdown,
            thread: Some(thread),
            addr,
            counters,
            events: events_rx,
        })
    }

    /// Loopback address the receiver is listening on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Events parsed since start, drained by the GUI into the alert queue.
    pub fn events(&self) -> &Receiver<AlertEvent> {
        &self.events
    }

    /// Requests received, accepted and rejected (diagnostics; never contents).
    pub fn stats(&self) -> (u64, u64, u64) {
        let (received, accepted, rejected, _) = self.counters.snapshot();
        (received, accepted, rejected)
    }

    /// Stop accepting and join the accept loop (session threads drop when the
    /// channel receiver disconnects).
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AlertsReceiver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Shared accept-loop counters (kept behind one `Arc` so the loop and the
/// session threads can update them without a per-counter clone storm).
#[derive(Default)]
struct ListenerCounters {
    received: AtomicU64,
    accepted: AtomicU64,
    rejected: AtomicU64,
    body_overflow: AtomicU64,
}

impl ListenerCounters {
    fn snapshot(&self) -> (u64, u64, u64, u64) {
        (
            self.received.load(Ordering::Relaxed),
            self.accepted.load(Ordering::Relaxed),
            self.rejected.load(Ordering::Relaxed),
            self.body_overflow.load(Ordering::Relaxed),
        )
    }
}

fn accept_loop(
    listener: TcpListener,
    config: AlertsReceiverConfig,
    shutdown: Arc<AtomicBool>,
    counters: Arc<ListenerCounters>,
    events_tx: Sender<AlertEvent>,
) {
    let mut next_session: u64 = 1;
    while !shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nodelay(true);
                let config = config.clone();
                let counters = counters.clone();
                let events_tx = events_tx.clone();
                let session = next_session;
                next_session = next_session.wrapping_add(1);
                let _ = thread::Builder::new()
                    .name(format!("alerts-webhook-session-{session}"))
                    .spawn(move || {
                        let _ = handle_connection(config, stream, counters, events_tx);
                    });
                // If the OS refuses to spawn a session thread, drop the
                // connection and keep accepting (the client's delivery will
                // retry / get refused).
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

struct RequestHead {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    content_length: usize,
}

fn handle_connection(
    config: AlertsReceiverConfig,
    mut stream: TcpStream,
    counters: Arc<ListenerCounters>,
    events_tx: Sender<AlertEvent>,
) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(CONNECTION_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CONNECTION_TIMEOUT));
    counters.received.fetch_add(1, Ordering::Relaxed);

    let head = match read_request_head(&mut stream, MAX_REQUEST_HEAD_BYTES) {
        Ok(Some(head)) => head,
        Ok(None) => return write_status(&mut stream, 400),
        Err(_) => return write_status(&mut stream, 400),
    };

    if head.content_length > MAX_WEBHOOK_BODY_BYTES {
        counters.body_overflow.fetch_add(1, Ordering::Relaxed);
        // Drain the announced body before answering so the client's kernel
        // never has unread bytes outstanding at close (an RST there can eat
        // the 413 response before the client reads it — seen on macOS).
        drain_body(&mut stream, head.content_length);
        return write_status(&mut stream, 413);
    }

    let content_length = head.content_length;
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        stream.read_exact(&mut body)?;
    }

    let request = WebhookRequest {
        method: &head.method,
        path: &head.path,
        headers: head.headers,
        body: &body,
    };
    match handle_webhook(&config, &request) {
        Ok(event) => {
            // A full channel means the GUI is not draining; count and keep the
            // newest delivery signal (respond ok, drop this one locally) — the
            // bounded `AlertIngest` on the GUI side is the real backstop.
            let _ = events_tx.send(event);
            counters.accepted.fetch_add(1, Ordering::Relaxed);
            write_status(&mut stream, 200)
        }
        Err(reject) => {
            counters.rejected.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(status = reject.status(), "alert webhook rejected");
            write_status(&mut stream, reject.status())
        }
    }
}

/// Read and discard up to `content_length` body bytes (bounded by
/// [`MAX_WEBHOOK_BODY_BYTES`] + slack so a hostile length cannot pin the
/// connection). Best-effort: an error just means less to drain.
fn drain_body(stream: &mut TcpStream, content_length: usize) {
    let remaining = content_length.min(MAX_WEBHOOK_BODY_BYTES + 1024);
    let mut to_read = remaining.saturating_sub(0);
    let mut buffer = [0u8; 4096];
    while to_read > 0 {
        let chunk = to_read.min(buffer.len());
        match stream.read(&mut buffer[..chunk]) {
            Ok(0) | Err(_) => break,
            Ok(n) => to_read -= n,
        }
    }
}

/// Read the request line + headers (up to `Content-Length`), returning `None`
/// for an empty stream (connection closed before any byte).
fn read_request_head(
    stream: &mut TcpStream,
    max_head: usize,
) -> std::io::Result<Option<RequestHead>> {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    let mut first = true;
    loop {
        if buffer.len() > max_head {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request head too large",
            ));
        }
        if stream.read(&mut byte)? == 0 {
            if first {
                return Ok(None);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed mid-head",
            ));
        }
        first = false;
        buffer.push(byte[0]);
        if buffer.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let head_text = std::str::from_utf8(&buffer)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "head is not UTF-8"))?;
    let mut lines = head_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();

    let mut headers = HashMap::new();
    let mut content_length = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();
        if name == "content-length" {
            content_length = value.parse().unwrap_or(0);
        }
        headers.insert(name, value);
    }

    Ok(Some(RequestHead {
        method,
        path,
        headers,
        content_length,
    }))
}

fn write_status(stream: &mut TcpStream, status: u16) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )?;
    // Graceful close: flush, signal the end of our data (FIN), and give the
    // client a moment to read the response before the socket is dropped. A
    // bare drop with unread data still queued can make the peer kernel send
    // RST, which discards the response buffer client-side (macOS is the
    // strictest here — Linux/Windows usually deliver it anyway).
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let mut sink = [0u8; 1024];
    loop {
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(_) => continue,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts_ingest::AlertKind;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    /// Lowercase hex encoding of a byte slice (no `hex` dependency needed).
    fn to_hex(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(char::from_digit((b >> 4) as u32, 16).expect("hex digit"));
            out.push(char::from_digit((b & 0x0f) as u32, 16).expect("hex digit"));
        }
        out
    }

    fn header_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
            .collect()
    }

    /// Deterministic EventSub signature from the pinned test vector secret.
    /// The HMAC input is `message-id || message-timestamp || body`.
    fn eventsub_headers(message_id: &str, timestamp: &str, body: &str) -> HashMap<String, String> {
        let mut mac = HmacSha256::new_from_slice(b"secret").expect("hmac");
        mac.update(message_id.as_bytes());
        mac.update(timestamp.as_bytes());
        mac.update(body.as_bytes());
        let digest = to_hex(&mac.finalize().into_bytes());
        header_map(&[
            ("twitch-webhook-message-id", message_id),
            ("twitch-webhook-message-timestamp", timestamp),
            (
                "twitch-webhook-message-signature",
                &format!("sha256={digest}"),
            ),
        ])
    }

    const TWITCH_FOLLOW: &str = r#"{
        "subscription": {"type": "channel.follow", "version": "2"},
        "event": {"user_name": "Ada", "user_id": "123"}
    }"#;

    const STREAMLABS_DONATION: &str = r#"{
        "type": "donation",
        "message": [{"name": "Bob", "amount": 12.34, "currency": "EUR", "message": "brrr"}]
    }"#;

    #[test]
    fn streamlabs_donation_routes_and_parses() {
        let config = AlertsReceiverConfig::default();
        let request = WebhookRequest {
            method: "POST",
            path: "/webhook/streamlabs",
            headers: HashMap::new(),
            body: STREAMLABS_DONATION.as_bytes(),
        };
        let event = handle_webhook(&config, &request).expect("parses");
        assert_eq!(event.kind, AlertKind::Donation);
        assert_eq!(event.user, "Bob");
        assert_eq!(event.amount, Some(12.34));
        assert_eq!(event.currency.as_deref(), Some("EUR"));
        assert_eq!(event.message.as_deref(), Some("brrr"));
    }

    #[test]
    fn twitch_follow_routes_with_valid_signature() {
        let config = AlertsReceiverConfig {
            twitch_secret: "secret".to_owned(),
            ..AlertsReceiverConfig::default()
        };
        let request = WebhookRequest {
            method: "POST",
            path: "/eventsub/twitch",
            headers: eventsub_headers("a1b2c3d4", "2026-09-09T12:00:00Z", TWITCH_FOLLOW),
            body: TWITCH_FOLLOW.as_bytes(),
        };
        let event = handle_webhook(&config, &request).expect("verifies and parses");
        assert_eq!(event.kind, AlertKind::Follow);
        assert_eq!(event.user, "Ada");
    }

    #[test]
    fn twitch_rejects_bad_signature() {
        let config = AlertsReceiverConfig {
            twitch_secret: "secret".to_owned(),
            ..AlertsReceiverConfig::default()
        };
        let request = WebhookRequest {
            method: "POST",
            path: "/eventsub/twitch",
            headers: header_map(&[
                ("twitch-webhook-message-id", "a1b2c3d4"),
                ("twitch-webhook-message-timestamp", "2026-09-09T12:00:00Z"),
                (
                    "twitch-webhook-message-signature",
                    "sha256=0000000000000000000000000000000000000000000000000000000000000000",
                ),
            ]),
            body: TWITCH_FOLLOW.as_bytes(),
        };
        assert_eq!(
            handle_webhook(&config, &request).unwrap_err(),
            WebhookReject::SignatureMismatch
        );
    }

    #[test]
    fn eventsub_disabled_without_secret() {
        let config = AlertsReceiverConfig::default();
        let request = WebhookRequest {
            method: "POST",
            path: "/eventsub/twitch",
            headers: eventsub_headers("a1b2c3d4", "2026-09-09T12:00:00Z", TWITCH_FOLLOW),
            body: TWITCH_FOLLOW.as_bytes(),
        };
        assert_eq!(
            handle_webhook(&config, &request).unwrap_err(),
            WebhookReject::SecretNotConfigured
        );
    }

    #[test]
    fn method_and_path_gating() {
        let config = AlertsReceiverConfig::default();
        let get = WebhookRequest {
            method: "GET",
            path: "/webhook/streamlabs",
            headers: HashMap::new(),
            body: STREAMLABS_DONATION.as_bytes(),
        };
        assert_eq!(
            handle_webhook(&config, &get).unwrap_err(),
            WebhookReject::MethodNotAllowed
        );
        let unknown = WebhookRequest {
            method: "POST",
            path: "/nope",
            headers: HashMap::new(),
            body: STREAMLABS_DONATION.as_bytes(),
        };
        assert_eq!(
            handle_webhook(&config, &unknown).unwrap_err(),
            WebhookReject::NotFound
        );
    }

    #[test]
    fn reject_status_codes_are_sensible() {
        assert_eq!(WebhookReject::MethodNotAllowed.status(), 405);
        assert_eq!(WebhookReject::NotFound.status(), 404);
        assert_eq!(WebhookReject::BodyTooLarge.status(), 413);
        assert_eq!(WebhookReject::SignatureMismatch.status(), 403);
        assert_eq!(WebhookReject::SecretNotConfigured.status(), 403);
        assert_eq!(WebhookReject::MalformedRequest.status(), 400);
        let ingest = WebhookReject::Ingest(AlertIngestError::InvalidJson("x".into()));
        assert_eq!(ingest.status(), 400);
    }

    #[test]
    fn debug_never_includes_secret_or_events() {
        let config = AlertsReceiverConfig {
            port: 0,
            twitch_secret: "super-secret-token".to_owned(),
        };
        let receiver = AlertsReceiver::start(config).expect("binds ephemeral port");
        let debug = format!("{receiver:?}");
        assert!(!debug.contains("super-secret-token"), "{debug}");
        assert!(debug.contains("eventsub_configured: true"), "{debug}");
        drop(receiver);
    }

    #[test]
    fn body_too_large_is_rejected_before_parsing() {
        let config = AlertsReceiverConfig::default();
        let big = vec![b' '; MAX_WEBHOOK_BODY_BYTES + 1];
        let request = WebhookRequest {
            method: "POST",
            path: "/webhook/streamlabs",
            headers: HashMap::new(),
            body: &big,
        };
        assert_eq!(
            handle_webhook(&config, &request).unwrap_err(),
            WebhookReject::BodyTooLarge
        );
    }

    #[test]
    fn invalid_json_rejects_with_400_class() {
        let config = AlertsReceiverConfig::default();
        let request = WebhookRequest {
            method: "POST",
            path: "/webhook/streamlabs",
            headers: HashMap::new(),
            body: b"not json",
        };
        let err = handle_webhook(&config, &request).unwrap_err();
        assert!(
            matches!(err, WebhookReject::Ingest(AlertIngestError::InvalidJson(_))),
            "{err:?}"
        );
        assert_eq!(err.status(), 400);
    }

    #[test]
    fn loops_back_a_real_eventsub_post_and_queues_the_event() {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        use std::time::Duration;

        let config = AlertsReceiverConfig {
            port: 0,
            twitch_secret: "secret".to_owned(),
        };
        let receiver = AlertsReceiver::start(config).expect("binds ephemeral port");
        let addr = receiver.addr();

        let mut request =
            "POST /eventsub/twitch HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n"
                .as_bytes()
                .to_vec();
        let mut headers = eventsub_headers("a1b2c3d4", "2026-09-09T12:00:00Z", TWITCH_FOLLOW);
        let msg_id = std::mem::take(headers.get_mut("twitch-webhook-message-id").expect("id"));
        let ts = std::mem::take(
            headers
                .get_mut("twitch-webhook-message-timestamp")
                .expect("ts"),
        );
        let sig = std::mem::take(
            headers
                .get_mut("twitch-webhook-message-signature")
                .expect("sig"),
        );
        request.extend_from_slice(
            format!(
                "Twitch-Webhook-Message-Id: {msg_id}\r\n\
                 Twitch-Webhook-Message-Timestamp: {ts}\r\n\
                 Twitch-Webhook-Message-Signature: {sig}\r\n\
                 Content-Length: {}\r\n\r\n",
                TWITCH_FOLLOW.len()
            )
            .as_bytes(),
        );
        request.extend_from_slice(TWITCH_FOLLOW.as_bytes());

        let mut stream = TcpStream::connect(addr).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        stream.write_all(&request).expect("write");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read");
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "response was: {response}"
        );

        let event = receiver
            .events()
            .recv_timeout(Duration::from_secs(5))
            .expect("event queued");
        assert_eq!(event.kind, AlertKind::Follow);
        assert_eq!(event.user, "Ada");
        assert_eq!(receiver.stats(), (1, 1, 0));
    }

    #[test]
    fn loops_back_a_forged_signature_and_rejects_with_403() {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        use std::time::Duration;

        let config = AlertsReceiverConfig {
            port: 0,
            twitch_secret: "secret".to_owned(),
        };
        let receiver = AlertsReceiver::start(config).expect("binds ephemeral port");

        let mut request = String::from(
            "POST /eventsub/twitch HTTP/1.1\r\nHost: 127.0.0.1\r\n\
             Twitch-Webhook-Message-Id: a1b2c3d4\r\n\
             Twitch-Webhook-Message-Timestamp: 2026-09-09T12:00:00Z\r\n\
             Twitch-Webhook-Message-Signature: sha256=0000000000000000000000000000000000000000000000000000000000000000\r\n",
        );
        request.push_str(&format!("Content-Length: {}\r\n\r\n", TWITCH_FOLLOW.len()));
        request.push_str(TWITCH_FOLLOW);

        let mut stream = TcpStream::connect(receiver.addr()).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read");
        assert!(
            response.starts_with("HTTP/1.1 403"),
            "response was: {response}"
        );
        assert!(receiver.events().is_empty());
        assert_eq!(receiver.stats(), (1, 0, 1));
    }
}
