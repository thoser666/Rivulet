//! Optional YouTube live-chat reader for the streamer-facing chat dock.
//!
//! YouTube has no public IRC/WebSocket chat API; the live chat is served
//! from the Innertube JSON endpoints the website itself uses. This module
//! mirrors the Twitch/Kick chat worker design:
//!
//! 1. **Pure parsing** — [`parse_youtube_payload`] turns one `get_live_chat`
//!    response into messages plus the next continuation token, without I/O.
//! 2. **Non-blocking** — all HTTP I/O happens on a worker thread.
//! 3. **Testable endpoints** — the initial page and the poll endpoint are
//!    configurable, so CI runs the real worker against a local HTTP
//!    listener deterministically.
//! 4. **Read-only** — anonymous YouTube chat cannot send messages (that
//!    requires an authenticated browser session), so the worker never sends.
//! 5. **Honest about brittleness** — Innertube endpoints are not a stable
//!    public API; when Google changes them the worker reports a connection
//!    error instead of pretending chat works.
//!
//! Flow: fetch the live-chat page (`/live_chat?is_popout=1&v=<id>`), extract
//! the continuation token, then poll `youtubei/v1/live_chat/get_live_chat`
//! with that token, each response yielding messages and the next token.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};

use crate::twitch_chat::{ChatConnState, ChatMessage};

/// Config for connecting to YouTube live chat. Endpoints are configurable so
/// tests can point them at a local listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YouTubeChatConfig {
    /// Initial live-chat page URL (contains `ytInitialData` with the first
    /// continuation token).
    pub page_endpoint: String,
    /// Innertube `get_live_chat` poll endpoint.
    pub poll_endpoint: String,
    /// Video id (`v=`) of the live stream.
    pub channel: String,
}

impl Default for YouTubeChatConfig {
    fn default() -> Self {
        Self {
            page_endpoint: "https://www.youtube.com/live_chat?is_popout=1&v=".to_owned(),
            poll_endpoint: "https://www.youtube.com/youtubei/v1/live_chat/get_live_chat".to_owned(),
            channel: String::new(),
        }
    }
}

/// Extract the first continuation token from a live-chat page HTML.
///
/// The popout page embeds `ytInitialData` JSON with
/// `"continuation":"..."` (URL-escaped). This scans for the first quoted
/// continuation string and unescapes `\u0026` → `&`. Pure and
/// deterministic; returns `None` when the page carries no continuation
/// (e.g. the stream ended or the video id is invalid).
pub fn youtube_initial_continuation(html: &str) -> Option<String> {
    let marker = "\"continuation\":\"";
    let start = html.find(marker)? + marker.len();
    let end = html[start..].find('"')? + start;
    let raw = &html[start..end];
    if raw.is_empty() {
        return None;
    }
    Some(raw.replace("\\u0026", "&").replace("\\/", "/"))
}

/// Parse one `get_live_chat` response into chat messages plus the next
/// continuation token.
///
/// Handles `continuationContents.liveChatContinuation.actions[]` where each
/// `addChatItemAction.item.liveChatTextMessageRenderer` carries the author
/// name/color/badges and the message runs. The next token is taken from the
/// first `continuations[].*ContinuationData.continuation`. Pure.
pub fn parse_youtube_payload(payload: &str) -> (Vec<ChatMessage>, Option<String>) {
    let value: serde_json::Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(_) => return (Vec::new(), None),
    };
    let continuation = value
        .pointer("/continuationContents/liveChatContinuation/continuations/0")
        .and_then(|c| {
            c.get("invalidationContinuationData")
                .or_else(|| c.get("timedContinuationData"))
                .or_else(|| c.get("liveChatReplayContinuationData"))
        })
        .and_then(|d| d.get("continuation"))
        .and_then(|c| c.as_str())
        .map(str::to_owned);

    let mut messages = Vec::new();
    let Some(actions) = value
        .pointer("/continuationContents/liveChatContinuation/actions")
        .and_then(|a| a.as_array())
    else {
        return (messages, continuation);
    };
    for action in actions {
        let Some(renderer) = action.pointer("/addChatItemAction/item/liveChatTextMessageRenderer")
        else {
            continue;
        };
        let user = renderer
            .get("authorName")
            .and_then(|n| n.get("simpleText"))
            .and_then(|t| t.as_str())
            .unwrap_or("viewer")
            .to_owned();
        let text = renderer
            .get("message")
            .and_then(|m| m.get("runs"))
            .and_then(|runs| runs.as_array())
            .map(|runs| {
                runs.iter()
                    .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                    .collect::<String>()
            })
            .unwrap_or_default();
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let mut color = renderer
            .get("authorNameTextColor")
            .and_then(|c| c.as_str())
            .map(str::to_owned);
        if color.as_deref() == Some("#000000") {
            color = None;
        }
        let mut badges: Vec<String> = Vec::new();
        let mut broadcaster = false;
        if let Some(list) = renderer.get("authorBadges").and_then(|b| b.as_array()) {
            for badge in list {
                if let Some(kind) = badge
                    .pointer("/liveChatAuthorBadgeRenderer/accessibility/accessibilityData/label")
                    .and_then(|l| l.as_str())
                {
                    let kind = kind.to_owned();
                    if kind.to_lowercase().contains("owner") {
                        broadcaster = true;
                    }
                    badges.push(kind);
                }
            }
        }
        messages.push(ChatMessage {
            user,
            text: text.to_owned(),
            action: false,
            color,
            badges,
            broadcaster,
            // YouTube chat is read-only; no IRC-style message id.
            id: None,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        });
    }
    (messages, continuation)
}

/// Handle to a running YouTube chat worker. Non-blocking by construction.
pub struct YouTubeChat {
    tx: Option<Sender<Msg>>,
    messages: Option<Receiver<ChatMessage>>,
    stop: Arc<AtomicBool>,
    conn: Arc<std::sync::atomic::AtomicU8>,
}

enum Msg {
    Disconnect,
}

impl YouTubeChat {
    /// Spawn the worker. Returns a disabled handle when the video id is empty.
    pub fn new(config: &YouTubeChatConfig) -> Self {
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
            .name("rivulet-youtube-chat".to_owned())
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

    /// YouTube chat is read-only without an authenticated browser session;
    /// sends are always rejected.
    pub fn send_message(&self, _text: &str) -> bool {
        false
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

impl Drop for YouTubeChat {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Disconnect);
        }
    }
}

fn worker_loop(
    rx: Receiver<Msg>,
    cfg: YouTubeChatConfig,
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
                tracing::warn!(error = %e, backoff_secs = backoff, "YouTube chat connection failed");
                std::thread::sleep(Duration::from_secs(backoff));
                backoff = (backoff * 2).min(30);
            }
        }
    }
}

fn run_session(
    cfg: &YouTubeChatConfig,
    msg_tx: &Sender<ChatMessage>,
    rx: &Receiver<Msg>,
) -> anyhow::Result<()> {
    let video_id = cfg.channel.trim();
    // 1. Fetch the live-chat page and extract the first continuation token.
    let page_url = format!("{}{}", cfg.page_endpoint, video_id);
    let page = ureq::get(&page_url)
        .header("User-Agent", "rivulet-youtube-chat")
        .call()?
        .into_body()
        .read_to_string()?;
    let mut continuation = youtube_initial_continuation(&page).ok_or_else(|| {
        anyhow::anyhow!("no live-chat continuation found (stream may have ended)")
    })?;

    tracing::info!(video = video_id, "YouTube chat connected");

    loop {
        if let Ok(Msg::Disconnect) = rx.try_recv() {
            return Ok(());
        }
        // 2. Poll for the next batch of messages.
        let body = serde_json::json!({
            "context": { "client": { "clientName": "WEB", "clientVersion": "2.0" } },
            "continuation": continuation,
        });
        let response = ureq::post(&cfg.poll_endpoint)
            .header("User-Agent", "rivulet-youtube-chat")
            .send_json(body)?;
        let payload = response.into_body().read_to_string()?;
        let (messages, next) = parse_youtube_payload(&payload);
        for message in messages {
            if !message.is_empty_artifact() {
                let _ = msg_tx.send(message);
            }
        }
        continuation = match next {
            Some(token) => token,
            None => {
                // No continuation: the stream ended or the endpoint changed.
                return Ok(());
            }
        };
        std::thread::sleep(Duration::from_secs(3));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn payload_fixture() -> &'static str {
        // r##"…"## because the JSON contains a "# sequence ("#FF9900").
        r##"{
            "continuationContents": {
                "liveChatContinuation": {
                    "actions": [
                        {
                            "addChatItemAction": {
                                "item": {
                                    "liveChatTextMessageRenderer": {
                                        "authorName": { "simpleText": "YTViewer" },
                                        "authorNameTextColor": "#FF9900",
                                        "authorBadges": [
                                            { "liveChatAuthorBadgeRenderer": {
                                                "accessibility": { "accessibilityData": { "label": "Owner" } }
                                            } }
                                        ],
                                        "message": { "runs": [ { "text": "hello " }, { "text": "youtube" } ] }
                                    }
                                }
                            }
                        },
                        {
                            "addChatItemAction": {
                                "item": {
                                    "liveChatTextMessageRenderer": {
                                        "authorName": { "simpleText": "Plain" },
                                        "message": { "runs": [ { "text": "second message" } ] }
                                    }
                                }
                            }
                        }
                    ],
                    "continuations": [
                        { "invalidationContinuationData": { "continuation": "TOKEN_2" } }
                    ]
                }
            }
        }"##
    }

    #[test]
    fn parses_messages_and_next_continuation() {
        let (messages, next) = parse_youtube_payload(payload_fixture());
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].user, "YTViewer");
        assert_eq!(messages[0].text, "hello youtube");
        assert_eq!(messages[0].color.as_deref(), Some("#FF9900"));
        assert!(
            messages[0].broadcaster,
            "Owner badge must mark the broadcaster"
        );
        assert_eq!(messages[1].user, "Plain");
        assert_eq!(messages[1].text, "second message");
        assert!(!messages[1].broadcaster);
        assert_eq!(next.as_deref(), Some("TOKEN_2"));
    }

    #[test]
    fn handles_end_of_stream_and_garbage() {
        let (messages, next) = parse_youtube_payload(
            r#"{"continuationContents":{"liveChatContinuation":{"actions":[]}}}"#,
        );
        assert!(messages.is_empty());
        assert!(next.is_none());
        let (messages, next) = parse_youtube_payload("not json");
        assert!(messages.is_empty());
        assert!(next.is_none());
    }

    #[test]
    fn extracts_continuation_from_page_html() {
        let html = r#"<script>var ytInitialData = {"contents":{"liveChatRenderer":{"continuations":[{"invalidationContinuationData":{"continuation":"abc\u0026def\/ghi"}}]}}};</script>"#;
        assert_eq!(
            youtube_initial_continuation(html).as_deref(),
            Some("abc&def/ghi")
        );
        assert!(youtube_initial_continuation("<html>no chat here</html>").is_none());
    }

    #[test]
    fn disabled_when_video_id_empty_and_read_only() {
        let chat = YouTubeChat::new(&YouTubeChatConfig {
            channel: String::new(),
            ..Default::default()
        });
        assert!(!chat.enabled());
        assert_eq!(chat.connection_state(), ChatConnState::Off);
        assert!(
            !chat.send_message("hello"),
            "YouTube chat is read-only without an authenticated session"
        );
    }

    /// End-to-end smoke: the real worker fetches the page (continuation
    /// extraction) and polls the get_live_chat endpoint from a local HTTP
    /// listener, delivering the parsed message.
    #[test]
    fn worker_connects_and_delivers_messages_to_local_http_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            // Connection 1: the initial live-chat page GET.
            let (mut stream, _) = listener.accept().expect("accept page");
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let page = r#"<script>var ytInitialData={"continuation":"TOKEN_1"};</script>"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                page.len(),
                page
            );
            let _ = stream.write_all(response.as_bytes());

            // Connection 2: the get_live_chat poll POST.
            let (mut stream2, _) = listener.accept().expect("accept poll");
            let mut buf2 = [0u8; 8192];
            let _ = stream2.read(&mut buf2);
            let body = payload_fixture();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream2.write_all(response.as_bytes());
        });

        let cfg = YouTubeChatConfig {
            page_endpoint: format!("http://{addr}/live_chat?is_popout=1&v="),
            poll_endpoint: format!("http://{addr}/get_live_chat"),
            channel: "abc123".to_owned(),
        };
        let mut chat = YouTubeChat::new(&cfg);
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
        assert_eq!(msg.user, "YTViewer");
        assert_eq!(msg.text, "hello youtube");
        chat.disconnect();
    }
}
