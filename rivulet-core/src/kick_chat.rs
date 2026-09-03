//! Optional Kick chat reader for the streamer-facing chat dock.
//!
//! Kick exposes chat through a Pusher WebSocket (the same infrastructure the
//! kick.com web app uses). This module mirrors the Twitch chat worker design:
//!
//! 1. **Pure parsing** — [`parse_kick_event`] turns one Pusher `chat_message`
//!    JSON payload into a [`ChatMessage`] without any I/O.
//! 2. **Non-blocking** — all socket I/O happens on a worker thread; the GUI
//!    enqueues connect/disconnect and polls a channel for messages.
//! 3. **Testable endpoints** — the WebSocket URL and the Kick API base are
//!    configurable, so CI runs the real worker against a local WebSocket
//!    server (`tungstenite::accept`) plus a local HTTP listener for the
//!    chatroom resolution, deterministically.
//! 4. **Privacy-safe** — only the configured slug and (optionally) a session
//!    token for sending are transmitted; tokens are never logged.
//!
//! Sending is gated on a session token: the worker POSTs to the Kick message
//! endpoint with the token as a Bearer credential (anonymous clients cannot
//! send on Kick).

use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;

use crossbeam_channel::{unbounded, Receiver, Sender};

use crate::twitch_chat::{ChatConnState, ChatMessage};

/// Config for connecting to Kick chat. Defaults to the public Pusher endpoint
/// and Kick API; tests override them with local listeners.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KickChatConfig {
    /// Pusher WebSocket URL. Defaults to the public `ws-us2` endpoint Kick
    /// uses with the client app key.
    pub ws_endpoint: String,
    /// Kick API base (`https://kick.com`). Used for chatroom resolution and
    /// message sending; tests point it at a local listener.
    pub api_base: String,
    /// Channel slug to resolve (e.g. `forsen`).
    pub channel: String,
    /// Session token (`x-sess-token` style) for sending. Empty = read-only.
    pub token: String,
}

impl Default for KickChatConfig {
    fn default() -> Self {
        Self {
            // The app key below is the public client key the kick.com web
            // app ships with; it changes rarely but is not part of the
            // product promise. The endpoint is overridable for tests.
            ws_endpoint: "ws://ws-us2.pusher.com/app/eb1d5f283081a6698b56?protocol=7&client=js&version=8.3.0&flash=false"
                .to_owned(),
            api_base: "https://kick.com".to_owned(),
            channel: String::new(),
            token: String::new(),
        }
    }
}

/// Parse one Pusher chat event payload into a chat message.
///
/// Kick delivers chat as `App\Events\ChatMessageEvent` events whose `data`
/// object looks like:
///
/// ```json
/// {
///   "id": "123",
///   "content": "hello",
///   "sender": {
///     "username": "viewer",
///     "identity": { "color": "#FF0000", "badges": [{"type":"broadcaster"}] }
///   },
///   "type": "message"
/// }
/// ```
///
/// Pure and deterministic — no I/O. Returns `None` for non-message events
/// (subscription acks, connection events, deleted messages, …).
pub fn parse_kick_event(payload: &str) -> Option<ChatMessage> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let event = value.get("event")?.as_str()?;
    if !event.ends_with("ChatMessageEvent") {
        return None;
    }
    let data = value.get("data")?;
    let content = data.get("content")?.as_str()?;
    let text = content.trim();
    if text.is_empty() {
        return None;
    }
    let sender = data.get("sender")?;
    let user = sender
        .get("username")
        .and_then(|u| u.as_str())
        .unwrap_or("viewer")
        .to_owned();
    let identity = sender.get("identity");
    let mut color = identity
        .and_then(|i| i.get("color"))
        .and_then(|c| c.as_str())
        .map(str::to_owned);
    if color.as_deref() == Some("#000000") || color.as_deref() == Some("") {
        color = None;
    }
    let mut badges: Vec<String> = Vec::new();
    let mut broadcaster = false;
    if let Some(badge_list) = identity
        .and_then(|i| i.get("badges"))
        .and_then(|b| b.as_array())
    {
        for badge in badge_list {
            if let Some(kind) = badge.get("type").and_then(|t| t.as_str()) {
                if kind == "broadcaster" {
                    broadcaster = true;
                }
                badges.push(kind.to_owned());
            }
        }
    }
    Some(ChatMessage {
        user,
        text: text.to_owned(),
        action: false,
        color,
        badges,
        broadcaster,
        // Kick chat has no IRC-style message id; replies are not supported.
        id: None,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    })
}

/// Resolve the numeric chatroom id for a channel slug via the Kick API
/// (`GET {api_base}/api/v2/channels/{slug}` → `chatroom.id`). Deterministic
/// against a local listener in tests.
pub fn kick_chatroom_id(api_base: &str, slug: &str) -> anyhow::Result<u64> {
    let slug = slug.trim().trim_start_matches('/');
    if slug.is_empty() {
        anyhow::bail!("empty Kick channel slug");
    }
    let url = format!("{api_base}/api/v2/channels/{slug}");
    let response = ureq::get(&url)
        .header("User-Agent", "rivulet-kick-chat")
        .call()?;
    let body = response.into_body().read_to_string()?;
    let value: serde_json::Value = serde_json::from_str(&body)?;
    let id = value
        .get("chatroom")
        .and_then(|c| c.get("id"))
        .and_then(|i| i.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Kick channel response has no chatroom id"))?;
    Ok(id)
}

/// Send a chat message on Kick via the message REST endpoint. Gated on a
/// session token (anonymous clients cannot send).
pub fn kick_send_message(
    api_base: &str,
    chatroom_id: u64,
    token: &str,
    text: &str,
) -> anyhow::Result<()> {
    let text = text.trim();
    if text.is_empty() {
        anyhow::bail!("empty message");
    }
    if token.trim().is_empty() {
        anyhow::bail!("Kick sending requires a session token");
    }
    let url = format!("{api_base}/api/v2/messages/send/{chatroom_id}");
    let body = serde_json::json!({ "content": text });
    let response = ureq::post(&url)
        .header("User-Agent", "rivulet-kick-chat")
        .header("Authorization", format!("Bearer {}", token.trim()))
        .send_json(body)?;
    response.into_body().read_to_string()?;
    Ok(())
}

/// Handle to a running Kick chat worker. Non-blocking by construction.
pub struct KickChat {
    tx: Option<Sender<Msg>>,
    messages: Option<Receiver<ChatMessage>>,
    stop: Arc<AtomicBool>,
    conn: Arc<std::sync::atomic::AtomicU8>,
}

enum Msg {
    Disconnect,
    SendMessage(String),
}

impl KickChat {
    /// Spawn the worker. Returns a disabled handle when the slug is empty.
    pub fn new(config: &KickChatConfig) -> Self {
        if config.channel.trim().is_empty() {
            return Self::disabled();
        }
        let (tx, rx) = unbounded();
        let (msg_tx, msg_rx) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let conn = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let worker_conn = Arc::clone(&conn);
        let worker_stop = Arc::clone(&stop);
        let cfg = config.clone();
        let spawned = std::thread::Builder::new()
            .name("rivulet-kick-chat".to_owned())
            .spawn(move || worker_loop(rx, cfg, worker_stop, worker_conn, msg_tx))
            .is_ok();
        if !spawned {
            return Self::disabled();
        }
        Self {
            tx: Some(tx),
            messages: Some(msg_rx),
            stop,
            conn,
        }
    }

    fn disabled() -> Self {
        Self {
            tx: None,
            messages: None,
            stop: Arc::new(AtomicBool::new(true)),
            conn: Arc::new(std::sync::atomic::AtomicU8::new(0)),
        }
    }

    /// Whether a worker is actually running.
    pub fn enabled(&self) -> bool {
        self.tx.is_some()
    }

    /// Current connection state for the GUI status line.
    pub fn connection_state(&self) -> ChatConnState {
        match self.conn.load(Ordering::SeqCst) {
            2 => ChatConnState::Disconnected,
            1 => ChatConnState::Connected,
            _ => ChatConnState::Off,
        }
    }

    /// Receiver for parsed chat messages, polled by the GUI each frame.
    pub fn messages(&self) -> Option<&Receiver<ChatMessage>> {
        self.messages.as_ref()
    }

    /// Enqueue a chat message to send. Returns `false` when the worker is
    /// disabled or the text is empty; the actual REST call happens on the
    /// worker thread.
    pub fn send_message(&self, text: &str) -> bool {
        match &self.tx {
            Some(tx) => {
                let text = text.trim();
                if text.is_empty() {
                    return false;
                }
                let _ = tx.try_send(Msg::SendMessage(text.to_owned()));
                true
            }
            None => false,
        }
    }

    /// Stop the worker. Safe to call repeatedly.
    pub fn disconnect(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Disconnect);
        }
        self.tx = None;
    }
}

impl Drop for KickChat {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Disconnect);
        }
    }
}

fn worker_loop(
    rx: Receiver<Msg>,
    cfg: KickChatConfig,
    stop: Arc<AtomicBool>,
    conn: Arc<std::sync::atomic::AtomicU8>,
    msg_tx: Sender<ChatMessage>,
) {
    let mut backoff = 1u64;
    while !stop.load(Ordering::SeqCst) {
        conn.store(1, Ordering::SeqCst);
        match run_session(&cfg, &msg_tx, &rx) {
            Ok(()) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                conn.store(2, Ordering::SeqCst);
                std::thread::sleep(Duration::from_secs(backoff));
            }
            Err(e) => {
                conn.store(2, Ordering::SeqCst);
                tracing::warn!(error = %e, backoff_secs = backoff, "Kick chat connection failed");
                std::thread::sleep(Duration::from_secs(backoff));
                backoff = (backoff * 2).min(30);
            }
        }
    }
}

/// Split a `ws://host:port/path` URL into host and port (80/443 defaults).
/// Plain TCP only — `wss://` is rejected by the caller until a TLS client
/// is wired in; Kick's public Pusher endpoint is `ws://`.
fn ws_host_port(url: &str) -> anyhow::Result<(String, u16)> {
    let rest = url
        .strip_prefix("ws://")
        .or_else(|| url.strip_prefix("wss://"))
        .ok_or_else(|| anyhow::anyhow!("unsupported Kick WebSocket URL: {url}"))?;
    let hostport = rest.split('/').next().unwrap_or(rest);
    match hostport.rsplit_once(':') {
        Some((host, port)) => {
            let port: u16 = port
                .parse()
                .map_err(|_| anyhow::anyhow!("bad port in Kick WebSocket URL: {url}"))?;
            Ok((host.to_owned(), port))
        }
        None => Ok((
            hostport.to_owned(),
            if url.starts_with("wss://") { 443 } else { 80 },
        )),
    }
}

fn run_session(
    cfg: &KickChatConfig,
    msg_tx: &Sender<ChatMessage>,
    rx: &Receiver<Msg>,
) -> anyhow::Result<()> {
    let chatroom_id = kick_chatroom_id(&cfg.api_base, &cfg.channel)?;

    // Connect manually so the read timeout applies before the handshake
    // (tungstenite::connect does not expose the underlying stream).
    if cfg.ws_endpoint.starts_with("wss://") {
        anyhow::bail!("Kick wss:// is not supported yet");
    }
    let (host, port) = ws_host_port(&cfg.ws_endpoint)?;
    let tcp = TcpStream::connect((host.as_str(), port))?;
    // Short read timeout so outbound messages are handled promptly between
    // reads; WouldBlock/TimedOut means "no input yet" and the loop drains
    // the outbound queue.
    tcp.set_read_timeout(Some(Duration::from_millis(500)))?;
    let stream = MaybeTlsStream::Plain(tcp);
    let (mut ws, _) = tungstenite::client(cfg.ws_endpoint.as_str(), stream)
        .map_err(|e| anyhow::anyhow!("Kick WebSocket handshake failed: {e}"))?;

    // Pusher handshake: subscribe to the chatroom channel.
    let subscribe = serde_json::json!({
        "event": "pusher:subscribe",
        "data": { "auth": "", "channel": format!("chatrooms.{chatroom_id}.v2") }
    });
    ws.send(tungstenite::Message::Text(subscribe.to_string().into()))?;

    tracing::info!(chatroom = chatroom_id, "Kick chat connected");

    loop {
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Disconnect => return Ok(()),
                Msg::SendMessage(text) => {
                    if let Err(e) = kick_send_message(&cfg.api_base, chatroom_id, &cfg.token, &text)
                    {
                        tracing::warn!(error = %e, "Kick send failed");
                    }
                }
            }
        }
        match ws.read() {
            Ok(tungstenite::Message::Text(text)) => {
                if let Some(message) = parse_kick_event(text.as_str()) {
                    if !message.is_empty_artifact() {
                        let _ = msg_tx.send(message);
                    }
                }
            }
            // Ping is answered automatically by tungstenite; a Close ends
            // the session (the worker reconnects with backoff).
            Ok(tungstenite::Message::Close(_)) => return Ok(()),
            Ok(tungstenite::Message::Ping(_)) => {}
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(tungstenite::Error::ConnectionClosed) => return Ok(()),
            Err(e) => return Err(anyhow::anyhow!("Kick WebSocket error: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Read one complete HTTP request (headers plus any body announced via
    /// `Content-Length`) from the socket. The fixture must fully drain the
    /// request before answering: responding (and dropping the socket) while
    /// the client is still sending unread bytes makes Windows close with
    /// `WSAECONNRESET` (os error 10054) instead of a clean FIN, which failed
    /// the YouTube worker smoke intermittently until it was fixed there; the
    /// same fixture pattern is used for the Kick chatroom-resolution HTTP
    /// endpoints (see also youtube_chat.rs::drain_request).
    fn drain_http_request(stream: &mut std::net::TcpStream) -> std::io::Result<()> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Ok(()); // client closed; nothing more to expect
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        // Drain a request body announced via Content-Length (POST send).
        let headers = String::from_utf8_lossy(&buf[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        let mut received = buf.len();
        let needed = header_end + content_length;
        while received < needed {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            received += n;
        }
        Ok(())
    }

    #[test]
    fn parses_chat_message_event() {
        // r##"…"## because the JSON contains a "# sequence ("#00FF00").
        let payload = r##"{
            "event": "App\\Events\\ChatMessageEvent",
            "data": {
                "id": "1",
                "content": "hello kick",
                "sender": {
                    "username": "ViewerOne",
                    "identity": { "color": "#00FF00", "badges": [{"type":"broadcaster"}] }
                },
                "type": "message"
            }
        }"##;
        let msg = parse_kick_event(payload).expect("message");
        assert_eq!(msg.user, "ViewerOne");
        assert_eq!(msg.text, "hello kick");
        assert_eq!(msg.color.as_deref(), Some("#00FF00"));
        assert!(msg.badges.contains(&"broadcaster".to_owned()));
        assert!(msg.broadcaster);
    }

    #[test]
    fn ignores_non_message_events() {
        assert!(
            parse_kick_event(r#"{"event":"pusher:connection_established","data":"{}"}"#).is_none()
        );
        assert!(
            parse_kick_event(r#"{"event":"pusher:subscription_succeeded","data":"{}"}"#).is_none()
        );
        assert!(parse_kick_event(
            r#"{"event":"App\\Events\\ChatMessageDeletedEvent","data":{"id":"1"}}"#
        )
        .is_none());
        assert!(parse_kick_event("not json").is_none());
        assert!(parse_kick_event(
            r#"{"event":"App\\Events\\ChatMessageEvent","data":{"content":"   "}}"#
        )
        .is_none());
    }

    #[test]
    fn resolves_chatroom_id_from_local_api() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let _ = drain_http_request(&mut stream);
            let body = r#"{"chatroom":{"id":42},"id":1}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        });
        let id = kick_chatroom_id(&format!("http://{addr}"), "testchannel").expect("id");
        assert_eq!(id, 42);
    }

    #[test]
    fn sending_requires_a_token_and_non_empty_text() {
        assert!(
            kick_send_message("http://127.0.0.1:1", 1, "", "hello").is_err(),
            "anonymous send must fail"
        );
        assert!(
            kick_send_message("http://127.0.0.1:1", 1, "tok", "   ").is_err(),
            "empty text must fail"
        );
    }

    #[test]
    fn disabled_when_slug_empty() {
        let chat = KickChat::new(&KickChatConfig {
            channel: String::new(),
            ..Default::default()
        });
        assert!(!chat.enabled());
        assert_eq!(chat.connection_state(), ChatConnState::Off);
        assert!(!chat.send_message("hello"));
    }

    /// End-to-end smoke: the real worker resolves the chatroom id against a
    /// local HTTP listener, connects to a local WebSocket server, subscribes
    /// and delivers a parsed chat message.
    #[test]
    fn worker_connects_and_delivers_messages_to_local_ws_server() {
        use tungstenite::accept;

        let api_listener = TcpListener::bind("127.0.0.1:0").expect("bind api");
        let api_addr = api_listener.local_addr().expect("api addr");
        std::thread::spawn(move || {
            let (mut stream, _) = api_listener.accept().expect("accept api");
            let _ = drain_http_request(&mut stream);
            let body = r#"{"chatroom":{"id":7}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        });

        let ws_listener = TcpListener::bind("127.0.0.1:0").expect("bind ws");
        let ws_addr = ws_listener.local_addr().expect("ws addr");
        let ws_thread = std::thread::spawn(move || {
            let (stream, _) = ws_listener.accept().expect("accept ws");
            let mut ws = accept(stream).expect("ws handshake");
            // Read the subscribe frame.
            let _ = ws.read().expect("subscribe");
            let payload = r##"{"event":"App\\Events\\ChatMessageEvent","data":{"id":"2","content":"hello from kick ws","sender":{"username":"KickFan","identity":{"color":"#FF00FF"}},"type":"message"}}"##;
            ws.send(tungstenite::Message::Text(payload.into()))
                .expect("send message");
            ws.flush().expect("flush");
            std::thread::sleep(Duration::from_millis(200));
        });

        let cfg = KickChatConfig {
            ws_endpoint: format!("ws://{ws_addr}/app/test"),
            api_base: format!("http://{api_addr}"),
            channel: "testchannel".to_owned(),
            token: String::new(),
        };
        let mut chat = KickChat::new(&cfg);
        assert!(chat.enabled());

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let delivered = loop {
            if let Some(rx) = chat.messages() {
                if let Ok(msg) = rx.try_recv() {
                    break Some(msg);
                }
            }
            if std::time::Instant::now() > deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        let msg = delivered.expect("message delivered");
        assert_eq!(msg.user, "KickFan");
        assert_eq!(msg.text, "hello from kick ws");
        assert_eq!(msg.color.as_deref(), Some("#FF00FF"));

        ws_thread.join().expect("ws server joined");
        chat.disconnect();
    }
}
