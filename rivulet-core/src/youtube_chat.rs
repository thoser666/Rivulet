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
            platform: Some(crate::chat::ChatPlatform::YouTube),
            source_room_id: None,
        });
    }
    (messages, continuation)
}

/// `purchaseAmountText.simpleText` like `"$5.00"`, `"5,00 €"`,
/// `"US$ 3.50"`, `"₹1,000.00"` → (5.0, "USD"), (5.0, "EUR"),
/// (3.5, "USD"), (1000.0, "INR"). Returns `None` when no amount can be
/// recovered — the caller then renders the raw display text.
fn parse_amount(raw: &str) -> Option<(f64, String)> {
    let currency_symbols: &[(&str, &str)] = &[
        ("$", "USD"),
        ("€", "EUR"),
        ("£", "GBP"),
        ("¥", "JPY"),
        ("₹", "INR"),
        ("₩", "KRW"),
    ];
    let prefix_codes = [
        "US$", "CA$", "AU$", "NT$", "HK$", "MX$", "AR$", "CLP$", "COP$",
    ];
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut currency: Option<String> = None;
    let mut rest = trimmed.to_owned();
    // `US$`-style display prefixes all map to USD in the locales where
    // YouTube uses them (CA$/AU$/NT$… are locale displays of USD);
    // a ISO-style `EUR 5,00` prefix is taken literally.
    for prefix in prefix_codes {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            currency = Some("USD".to_owned());
            rest = stripped.trim().to_owned();
            break;
        }
    }
    if currency.is_none() {
        if let Some(first) = trimmed.split_whitespace().next() {
            if first.len() == 3 && first.chars().all(|c| c.is_ascii_uppercase()) {
                if let Some(stripped) = trimmed.strip_prefix(first) {
                    currency = Some(first.to_owned());
                    rest = stripped.trim().to_owned();
                }
            }
        }
    }
    if currency.is_none() {
        for (symbol, code) in currency_symbols {
            if rest.contains(symbol) {
                currency = Some((*code).to_owned());
                rest = rest.replace(symbol, "");
                break;
            }
        }
    }
    // Recover the digits: drop currency letters and separators, keep
    // the last separator as the decimal point.
    let mut digits = String::new();
    let mut last_separator: Option<char> = None;
    for ch in rest.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if ch == ',' || ch == '.' {
            last_separator = Some(ch);
        }
    }
    if digits.is_empty() {
        return None;
    }
    let decimals = last_separator
        .map(|sep| {
            // Exactly three trailing digits after a separator that also has
            // digits in front of it is the thousands-group pattern
            // (`1,000`, `1.000.000`) — not a decimal point. Two or fewer
            // trailing digits (`19.99`, `5,00`) are fractional.
            let after = rest
                .rfind(sep)
                .map(|pos| {
                    rest[pos + 1..]
                        .chars()
                        .filter(|c| c.is_ascii_digit())
                        .count()
                })
                .unwrap_or(0);
            let before = rest
                .find(sep)
                .map(|pos| rest[..pos].chars().any(|c| c.is_ascii_digit()))
                .unwrap_or(false);
            if after == 3 && before {
                0
            } else {
                after
            }
        })
        .unwrap_or(0)
        .min(2) as i32;
    let value = digits.parse::<f64>().ok()? / 10f64.powi(decimals);
    Some((value, currency.unwrap_or_else(|| "".to_owned())))
}

/// Parse one `get_live_chat` response into engagement events.
///
/// Handled actions (renderer → kind):
///
/// - `liveChatPaidMessageRenderer` → [`AlertKind::Donation`] (Super Chat;
///   amount/currency parsed from `purchaseAmountText.simpleText`, message
///   joined from `message.runs`)
/// - `addLiveChatTickerItemAction.item.liveChatPaidStickerRenderer` →
///   [`AlertKind::Donation`] (Super Sticker; same amount source, label from
///   `moneyChipBackgroundColor` as the sticker tier hint)
/// - `addLiveChatTickerItemAction.item.liveChatMembershipItemRenderer` →
///   [`AlertKind::Follow`] (new member; header text like "Welcome to the
///   member's chat"). Ticker duplicates of member milestones are skipped
///   (their `headerSubtext`/header text names the milestone age, not the
///   welcome).
///
/// Everything else yields nothing. Pure and deterministic — no I/O. User
/// names are validated by the ingest queue at push time and the events
/// carry no tokens.
pub fn parse_youtube_alert_events(payload: &str) -> Vec<crate::alerts_ingest::AlertEvent> {
    use crate::alerts_ingest::{AlertEvent, AlertKind};

    fn alert(
        kind: AlertKind,
        user: String,
        tier: Option<String>,
        amount: Option<f64>,
        currency: Option<String>,
        message: Option<String>,
    ) -> AlertEvent {
        AlertEvent {
            kind,
            user,
            recipient: None,
            count: 0,
            tier,
            amount,
            currency,
            message: message
                .and_then(|m| AlertEvent::sanitize_message(&m))
                .and_then(|m| (!m.is_empty()).then_some(m)),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            platform: Some(crate::chat::ChatPlatform::YouTube),
        }
    }

    fn runs_text(value: &serde_json::Value) -> Option<String> {
        let text = value
            .get("message")
            .or_else(|| value.get("headerPrimaryText"))
            .and_then(|m| m.get("runs"))
            .and_then(|runs| runs.as_array())
            .map(|runs| {
                runs.iter()
                    .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                    .collect::<String>()
            })
            .unwrap_or_default();
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    }

    let value: serde_json::Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(actions) = value
        .pointer("/continuationContents/liveChatContinuation/actions")
        .and_then(|a| a.as_array())
    else {
        return Vec::new();
    };
    let mut events = Vec::new();
    for action in actions {
        // 1. Super Chat: a paid chat message action.
        if let Some(renderer) =
            action.pointer("/addChatItemAction/item/liveChatPaidMessageRenderer")
        {
            let Some(user) = renderer
                .pointer("/authorName/simpleText")
                .and_then(|u| u.as_str())
                .map(str::trim)
                .filter(|u| !u.is_empty())
            else {
                continue;
            };
            let raw_amount = renderer
                .pointer("/purchaseAmountText/simpleText")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let (amount, currency) = match parse_amount(raw_amount) {
                Some(pair) => pair,
                None => continue,
            };
            events.push(alert(
                AlertKind::Donation,
                user.to_owned(),
                None,
                Some(amount),
                (!currency.is_empty()).then_some(currency),
                runs_text(renderer),
            ));
            continue;
        }
        // 2. Ticker items: Super Stickers and memberships surface as
        //    ticker actions alongside the chat actions.
        if let Some(item) = action.pointer("/addLiveChatTickerItemAction/item") {
            // 2a. Super Sticker → Donation with the sticker label as tier.
            if let Some(renderer) = item.get("liveChatPaidStickerRenderer") {
                let Some(user) = renderer
                    .pointer("/authorName/simpleText")
                    .and_then(|u| u.as_str())
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
                else {
                    continue;
                };
                let raw_amount = renderer
                    .pointer("/purchaseAmountText/simpleText")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let (amount, currency) = match parse_amount(raw_amount) {
                    Some(pair) => pair,
                    None => continue,
                };
                let tier = renderer
                    .get("moneyChipBackgroundColor")
                    .and_then(|t| t.as_str())
                    .map(str::to_owned);
                events.push(alert(
                    AlertKind::Donation,
                    user.to_owned(),
                    tier,
                    Some(amount),
                    (!currency.is_empty()).then_some(currency),
                    None,
                ));
                continue;
            }
            // 2b. Membership → Follow (new member welcome). Milestone
            //     tickers carry a different header shape and are skipped.
            if let Some(renderer) = item.get("liveChatMembershipItemRenderer") {
                let Some(user) = renderer
                    .pointer("/authorName/simpleText")
                    .and_then(|u| u.as_str())
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
                else {
                    continue;
                };
                let header = runs_text(renderer).unwrap_or_default();
                let is_welcome = header.to_lowercase().contains("member")
                    && !header.to_lowercase().contains("month");
                if !is_welcome {
                    continue;
                }
                events.push(alert(
                    AlertKind::Follow,
                    user.to_owned(),
                    None,
                    None,
                    None,
                    Some(header),
                ));
            }
        }
    }
    events
}

/// Handle to a running YouTube chat worker. Non-blocking by construction.
pub struct YouTubeChat {
    tx: Option<Sender<Msg>>,
    messages: Option<Receiver<ChatMessage>>,
    alerts: Option<Receiver<crate::alerts_ingest::AlertEvent>>,
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
        let (alert_tx, alert_rx) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let conn = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let worker_conn = Arc::clone(&conn);
        let worker_stop = Arc::clone(&stop);
        let cfg = config.clone();
        let spawned = std::thread::Builder::new()
            .name("rivulet-youtube-chat".to_owned())
            .spawn(move || worker_loop(rx, cfg, worker_stop, worker_conn, msg_tx, alert_tx))
            .is_ok();
        if !spawned {
            return Self::disabled();
        }
        Self {
            tx: Some(tx),
            messages: Some(msg_rx),
            alerts: Some(alert_rx),
            stop,
            conn,
        }
    }

    fn disabled() -> Self {
        Self {
            tx: None,
            messages: None,
            alerts: None,
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

    /// Receiver for engagement events (Super Chats, Super Stickers, new
    /// members) parsed from the same poll feed as the chat messages.
    pub fn alerts(&self) -> Option<&Receiver<crate::alerts_ingest::AlertEvent>> {
        self.alerts.as_ref()
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
    alert_tx: Sender<crate::alerts_ingest::AlertEvent>,
) {
    let mut backoff = 1u64;
    while !stop.load(Ordering::SeqCst) {
        conn.store(1, Ordering::SeqCst);
        match run_session(&cfg, &msg_tx, &alert_tx, &rx) {
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
    alert_tx: &Sender<crate::alerts_ingest::AlertEvent>,
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

    // Static message: no config-derived value in log sinks (rust/
    // cleartext-logging; the video id derives from the same struct as
    // the token).
    tracing::info!("YouTube chat connected");

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
        for event in parse_youtube_alert_events(&payload) {
            let _ = alert_tx.send(event);
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

    /// Read one complete HTTP request (headers plus any body announced via
    /// `Content-Length`) from the socket. The fixture must fully drain the
    /// request before answering: responding (and dropping the socket) while
    /// the client is still sending unread bytes makes Windows close with
    /// `WSAECONNRESET` (os error 10054) instead of a clean FIN, which failed
    /// the worker intermittently (observed ~1-in-8 locally and on
    /// windows-latest).
    fn drain_request(stream: &mut std::net::TcpStream) -> std::io::Result<()> {
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
        // Drain a POST body announced via Content-Length (the poll request).
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
        assert!(
            messages
                .iter()
                .all(|m| m.platform == Some(crate::chat::ChatPlatform::YouTube)),
            "the youtube parser must tag its messages"
        );
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
            let _ = drain_request(&mut stream);
            let page = r#"<script>var ytInitialData={"continuation":"TOKEN_1"};</script>"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                page.len(),
                page
            );
            let _ = stream.write_all(response.as_bytes());

            // Connection 2: the get_live_chat poll POST.
            let (mut stream2, _) = listener.accept().expect("accept poll");
            let _ = drain_request(&mut stream2);
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

        // The worker performs two sequential HTTP round trips (page fetch +
        // first poll) before the message is delivered. The fixture drains
        // each request completely before answering (see drain_request): on
        // Windows, answering while request bytes are still unread makes the
        // socket close with WSAECONNRESET instead of a clean FIN, which
        // failed this smoke intermittently (~1-in-8 locally and on
        // windows-latest) until fixed. The generous deadline below and the
        // fail-fast on Disconnected are defense-in-depth: after a session
        // error the worker retries against the already-exhausted local
        // listener, which can never succeed, so waiting out the deadline
        // would only slow the failure down.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let delivered = loop {
            if let Some(rx) = chat.messages() {
                if let Ok(msg) = rx.try_recv() {
                    break Some(msg);
                }
            }
            if chat.connection_state() == ChatConnState::Disconnected {
                break None;
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

    #[test]
    fn parses_super_chat_into_a_donation_event() {
        let payload = r#"{
            "continuationContents": { "liveChatContinuation": { "actions": [
                { "addChatItemAction": { "item": { "liveChatPaidMessageRenderer": {
                    "authorName": { "simpleText": "BigSpender" },
                    "purchaseAmountText": { "simpleText": "$19.99" },
                    "message": { "runs": [ { "text": "love the " }, { "text": "stream" } ] }
                } } } }
            ] } }
        }"#;
        let events = parse_youtube_alert_events(payload);
        assert_eq!(events.len(), 1, "one super chat → one donation");
        let event = &events[0];
        assert_eq!(event.kind, crate::alerts_ingest::AlertKind::Donation);
        assert_eq!(event.user, "BigSpender");
        assert_eq!(event.amount, Some(19.99));
        assert_eq!(event.currency.as_deref(), Some("USD"));
        assert_eq!(event.message.as_deref(), Some("love the stream"));
        assert_eq!(
            event.platform,
            Some(crate::chat::ChatPlatform::YouTube),
            "youtube alerts must carry the platform badge"
        );
    }

    #[test]
    fn parses_super_sticker_ticker_into_a_donation_event() {
        let payload = r#"{
            "continuationContents": { "liveChatContinuation": { "actions": [
                { "addLiveChatTickerItemAction": { "item": { "liveChatPaidStickerRenderer": {
                    "authorName": { "simpleText": "StickerFan" },
                    "purchaseAmountText": { "simpleText": "5,00 €" },
                    "moneyChipBackgroundColor": "MONEY_CHIP_GREEN"
                } } } }
            ] } }
        }"#;
        let events = parse_youtube_alert_events(payload);
        assert_eq!(events.len(), 1, "one sticker → one donation");
        let event = &events[0];
        assert_eq!(event.kind, crate::alerts_ingest::AlertKind::Donation);
        assert_eq!(event.user, "StickerFan");
        assert_eq!(event.amount, Some(5.0));
        assert_eq!(event.currency.as_deref(), Some("EUR"));
        assert_eq!(event.tier.as_deref(), Some("MONEY_CHIP_GREEN"));
        assert!(event.message.is_none(), "stickers carry no message text");
    }

    #[test]
    fn parses_new_member_ticker_but_skips_milestones() {
        // New-member welcome ticker.
        let payload = r#"{
            "continuationContents": { "liveChatContinuation": { "actions": [
                { "addLiveChatTickerItemAction": { "item": { "liveChatMembershipItemRenderer": {
                    "authorName": { "simpleText": "NewMember" },
                    "headerPrimaryText": { "runs": [ { "text": "Welcome to the member's chat!" } ] }
                } } } }
            ] } }
        }"#;
        let events = parse_youtube_alert_events(payload);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, crate::alerts_ingest::AlertKind::Follow);
        assert_eq!(events[0].user, "NewMember");
        assert_eq!(
            events[0].message.as_deref(),
            Some("Welcome to the member's chat!")
        );

        // Milestone ticker ("x months") must not become a new-member event.
        let payload = r#"{
            "continuationContents": { "liveChatContinuation": { "actions": [
                { "addLiveChatTickerItemAction": { "item": { "liveChatMembershipItemRenderer": {
                    "authorName": { "simpleText": "Veteran" },
                    "headerPrimaryText": { "runs": [ { "text": "Member for 12 months" } ] }
                } } } }
            ] } }
        }"#;
        assert!(
            parse_youtube_alert_events(payload).is_empty(),
            "milestone tickers are not new members"
        );
    }

    #[test]
    fn alert_parser_skips_malformed_and_non_engagement_actions() {
        // Plain chat messages are the chat parser's job.
        assert!(
            parse_youtube_alert_events(payload_fixture()).is_empty(),
            "plain chat text must not produce alert events"
        );
        // Super Chat without an author is unusable.
        assert!(parse_youtube_alert_events(
            r#"{"continuationContents":{"liveChatContinuation":{"actions":[{"addChatItemAction":{"item":{"liveChatPaidMessageRenderer":{"purchaseAmountText":{"simpleText":"$5.00"}}}}}]}}"#,
        )
        .is_empty());
        // Unparseable amount text → no event (honest drop, no 0.00 noise).
        assert!(parse_youtube_alert_events(
            r#"{"continuationContents":{"liveChatContinuation":{"actions":[{"addChatItemAction":{"item":{"liveChatPaidMessageRenderer":{"authorName":{"simpleText":"A"},"purchaseAmountText":{"simpleText":"n/a"}}}}}]}}"#,
        )
        .is_empty());
        // Garbage payloads yield nothing.
        assert!(parse_youtube_alert_events("not json").is_empty());
        assert!(parse_youtube_alert_events("{}").is_empty());
    }

    #[test]
    fn amount_parser_handles_locale_displays() {
        use super::parse_amount as p;
        assert_eq!(p("$19.99"), Some((19.99, "USD".to_owned())));
        assert_eq!(p("5,00 €"), Some((5.0, "EUR".to_owned())));
        assert_eq!(p("US$ 3.50"), Some((3.5, "USD".to_owned())));
        assert_eq!(p("₹1,000.00"), Some((1000.0, "INR".to_owned())));
        assert_eq!(p("£10"), Some((10.0, "GBP".to_owned())));
        assert_eq!(p("JPY 500"), Some((500.0, "JPY".to_owned())));
        // Thousands separator only → whole units, no decimal split.
        assert_eq!(p("1,000"), Some((1000.0, "".to_owned())));
        assert_eq!(p(""), None);
        assert_eq!(p("n/a"), None);
    }

    /// End-to-end: the real worker polls a local HTTP fixture whose payload
    /// carries a Super Chat, and the donation lands on the alert receiver
    /// while the plain chat line lands on the message receiver.
    #[test]
    fn worker_delivers_super_chats_to_the_alert_receiver() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            // Connection 1: the initial live-chat page GET.
            let (mut stream, _) = listener.accept().expect("accept page");
            let _ = drain_request(&mut stream);
            let page = r#"<script>var ytInitialData={"continuation":"TOKEN_1"};</script>"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                page.len(),
                page
            );
            let _ = stream.write_all(response.as_bytes());

            // Connection 2: the get_live_chat poll POST with a mixed
            // payload: plain chat + Super Chat.
            let (mut stream2, _) = listener.accept().expect("accept poll");
            let _ = drain_request(&mut stream2);
            let body = r#"{"continuationContents":{"liveChatContinuation":{"actions":[{"addChatItemAction":{"item":{"liveChatTextMessageRenderer":{"authorName":{"simpleText":"Chatter"},"message":{"runs":[{"text":"plain line"}]}}}}},{"addChatItemAction":{"item":{"liveChatPaidMessageRenderer":{"authorName":{"simpleText":"WsDonor"},"purchaseAmountText":{"simpleText":"$7.50"},"message":{"runs":[{"text":"take my money"}]}}}}}],"continuations":[{"invalidationContinuationData":{"continuation":"TOKEN_2"}}]}}}"#;
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

        // The plain chat line arrives on the message receiver…
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let chat_delivered = loop {
            if let Some(rx) = chat.messages() {
                if let Ok(msg) = rx.try_recv() {
                    break Some(msg);
                }
            }
            if chat.connection_state() == ChatConnState::Disconnected {
                break None;
            }
            if std::time::Instant::now() > deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        assert_eq!(chat_delivered.expect("chat delivered").text, "plain line");

        // …and the Super Chat lands on the alert receiver.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let alert_delivered = loop {
            if let Some(rx) = chat.alerts() {
                if let Ok(event) = rx.try_recv() {
                    break Some(event);
                }
            }
            if chat.connection_state() == ChatConnState::Disconnected {
                break None;
            }
            if std::time::Instant::now() > deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        let event = alert_delivered.expect("alert delivered");
        assert_eq!(event.kind, crate::alerts_ingest::AlertKind::Donation);
        assert_eq!(event.user, "WsDonor");
        assert_eq!(event.amount, Some(7.5));

        chat.disconnect();
    }
}
