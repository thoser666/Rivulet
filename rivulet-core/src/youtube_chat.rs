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
//! 4. **Observer mode without credentials** — the read path is the anonymous
//!    Innertube poller and always works. Sending goes through the *official*
//!    Live Streaming API (`liveChatMessages.insert`), which needs an API key
//!    plus an OAuth token with the `youtube.force-ssl` scope and the
//!    broadcast's `liveChatId`. Without all three the dock reports itself as
//!    read-only instead of pretending it can send.
//! 5. **Quota-accounted** — `insert` costs ~200 units against a 10 000-unit
//!    daily project budget (~50 messages/day). [`YouTubeQuota`] enforces that
//!    locally so the bot stops before Google hard-fails the project, and a
//!    `quotaExceeded`/`dailyLimitExceeded` response pins the account to
//!    read-only for the rest of the day.
//! 6. **Honest about brittleness** — Innertube endpoints are not a stable
//!    public API; when Google changes them the worker reports a connection
//!    error instead of pretending chat works.
//!
//! Flow: fetch the live-chat page (`/live_chat?is_popout=1&v=<id>`), extract
//! the continuation token, then poll `youtubei/v1/live_chat/get_live_chat`
//! with that token, each response yielding messages and the next token.
//! Outbound text is queued on the same worker thread and sent through
//! [`youtube_send_message`], replies carrying the `snippet.parentId` that
//! makes them thread onto a specific chat line.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};

use crate::chat::ChatPlatform;
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
    /// Official `liveChatMessages.insert` endpoint (Live Streaming API).
    pub send_endpoint: String,
    /// Video id (`v=`) of the live stream.
    pub channel: String,
    /// `liveChatId` of the running broadcast. The insert call needs it; the
    /// read path does not. Empty → read-only.
    pub live_chat_id: String,
    /// API key + OAuth token for the official send path. `None` or an
    /// incomplete pair → read-only.
    pub send_credentials: Option<YouTubeSendCredentials>,
    /// Daily quota budget. `None` → the documented 200/10 000 default.
    pub quota: Option<YouTubeQuotaConfig>,
}

impl Default for YouTubeChatConfig {
    fn default() -> Self {
        Self {
            page_endpoint: "https://www.youtube.com/live_chat?is_popout=1&v=".to_owned(),
            poll_endpoint: "https://www.youtube.com/youtubei/v1/live_chat/get_live_chat".to_owned(),
            send_endpoint: "https://www.googleapis.com/youtube/v3/liveChat/messages".to_owned(),
            channel: String::new(),
            live_chat_id: String::new(),
            send_credentials: None,
            quota: None,
        }
    }
}

/// Official-API credentials for `liveChatMessages.insert`.
///
/// Both halves are secrets, so [`std::fmt::Debug`] reports presence and
/// length only — a `{:?}` in a log line or panic message can never leak
/// them.
#[derive(Clone, PartialEq, Eq)]
pub struct YouTubeSendCredentials {
    /// Data API key, sent as the `key=` query parameter.
    pub api_key: String,
    /// OAuth token with the `youtube.force-ssl` scope (`youtube` is the
    /// legacy alias and is accepted by the same call).
    pub oauth_token: String,
}

impl YouTubeSendCredentials {
    pub fn new(api_key: String, oauth_token: String) -> Self {
        Self {
            api_key,
            oauth_token,
        }
    }

    /// Whether both halves of the official send contract are present.
    /// Without either one the account can only observe.
    pub fn is_complete(&self) -> bool {
        !self.api_key.trim().is_empty() && !self.oauth_token.trim().is_empty()
    }
}

impl std::fmt::Debug for YouTubeSendCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YouTubeSendCredentials")
            .field("api_key", &masked_secret(self.api_key.trim()))
            .field("oauth_token", &masked_secret(self.oauth_token.trim()))
            .finish()
    }
}

/// `<missing>` or `<set: N chars>` — presence and length only, never a
/// character of the secret itself.
fn masked_secret(value: &str) -> String {
    if value.is_empty() {
        "<missing>".to_owned()
    } else {
        format!("<set: {} chars>", value.chars().count())
    }
}

/// One credential the bot needs on a platform, reported by presence only.
/// This is the row type behind the masked auth/scope matrix: it can say
/// *that* a slot is filled, never *what* is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatCredentialSlot {
    /// Stable identifier for the slot (`api_key`, `oauth`, `session`).
    pub slot: &'static str,
    /// What the platform calls it, shown as the UI hint.
    pub label: &'static str,
    /// Whether the slot is filled for the configured account.
    pub present: bool,
    /// Why the platform needs this credential at all.
    pub purpose: &'static str,
}

/// The masked auth/scope matrix for one platform: every credential the
/// platform needs before the bot may send, with presence flags only. Values
/// are never part of the return type, so this cannot be logged, exported or
/// screenshotted by accident.
///
/// `api_key` / `oauth` describe what the caller has configured; both default
/// to `false`, which is the honest state for a chat account that only stores
/// a single token.
pub fn chat_auth_matrix(
    platform: ChatPlatform,
    api_key: bool,
    token: bool,
) -> Vec<ChatCredentialSlot> {
    match platform {
        ChatPlatform::Twitch => vec![
            ChatCredentialSlot {
                slot: "oauth",
                label: "OAuth token (chat:read + chat:edit)",
                present: token,
                purpose: "read and send chat",
            },
            ChatCredentialSlot {
                slot: "irc_tags",
                label: "twitch.tv/tags capability",
                present: token,
                purpose: "reply-parent-msg-id threading",
            },
            ChatCredentialSlot {
                slot: "phone_verified",
                label: "phone-verified bot account",
                present: false,
                purpose: "server-side send requirement",
            },
        ],
        ChatPlatform::Kick => vec![ChatCredentialSlot {
            slot: "session",
            label: "session token",
            present: token,
            purpose: "send via the undocumented API",
        }],
        ChatPlatform::YouTube => vec![
            ChatCredentialSlot {
                slot: "api_key",
                label: "Data API key",
                present: api_key,
                purpose: "identify the API project",
            },
            ChatCredentialSlot {
                slot: "oauth",
                label: "OAuth token (youtube.force-ssl)",
                present: token,
                purpose: "authorize liveChatMessages.insert",
            },
            ChatCredentialSlot {
                slot: "live_chat_id",
                label: "liveChatId of the broadcast",
                present: false,
                purpose: "target the insert call",
            },
        ],
    }
}

/// Quota budget of the official YouTube Live Streaming API, in the project's
/// quota units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YouTubeQuotaConfig {
    /// Units one `liveChatMessages.insert` call costs (~200).
    pub insert_units: u32,
    /// Daily project budget (10 000).
    pub daily_units: u32,
}

impl YouTubeQuotaConfig {
    /// Documented cost of one insert.
    pub const INSERT_UNITS: u32 = 200;
    /// Documented daily project budget.
    pub const DAILY_UNITS: u32 = 10_000;

    /// The documented default: ~50 sends per day.
    pub const fn documented() -> Self {
        Self {
            insert_units: Self::INSERT_UNITS,
            daily_units: Self::DAILY_UNITS,
        }
    }

    /// Whole inserts that fit into the daily budget.
    pub fn sends_per_day(&self) -> u32 {
        self.daily_units / self.insert_units.max(1)
    }
}

impl Default for YouTubeQuotaConfig {
    fn default() -> Self {
        Self::documented()
    }
}

/// Tracks the daily API quota of one YouTube project so the bot never
/// overdraws it: each accepted insert costs [`YouTubeQuotaConfig::INSERT_UNITS`]
/// and the budget resets at the UTC day boundary. The clock is injectable so
/// tests cross a day boundary deterministically instead of sleeping.
pub struct YouTubeQuota {
    config: YouTubeQuotaConfig,
    spent_units: u32,
    day: u64,
    now: Box<dyn Fn() -> u64 + Send + Sync>,
}

const SECS_PER_DAY: u64 = 86_400;

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl YouTubeQuota {
    /// Tracker with the documented budget and the system clock.
    pub fn new(config: YouTubeQuotaConfig) -> Self {
        Self::with_clock(config, unix_secs)
    }

    /// Tracker with an explicit unix-seconds clock (tests).
    pub fn with_clock(
        config: YouTubeQuotaConfig,
        now: impl Fn() -> u64 + Send + Sync + 'static,
    ) -> Self {
        let secs = now();
        Self {
            config,
            spent_units: 0,
            day: secs / SECS_PER_DAY,
            now: Box::new(now),
        }
    }

    /// The budget this tracker enforces.
    pub fn config(&self) -> YouTubeQuotaConfig {
        self.config
    }

    /// Units still available today (0 once the budget is spent).
    pub fn remaining_units(&mut self) -> u32 {
        self.roll_day();
        self.config.daily_units.saturating_sub(self.spent_units)
    }

    /// Whole sends still possible today.
    pub fn remaining_sends(&mut self) -> u32 {
        self.remaining_units() / self.config.insert_units.max(1)
    }

    /// Charge one insert against today's budget. `false` once the budget is
    /// exhausted — the caller must degrade to read-only rather than send.
    pub fn try_acquire(&mut self) -> bool {
        self.roll_day();
        let cost = self.config.insert_units;
        if self.spent_units.saturating_add(cost) > self.config.daily_units {
            return false;
        }
        self.spent_units += cost;
        true
    }

    /// Mark today's budget as fully spent. Used when the API itself reports
    /// `quotaExceeded`/`dailyLimitExceeded`: the local estimate may still have
    /// units left (other clients share the project), but we must not send.
    pub fn exhaust(&mut self) {
        self.roll_day();
        self.spent_units = self.config.daily_units;
    }

    /// Units charged so far today.
    pub fn spent_units(&mut self) -> u32 {
        self.roll_day();
        self.spent_units
    }

    /// Reset the counter when the clock crossed into a new UTC day.
    fn roll_day(&mut self) {
        let day = (self.now)() / SECS_PER_DAY;
        if day != self.day {
            self.day = day;
            self.spent_units = 0;
        }
    }
}

impl std::fmt::Debug for YouTubeQuota {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YouTubeQuota")
            .field("config", &self.config)
            .field("spent_units", &self.spent_units)
            .field("day", &self.day)
            .finish_non_exhaustive()
    }
}

/// Result of one `liveChatMessages.insert` attempt, classified so the caller
/// can decide between "retry later" and "stop sending today".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YouTubeSendOutcome {
    /// The API accepted the message.
    Sent,
    /// The project quota or daily limit is exhausted — sending stops until
    /// the next UTC day.
    QuotaExhausted,
    /// The API refused the call for another reason (missing scope, wrong
    /// `liveChatId`, network error). The account stays in observer mode.
    Rejected,
}

impl YouTubeSendOutcome {
    pub fn is_sent(self) -> bool {
        matches!(self, Self::Sent)
    }
}

/// Percent-encode one `application/x-www-form-urlencoded` component.
///
/// Only the unreserved set from RFC 3986 stays literal; everything else —
/// including the space chat messages are full of — is escaped as
/// `%XX` over the UTF-8 bytes.
pub fn form_encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

/// Build the `application/x-www-form-urlencoded` body of an insert call.
///
/// Pure, so the exact wire format (including `snippet.parentId` for replies)
/// is unit-tested without a socket.
pub fn youtube_insert_body(live_chat_id: &str, text: &str, parent_id: Option<&str>) -> String {
    let mut fields: Vec<(String, String)> = vec![
        ("part".to_owned(), "snippet".to_owned()),
        ("snippet.liveChatId".to_owned(), live_chat_id.to_owned()),
        ("snippet.type".to_owned(), "textMessageEvent".to_owned()),
        ("snippet.textOriginal".to_owned(), text.to_owned()),
    ];
    if let Some(parent) = parent_id.map(str::trim).filter(|p| !p.is_empty()) {
        fields.push(("snippet.parentId".to_owned(), parent.to_owned()));
    }
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode_component(k), form_encode_component(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Whether an error body from the API reports an exhausted quota
/// (`quotaExceeded`, `dailyLimitExceeded`, `rateLimitExceeded`) rather than a
/// fixable request problem.
pub fn youtube_quota_exhausted(body: &str) -> bool {
    ["quotaExceeded", "dailyLimitExceeded", "rateLimitExceeded"]
        .iter()
        .any(|reason| body.contains(reason))
}

/// Send one chat message through the official Live Streaming API.
///
/// POSTs [`youtube_insert_body`] to `liveChatMessages.insert` with the API
/// key as `key=` and the OAuth token as `Bearer`. `parent_id` turns the
/// message into a reply to that chat line. Credentials and message text never
/// reach an error string — failures are returned as a classified
/// [`YouTubeSendOutcome`], so the caller can degrade without logging secrets.
pub fn youtube_send_message(
    send_endpoint: &str,
    credentials: &YouTubeSendCredentials,
    live_chat_id: &str,
    text: &str,
    parent_id: Option<&str>,
) -> YouTubeSendOutcome {
    let text = text.trim();
    let live_chat_id = live_chat_id.trim();
    if text.is_empty() || live_chat_id.is_empty() || !credentials.is_complete() {
        return YouTubeSendOutcome::Rejected;
    }
    let url = format!(
        "{send_endpoint}?part=snippet&key={}",
        form_encode_component(credentials.api_key.trim())
    );
    let body = youtube_insert_body(live_chat_id, text, parent_id);
    let bearer = format!("Bearer {}", credentials.oauth_token.trim());

    // `http_status_as_error(false)`: a quota refusal arrives as a 403 whose
    // *body* names the reason. The default agent turns 4xx into an error that
    // discards the body, which would make an exhausted budget
    // indistinguishable from a bad token.
    let config = ureq::config::Config::builder()
        .http_status_as_error(false)
        .build();
    let attempt = ureq::Agent::new_with_config(config)
        .post(&url)
        .header("Authorization", bearer)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(body.as_bytes());
    let response = match attempt {
        Ok(response) => response,
        Err(_) => return YouTubeSendOutcome::Rejected,
    };
    let status = response.status().as_u16();
    let response_body = response.into_body().read_to_string().unwrap_or_default();

    if (200..300).contains(&status) {
        return YouTubeSendOutcome::Sent;
    }
    if youtube_quota_exhausted(&response_body) {
        YouTubeSendOutcome::QuotaExhausted
    } else {
        YouTubeSendOutcome::Rejected
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
    /// Daily quota of the API project, shared with the worker so a send that
    /// the API rejects for quota is remembered (and shown in the dock).
    quota: Arc<Mutex<YouTubeQuota>>,
    /// Whether the official send contract is fully configured (API key,
    /// OAuth token, `liveChatId`). Without it the account is an observer.
    sendable: bool,
}

enum Msg {
    Disconnect,
    SendMessage(String),
    /// Reply to a specific chat line via the snippet's `parentId`.
    SendReply {
        text: String,
        parent: String,
    },
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
        let quota = Arc::new(Mutex::new(YouTubeQuota::new(
            config.quota.unwrap_or_default(),
        )));
        let sendable = !config.send_endpoint.trim().is_empty()
            && !config.live_chat_id.trim().is_empty()
            && config
                .send_credentials
                .as_ref()
                .is_some_and(YouTubeSendCredentials::is_complete);
        let worker_conn = Arc::clone(&conn);
        let worker_stop = Arc::clone(&stop);
        let worker_quota = Arc::clone(&quota);
        let cfg = config.clone();
        let spawned = std::thread::Builder::new()
            .name("rivulet-youtube-chat".to_owned())
            .spawn(move || {
                worker_loop(
                    rx,
                    cfg,
                    worker_stop,
                    worker_conn,
                    msg_tx,
                    alert_tx,
                    worker_quota,
                )
            })
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
            quota,
            sendable,
        }
    }

    fn disabled() -> Self {
        Self {
            tx: None,
            messages: None,
            alerts: None,
            stop: Arc::new(AtomicBool::new(true)),
            conn: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            quota: Arc::new(Mutex::new(YouTubeQuota::new(YouTubeQuotaConfig::default()))),
            sendable: false,
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

    /// Whether this account can send at all: the official insert contract
    /// (API key, OAuth token, `liveChatId`) must be fully configured and
    /// there must be quota left today. The dock uses this to keep a YouTube
    /// account read-only instead of offering an input that cannot work.
    pub fn can_send(&self) -> bool {
        self.sendable && self.quota_remaining_sends() > 0
    }

    /// Whether the account is configured to send but has spent its daily
    /// quota — the dock shows this as "read-only today" rather than as a
    /// configuration error.
    pub fn quota_exhausted(&self) -> bool {
        self.sendable && self.quota_remaining_sends() == 0
    }

    /// Whole sends still possible today under the tracked quota.
    pub fn quota_remaining_sends(&self) -> u32 {
        self.quota
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remaining_sends()
    }

    /// Enqueue a chat message for `liveChatMessages.insert`. Returns `false`
    /// when the worker is disabled, the account is an observer (no
    /// credentials or `liveChatId`), or the text is empty. Never blocks.
    pub fn send_message(&self, text: &str) -> bool {
        let text = text.trim();
        if !self.sendable || text.is_empty() {
            return false;
        }
        match &self.tx {
            Some(tx) => {
                let _ = tx.try_send(Msg::SendMessage(text.to_owned()));
                true
            }
            None => false,
        }
    }

    /// Reply to a specific chat line via the insert snippet's `parentId`
    /// (YouTube's counterpart to Twitch's `reply-parent-msg-id`). Returns
    /// `false` for an observer account or an empty parent id.
    pub fn send_reply(&self, text: &str, parent_id: &str) -> bool {
        let text = text.trim();
        let parent = parent_id.trim();
        if !self.sendable || text.is_empty() || parent.is_empty() {
            return false;
        }
        match &self.tx {
            Some(tx) => {
                let _ = tx.try_send(Msg::SendReply {
                    text: text.to_owned(),
                    parent: parent.to_owned(),
                });
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
    quota: Arc<Mutex<YouTubeQuota>>,
) {
    let mut backoff = 1u64;
    while !stop.load(Ordering::SeqCst) {
        // Deliberately *not* Connected yet: the session owns that transition
        // and performs it once the live-chat page actually answered. Marking
        // it here would let the dock report "Connected" for a worker that is
        // still in its first HTTP request, and a failed first attempt would
        // then be hidden by a backoff sleep instead of being visible.
        conn.store(2, Ordering::SeqCst);
        match run_session(&cfg, &msg_tx, &alert_tx, &rx, &quota, &conn) {
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

/// Drain the outbound queue and send each entry through the official API.
///
/// Runs on the worker thread so the poll loop keeps its shape. A send is
/// charged against the daily quota *before* the request leaves, so an
/// over-budget bot stops locally; a `quotaExceeded` from the server exhausts
/// the tracker for the rest of the day and every later queued message is
/// dropped without a request. Static messages only — the config carries the
/// API key and OAuth token (rust/cleartext-logging).
fn drain_outbound(
    rx: &Receiver<Msg>,
    cfg: &YouTubeChatConfig,
    quota: &Mutex<YouTubeQuota>,
) -> bool {
    while let Ok(msg) = rx.try_recv() {
        let (text, parent) = match msg {
            Msg::Disconnect => return true,
            Msg::SendMessage(text) => (text, None),
            Msg::SendReply { text, parent } => (text, Some(parent)),
        };
        let mut tracker = quota.lock().unwrap_or_else(|e| e.into_inner());
        if !tracker.try_acquire() {
            tracing::warn!("YouTube send dropped: daily API quota exhausted");
            continue;
        }
        let outcome = match cfg.send_credentials.as_ref() {
            Some(credentials) => youtube_send_message(
                &cfg.send_endpoint,
                credentials,
                &cfg.live_chat_id,
                &text,
                parent.as_deref(),
            ),
            None => YouTubeSendOutcome::Rejected,
        };
        match outcome {
            YouTubeSendOutcome::Sent => {}
            YouTubeSendOutcome::QuotaExhausted => {
                tracker.exhaust();
                tracing::warn!("YouTube send rejected: API quota exhausted for today");
            }
            YouTubeSendOutcome::Rejected => {
                tracing::warn!("YouTube send rejected by the Live Streaming API");
            }
        }
    }
    false
}

fn run_session(
    cfg: &YouTubeChatConfig,
    msg_tx: &Sender<ChatMessage>,
    alert_tx: &Sender<crate::alerts_ingest::AlertEvent>,
    rx: &Receiver<Msg>,
    quota: &Mutex<YouTubeQuota>,
    conn: &std::sync::atomic::AtomicU8,
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

    // The connection exists from here: the page answered and a continuation
    // token was found, so the poll loop is live. This is the transition the
    // dock's status line reports, and it must not happen before it is true.
    conn.store(1, Ordering::SeqCst);

    // Static message: no config-derived value in log sinks (rust/
    // cleartext-logging; the video id derives from the same struct as
    // the token).
    tracing::info!("YouTube chat connected");

    loop {
        // Outbound first: a queued message must not wait for the next poll
        // cycle, and a disconnect must not be stuck behind a 3 s sleep.
        if drain_outbound(rx, cfg, quota) {
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
    use crate::test_helpers::wait_until;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicU64;

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

    /// Regression: the worker used to store `Connected` at the top of the
    /// retry loop, so the dock reported "Connected" while the first HTTP
    /// request was still in flight — and a failed first attempt hid behind
    /// an exponentially growing backoff (1…30 s). On a loaded CI runner that
    /// race turned `chat_send_input_follows_worker_capability_not_the_platform_list`
    /// red while Windows stayed green.
    ///
    /// Pinned here rather than in the GUI test because the bug is in the
    /// core: the transition belongs to the session, not the retry loop.
    #[test]
    fn the_worker_is_not_connected_before_the_live_chat_page_answers() {
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicU8, Ordering};

        // Bind but never accept: the connect succeeds, the response never
        // does. That is the state the old code called Connected.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        std::thread::spawn(move || {
            // Hold the listener open without answering for the whole test.
            std::thread::sleep(Duration::from_millis(800));
            drop(listener);
        });

        let conn = std::sync::Arc::new(AtomicU8::new(0));
        let (msg_tx, _msg_rx) = unbounded();
        let (alert_tx, _alert_rx) = unbounded();
        let cfg = YouTubeChatConfig {
            channel: "abc123".to_owned(),
            page_endpoint: format!("http://{addr}/live_chat?is_popout=1&v="),
            poll_endpoint: format!("http://{addr}/get_live_chat"),
            ..Default::default()
        };
        let (_tx, rx): (_, Receiver<Msg>) = unbounded();
        let quota = Mutex::new(YouTubeQuota::new(YouTubeQuotaConfig::default()));
        // The test keeps its own handle to observe the same flag the session
        // writes, which is the whole point of the assertion.
        let session_conn = std::sync::Arc::clone(&conn);
        std::thread::spawn(move || {
            let _ = run_session(&cfg, &msg_tx, &alert_tx, &rx, &quota, &session_conn);
        });

        // Give the page request time to be in flight; the state must still
        // not claim a connection that does not exist.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            conn.load(Ordering::SeqCst),
            0,
            "the worker must not report Connected before the page answered"
        );

        // Sanity check on the mapping used by `connection_state`.
        let reported = match conn.load(Ordering::SeqCst) {
            2 => ChatConnState::Disconnected,
            1 => ChatConnState::Connected,
            _ => ChatConnState::Off,
        };
        assert_eq!(reported, ChatConnState::Off);
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
            ..Default::default()
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
        let msg = wait_until(Duration::from_secs(20), || {
            if let Some(rx) = chat.messages() {
                if let Ok(msg) = rx.try_recv() {
                    return Some(msg);
                }
            }
            if chat.connection_state() == ChatConnState::Disconnected {
                // Fail fast instead of waiting out the deadline: after a
                // session error the worker retries against the already
                // exhausted local listener, which can never succeed.
                panic!("worker disconnected before delivering the message");
            }
            None
        })
        .expect("message delivered");
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
            ..Default::default()
        };
        let mut chat = YouTubeChat::new(&cfg);
        assert!(chat.enabled());

        // The plain chat line arrives on the message receiver…
        let chat_delivered = wait_until(Duration::from_secs(20), || {
            if let Some(rx) = chat.messages() {
                if let Ok(msg) = rx.try_recv() {
                    return Some(msg);
                }
            }
            if chat.connection_state() == ChatConnState::Disconnected {
                panic!("worker disconnected before delivering the chat line");
            }
            None
        });
        assert_eq!(chat_delivered.expect("chat delivered").text, "plain line");

        // …and the Super Chat lands on the alert receiver.
        let alert_delivered = wait_until(Duration::from_secs(20), || {
            if let Some(rx) = chat.alerts() {
                if let Ok(event) = rx.try_recv() {
                    return Some(event);
                }
            }
            if chat.connection_state() == ChatConnState::Disconnected {
                panic!("worker disconnected before delivering the alert");
            }
            None
        });
        let event = alert_delivered.expect("alert delivered");
        assert_eq!(event.kind, crate::alerts_ingest::AlertKind::Donation);
        assert_eq!(event.user, "WsDonor");
        assert_eq!(event.amount, Some(7.5));

        chat.disconnect();
    }

    // ------------------------------------------------------- send credentials

    #[test]
    fn credentials_are_masked_in_debug_output() {
        let creds = YouTubeSendCredentials::new(
            "AIzaSyTOPSECRETKEY".to_owned(),
            "ya29.a0AfB-SECRETTOKEN".to_owned(),
        );
        let rendered = format!("{creds:?}");
        assert!(
            !rendered.contains("AIza"),
            "API key must not be printed: {rendered}"
        );
        assert!(
            !rendered.contains("ya29"),
            "token must not be printed: {rendered}"
        );
        assert!(
            !rendered.contains("<missing>"),
            "filled slots are not missing"
        );
        assert!(creds.is_complete());
    }

    #[test]
    fn half_configured_credentials_are_not_sendable() {
        assert!(!YouTubeSendCredentials::new("key".to_owned(), "  ".to_owned()).is_complete());
        assert!(!YouTubeSendCredentials::new(String::new(), "token".to_owned()).is_complete());
        assert!(YouTubeSendCredentials::new("key".to_owned(), "token".to_owned()).is_complete());
        let missing = YouTubeSendCredentials::new(String::new(), "   ".to_owned());
        assert!(format!("{missing:?}").contains("<missing>"));
    }

    // ------------------------------------------------------------ insert body

    #[test]
    fn insert_body_carries_the_official_snippet_fields() {
        let body = youtube_insert_body("LC_CHAT", "hello there", None);
        assert!(body.contains("part=snippet"), "{body}");
        assert!(body.contains("snippet.liveChatId=LC_CHAT"), "{body}");
        assert!(body.contains("snippet.type=textMessageEvent"), "{body}");
        assert!(
            body.contains("snippet.textOriginal=hello%20there"),
            "{body}"
        );
        assert!(
            !body.contains("parentId"),
            "a plain message must not invent a parent: {body}"
        );
    }

    #[test]
    fn insert_body_threads_a_reply_via_parent_id() {
        let body = youtube_insert_body("LC_CHAT", "answer", Some("PARENT 1"));
        assert!(body.contains("snippet.parentId=PARENT%201"), "{body}");
        // An empty or blank parent id is not a reply.
        let blank = youtube_insert_body("LC_CHAT", "answer", Some("   "));
        assert!(!blank.contains("parentId"), "{blank}");
    }

    #[test]
    fn form_encoding_escapes_everything_outside_the_unreserved_set() {
        assert_eq!(form_encode_component("abcXYZ019-_.~"), "abcXYZ019-_.~");
        assert_eq!(form_encode_component("a b"), "a%20b");
        assert_eq!(form_encode_component("a&b=c"), "a%26b%3Dc");
        assert_eq!(form_encode_component("100%"), "100%25");
        assert_eq!(
            form_encode_component("gr\u{00fc}\u{00df}e"),
            "gr%C3%BC%C3%9Fe"
        );
    }

    // ------------------------------------------------------------------ quota

    /// Manual unix-seconds clock so a test can cross the UTC day boundary
    /// without sleeping.
    fn manual_quota_clock() -> (Arc<AtomicU64>, impl Fn() -> u64) {
        let secs = Arc::new(AtomicU64::new(0));
        let tick = Arc::clone(&secs);
        (secs, move || tick.load(Ordering::Relaxed))
    }

    #[test]
    fn documented_quota_allows_fifty_sends_per_day() {
        let config = YouTubeQuotaConfig::documented();
        assert_eq!(config.insert_units, 200);
        assert_eq!(config.daily_units, 10_000);
        assert_eq!(config.sends_per_day(), 50);
        assert_eq!(YouTubeQuotaConfig::default(), config);
    }

    #[test]
    fn quota_stops_sends_once_the_daily_budget_is_spent() {
        // This test never moves the clock, so the day never rolls.
        let (_cell, clock) = manual_quota_clock();
        let mut quota = YouTubeQuota::with_clock(YouTubeQuotaConfig::documented(), clock);
        for _ in 0..50 {
            assert!(
                quota.try_acquire(),
                "the documented budget allows 50 inserts"
            );
        }
        assert_eq!(quota.spent_units(), 10_000);
        assert_eq!(quota.remaining_units(), 0);
        assert_eq!(quota.remaining_sends(), 0);
        assert!(
            !quota.try_acquire(),
            "the 51st insert must not be charged against an empty budget"
        );
        assert_eq!(
            quota.spent_units(),
            10_000,
            "a rejected acquire must not push the counter past the budget"
        );
    }

    #[test]
    fn quota_resets_at_the_utc_day_boundary() {
        let (cell, clock) = manual_quota_clock();
        let mut quota = YouTubeQuota::with_clock(YouTubeQuotaConfig::documented(), clock);
        for _ in 0..50 {
            assert!(quota.try_acquire());
        }
        assert!(!quota.try_acquire());
        // One second before midnight is still the same day.
        cell.store(SECS_PER_DAY - 1, Ordering::Relaxed);
        assert!(
            !quota.try_acquire(),
            "still out of budget before the day rolls"
        );
        // First second of the next day: the budget is back.
        cell.store(SECS_PER_DAY, Ordering::Relaxed);
        assert!(quota.try_acquire(), "a new UTC day restores the budget");
        assert_eq!(quota.spent_units(), 200);
    }

    #[test]
    fn server_side_quota_exhaustion_stops_the_tracker_early() {
        let (_cell, clock) = manual_quota_clock();
        let mut quota = YouTubeQuota::with_clock(YouTubeQuotaConfig::documented(), clock);
        assert!(quota.try_acquire());
        quota.exhaust();
        assert!(
            !quota.try_acquire(),
            "other clients sharing the project can exhaust it before we do"
        );
        assert_eq!(quota.remaining_sends(), 0);
    }

    #[test]
    fn quota_classification_reads_the_api_error_reason() {
        assert!(youtube_quota_exhausted(
            r#"{"error":{"errors":[{"reason":"quotaExceeded"}]}}"#
        ));
        assert!(youtube_quota_exhausted(
            r#"{"error":{"errors":[{"reason":"dailyLimitExceeded"}]}}"#
        ));
        assert!(youtube_quota_exhausted(
            r#"{"error":{"errors":[{"reason":"rateLimitExceeded"}]}}"#
        ));
        assert!(!youtube_quota_exhausted(
            r#"{"error":{"errors":[{"reason":"forbidden","message":"Insufficient permissions"}]}}"#
        ));
        assert!(!youtube_quota_exhausted(""));
    }

    // ------------------------------------------------------- outcome plumbing

    #[test]
    fn incomplete_send_setup_is_rejected_without_a_request() {
        let creds = YouTubeSendCredentials::new("key".to_owned(), "token".to_owned());
        // An unreachable endpoint proves no request is attempted: without the
        // pre-flight guard this would take the transport error path instead.
        let dead = "http://127.0.0.1:1/youtube/v3/liveChat/messages";
        assert_eq!(
            youtube_send_message(dead, &creds, "", "hi", None),
            YouTubeSendOutcome::Rejected,
            "an empty liveChatId must be refused up front"
        );
        assert_eq!(
            youtube_send_message(dead, &creds, "LC", "   ", None),
            YouTubeSendOutcome::Rejected,
            "an empty message must be refused up front"
        );
        let half = YouTubeSendCredentials::new(String::new(), "token".to_owned());
        assert_eq!(
            youtube_send_message(dead, &half, "LC", "hi", None),
            YouTubeSendOutcome::Rejected,
            "an incomplete credential pair must be refused up front"
        );
    }

    // ------------------------------------------------------------- local server

    /// One captured request: the request head and the body it carried.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct CapturedRequest {
        head: String,
        body: String,
    }

    impl CapturedRequest {
        /// Everything the client sent, so a test can assert on the wire
        /// format without caring where the head ended.
        fn all(&self) -> String {
            format!("{}\n{}", self.head, self.body)
        }
    }

    /// Answer `count` requests with `body` and hand the captured requests to
    /// the caller, so the insert test can assert on the real wire format.
    fn recording_server(
        count: usize,
        status_line: &'static str,
        body: &'static str,
    ) -> (String, std::sync::mpsc::Receiver<CapturedRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..count {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let Ok(captured) = capture_request(&mut stream) else {
                    return;
                };
                let response = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                let _ = tx.send(captured);
            }
        });
        (format!("http://{addr}/youtube/v3/liveChat/messages"), rx)
    }

    /// Read one complete request off the socket and keep what it said.
    ///
    /// `drain_request` throws the bytes away; this variant is the same read
    /// loop with the result preserved, so the send tests can prove the
    /// request really carried the `snippet.*` fields. The body length comes
    /// from `Content-Length`, and the full request is consumed *before* the
    /// response goes out (see `drain_request` for why that matters on
    /// Windows).
    fn capture_request(stream: &mut std::net::TcpStream) -> std::io::Result<CapturedRequest> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Ok(CapturedRequest {
                    head: String::from_utf8_lossy(&buf).into_owned(),
                    body: String::new(),
                });
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
        let content_length = head
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
            buf.extend_from_slice(&chunk[..n]);
            received += n;
        }
        let body = String::from_utf8_lossy(&buf[header_end.min(buf.len())..]).into_owned();
        Ok(CapturedRequest { head, body })
    }

    #[test]
    fn insert_reaches_the_official_endpoint_and_carries_bearer_and_key() {
        let (endpoint, rx) = recording_server(1, "HTTP/1.1 200 OK", r#"{"id":"msg-1"}"#);
        let creds = YouTubeSendCredentials::new("AIzaKEY".to_owned(), "ya29.TOKEN".to_owned());
        assert_eq!(
            youtube_send_message(&endpoint, &creds, "LC_CHAT", "hello there", None),
            YouTubeSendOutcome::Sent
        );
        let request = rx.recv_timeout(Duration::from_secs(10)).expect("request");
        assert!(
            request
                .head
                .starts_with("POST /youtube/v3/liveChat/messages?part=snippet&key=AIzaKEY"),
            "unexpected request line: {request:?}"
        );
        assert!(
            request.head.contains("authorization: Bearer ya29.TOKEN"),
            "{request:?}"
        );
        assert_eq!(
            request.body,
            "part=snippet&snippet.liveChatId=LC_CHAT&snippet.type=textMessageEvent\
&snippet.textOriginal=hello%20there",
            "the request body must be the documented insert form: {request:?}"
        );
    }

    #[test]
    fn quota_refusal_is_classified_as_exhausted_not_as_a_bad_token() {
        let (endpoint, _rx) = recording_server(
            1,
            "HTTP/1.1 403 Forbidden",
            r#"{"error":{"errors":[{"reason":"quotaExceeded","message":"Quota exceeded"}]}}"#,
        );
        let creds = YouTubeSendCredentials::new("key".to_owned(), "token".to_owned());
        assert_eq!(
            youtube_send_message(&endpoint, &creds, "LC", "hi", None),
            YouTubeSendOutcome::QuotaExhausted
        );
    }

    #[test]
    fn permission_refusal_is_classified_as_rejected() {
        let (endpoint, _rx) = recording_server(
            1,
            "HTTP/1.1 403 Forbidden",
            r#"{"error":{"errors":[{"reason":"forbidden","message":"Insufficient permissions"}]}}"#,
        );
        let creds = YouTubeSendCredentials::new("key".to_owned(), "token".to_owned());
        assert_eq!(
            youtube_send_message(&endpoint, &creds, "LC", "hi", None),
            YouTubeSendOutcome::Rejected
        );
    }

    // ------------------------------------------------------- worker send path

    /// Full worker test: the read path polls from one local listener while the
    /// official insert goes to a second, and the recorded request must carry
    /// the reply's `parentId`.
    #[test]
    fn worker_sends_through_the_official_api_and_threads_replies() {
        let poll_listener = TcpListener::bind("127.0.0.1:0").expect("bind poll");
        let poll_addr = poll_listener.local_addr().expect("addr poll");
        let (insert_endpoint, insert_rx) =
            recording_server(1, "HTTP/1.1 200 OK", r#"{"id":"sent-1"}"#);

        std::thread::spawn(move || {
            // Initial page GET, then one get_live_chat poll POST.
            for index in 0..2 {
                let Ok((mut stream, _)) = poll_listener.accept() else {
                    return;
                };
                let _ = drain_request(&mut stream);
                let body = if index == 0 {
                    r#"<script>var ytInitialData={"continuation":"TOKEN_1"};</script>"#.to_owned()
                } else {
                    payload_fixture().to_owned()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        let mut chat = YouTubeChat::new(&YouTubeChatConfig {
            page_endpoint: format!("http://{poll_addr}/live_chat?is_popout=1&v="),
            poll_endpoint: format!("http://{poll_addr}/youtubei/v1/live_chat/get_live_chat"),
            send_endpoint: insert_endpoint,
            channel: "VIDEO1".to_owned(),
            live_chat_id: "LC_CHAT".to_owned(),
            send_credentials: Some(YouTubeSendCredentials::new(
                "AIzaKEY".to_owned(),
                "ya29.TOKEN".to_owned(),
            )),
            quota: Some(YouTubeQuotaConfig::documented()),
        });
        assert!(
            chat.can_send(),
            "a fully configured account must be sendable"
        );
        assert_eq!(chat.quota_remaining_sends(), 50);

        let connected = wait_until(Duration::from_secs(20), || {
            (chat.connection_state() == ChatConnState::Connected).then_some(())
        });
        assert!(connected.is_some(), "worker must reach the connected state");

        assert!(chat.send_reply("answer", "PARENT-1"));
        let request = insert_rx
            .recv_timeout(Duration::from_secs(20))
            .expect("insert request must reach the official endpoint");
        assert!(
            request.all().contains("snippet.parentId=PARENT-1"),
            "the reply must carry parentId: {request:?}"
        );
        assert!(
            request.all().contains("snippet.textOriginal=answer"),
            "{request:?}"
        );

        chat.disconnect();
    }

    #[test]
    fn worker_stays_read_only_when_the_send_contract_is_incomplete() {
        for (label, mut cfg) in [
            (
                "no credentials at all",
                YouTubeChatConfig {
                    channel: "VIDEO1".to_owned(),
                    ..Default::default()
                },
            ),
            (
                "token without api key",
                YouTubeChatConfig {
                    channel: "VIDEO1".to_owned(),
                    live_chat_id: "LC".to_owned(),
                    send_credentials: Some(YouTubeSendCredentials::new(
                        String::new(),
                        "token".to_owned(),
                    )),
                    ..Default::default()
                },
            ),
            (
                "api key without token",
                YouTubeChatConfig {
                    channel: "VIDEO1".to_owned(),
                    live_chat_id: "LC".to_owned(),
                    send_credentials: Some(YouTubeSendCredentials::new(
                        "key".to_owned(),
                        "   ".to_owned(),
                    )),
                    ..Default::default()
                },
            ),
            (
                "credentials without a liveChatId",
                YouTubeChatConfig {
                    channel: "VIDEO1".to_owned(),
                    send_credentials: Some(YouTubeSendCredentials::new(
                        "key".to_owned(),
                        "token".to_owned(),
                    )),
                    ..Default::default()
                },
            ),
        ] {
            cfg.page_endpoint = "http://127.0.0.1:1/live_chat".to_owned();
            cfg.poll_endpoint = "http://127.0.0.1:1/poll".to_owned();
            let chat = YouTubeChat::new(&cfg);
            assert!(!chat.can_send(), "{label} must not be sendable");
            assert!(
                !chat.quota_exhausted(),
                "{label} is a config gap, not a quota gap"
            );
            assert!(!chat.send_message("hello"), "{label} must refuse sends");
            assert!(
                !chat.send_reply("hello", "parent"),
                "{label} must refuse replies"
            );
        }
    }

    #[test]
    fn a_spent_budget_turns_a_configured_account_into_an_observer() {
        // A one-unit budget is the smallest possible: one insert exhausts it.
        let chat = YouTubeChat::new(&YouTubeChatConfig {
            channel: "VIDEO1".to_owned(),
            page_endpoint: "http://127.0.0.1:1/live_chat".to_owned(),
            poll_endpoint: "http://127.0.0.1:1/poll".to_owned(),
            live_chat_id: "LC".to_owned(),
            send_credentials: Some(YouTubeSendCredentials::new(
                "key".to_owned(),
                "token".to_owned(),
            )),
            quota: Some(YouTubeQuotaConfig {
                insert_units: 1,
                daily_units: 2,
            }),
            ..Default::default()
        });
        assert!(chat.can_send(), "two units still allow two sends");
        chat.quota.lock().unwrap().try_acquire();
        chat.quota.lock().unwrap().try_acquire();
        assert_eq!(chat.quota_remaining_sends(), 0);
        assert!(!chat.can_send(), "a spent budget means read-only for today");
        assert!(
            chat.quota_exhausted(),
            "the dock must be able to say 'read-only today' rather than 'misconfigured'"
        );
    }
}
