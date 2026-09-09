//! Native alert event ingestion for the chat dock (follows/subs/donations/raids).
//!
//! This module provides the deterministic *ingestion contract* for the M5
//! "Alerts (follows/subs/donations)" roadmap row:
//!
//! - parsing provider webhook payloads (**Streamlabs** donations and **Twitch
//!   EventSub** follow / subscribe / gifted-subscription / raid notifications)
//!   into a flat, privacy-safe [`AlertEvent`] model that carries **no tokens**
//!   (user names, amounts and optional messages only);
//! - Twitch EventSub **HMAC-SHA-256 signature verification** so webhook
//!   notifications can be authenticated against the subscription secret
//!   (Twitch spec: `sha256=`-prefixed hex HMAC over
//!   `message-id || message-timestamp || raw-body`);
//! - a bounded, purely local [`AlertIngest`] queue the GUI drains into the chat
//!   dock, surfacing localized entries via [`crate::Locale::tr_fmt`].
//!
//! **Honest scope:** like telemetry, this ships **no network receiver**. The
//! HTTPS endpoint those payloads would arrive on (or the EventSub WebSocket)
//! is a documented follow-up ([`docs/alerts-ingest.md`]); nothing in the
//! shipped build listens on any socket for alerts. Ingestion is testable
//! deterministically because parsing, signature verification and the queue are
//! all network-free.
//!
//! Privacy posture: `AlertEvent` intentionally has **no `Serialize` impl** (an
//! event can never be serialized out of the app), `Debug` renders the queue
//! without ever touching entry contents, and helper methods validate event
//! data so names/messages stay free of path-like or control characters.

use std::collections::VecDeque;
use std::fmt;

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Bounded size of the local alert queue (oldest entry dropped when full).
pub const DEFAULT_ALERT_QUEUE_CAPACITY: usize = 64;

/// The kind of engagement event an ingest entry represents.
///
/// Each variant maps to exactly one i18n key (the parity test keeps EN and DE
/// catalogs in lock-step), so the GUI renders alerts with localized wording
/// instead of the provider's English strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    /// `channel.follow` (EventSub v2) / provider follow event.
    Follow,
    /// `channel.subscribe` (incl. gifted-but-individually-surfaced subs).
    Subscribe,
    /// `channel.subscription.gift` (a viewer gifted multiple subs).
    GiftSub,
    /// Streamlabs `donation` webhook.
    Donation,
    /// `channel.raid`.
    Raid,
}

impl AlertKind {
    /// All kinds in UI display order.
    pub fn all() -> &'static [AlertKind] {
        &[
            AlertKind::Follow,
            AlertKind::Subscribe,
            AlertKind::GiftSub,
            AlertKind::Donation,
            AlertKind::Raid,
        ]
    }

    /// i18n key used by the GUI to render a localized alert line.
    pub fn i18n_key(self) -> &'static str {
        match self {
            AlertKind::Follow => "alert_kind_follow",
            AlertKind::Subscribe => "alert_kind_subscribe",
            AlertKind::GiftSub => "alert_kind_giftsub",
            AlertKind::Donation => "alert_kind_donation",
            AlertKind::Raid => "alert_kind_raid",
        }
    }
}

/// A fully-parsed alert ingestion entry.
///
/// Deliberately **not** `Serialize`: entries must never leave the app. The
/// `message` field is kept strictly local and displayed only inside the chat
/// dock.
#[derive(Debug, Clone)]
pub struct AlertEvent {
    pub kind: AlertKind,
    /// Display name of the acting viewer (follower, subscriber, gifter,
    /// raider or donor). `"Anonymous"` for anonymous gifted subs.
    pub user: String,
    /// Secondary actor for GiftSub: the subscription *recipient*, when named.
    pub recipient: Option<String>,
    /// Gifted-sub count / raid viewer count. `0` for other kinds.
    pub count: u32,
    /// Tier label for subscriptions (e.g. `"Tier 1"`). `None` otherwise.
    pub tier: Option<String>,
    /// Donation amount in the provider's units.
    pub amount: Option<f64>,
    /// Donation currency code when the provider supplies one (`"USD"`,
    /// `"EUR"`, ...). `None` when unknown.
    pub currency: Option<String>,
    /// Optional free-form viewer message (donation/sub message). Kept local,
    /// validated (no control characters), never echoed back to any service.
    pub message: Option<String>,
    /// Unix timestamp (seconds) from the payload, `0` when absent.
    pub timestamp: u64,
}

impl PartialEq for AlertEvent {
    /// Structural equality that treats `NaN` amount as equal memberwise (so
    /// hand-`f64` don't opt out of `Eq`-ful comparisons in tests).
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.user == other.user
            && self.recipient == other.recipient
            && self.count == other.count
            && self.tier == other.tier
            && amount_eq(self.amount, other.amount)
            && self.currency == other.currency
            && self.message == other.message
            && self.timestamp == other.timestamp
    }
}

/// `f64` equality that is well-defined for `Option` members (both `None`, or
/// bit-equal `Some`). Used instead of deriving `PartialEq` so the model stays
/// `Copy`-friendly and the intent is explicit.
fn amount_eq(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

impl Eq for AlertEvent {}

impl AlertEvent {
    /// A deterministic sample follow used by the chat-dock "preview" button and
    /// by GUI tests (so the layout is testable without a live stream).
    pub fn sample_follow() -> AlertEvent {
        AlertEvent {
            kind: AlertKind::Follow,
            user: "PreviewViewer".to_owned(),
            recipient: None,
            count: 0,
            tier: None,
            amount: None,
            currency: None,
            message: None,
            timestamp: 0,
        }
    }

    /// Validate/modify a user-supplied message so it stays safe to render in
    /// the chat dock: control characters are stripped and the result is
    /// trimmed. Returns `None` when nothing but whitespace remains.
    pub fn sanitize_message(raw: &str) -> Option<String> {
        let cleaned: String = raw
            .chars()
            .filter(|c| !c.is_control() && !matches!(c, '\u{200b}' | '\u{200e}' | '\u{200f}'))
            .collect::<String>()
            .trim()
            .to_owned();
        if cleaned.is_empty() {
            None
        } else {
            Some(cleaned)
        }
    }
}

/// Errors raised while parsing or authenticating alert payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertIngestError {
    /// The payload is not valid JSON.
    InvalidJson(String),
    /// The payload's `type` is not an ingestable alert kind.
    UnsupportedKind(String),
    /// A required field is missing or has the wrong shape.
    MissingField(String),
    /// The Twitch EventSub HMAC signature did not match the body.
    SignatureMismatch,
}

impl fmt::Display for AlertIngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(detail) => write!(f, "alert payload is not valid JSON: {detail}"),
            Self::UnsupportedKind(kind) => {
                write!(f, "alert payload type is not ingestable: {kind}")
            }
            Self::MissingField(field) => write!(f, "alert payload is missing `{field}`"),
            Self::SignatureMismatch => {
                write!(
                    f,
                    "Twitch EventSub signature mismatch (secret or body differs)"
                )
            }
        }
    }
}

impl std::error::Error for AlertIngestError {}

fn json_string<'a>(v: &'a serde_json::Value, path: &str) -> Option<&'a str> {
    v.get(path).and_then(|x| x.as_str())
}

fn json_optional_string(v: &serde_json::Value, path: &str) -> Option<String> {
    v.get(path)
        .and_then(|x| x.as_str())
        .map(str::to_owned)
        .map(|s| s.to_owned())
}

fn json_u32(v: &serde_json::Value, path: &str) -> Option<u32> {
    v.get(path).and_then(|x| x.as_u64()).map(|n| n as u32)
}

fn json_f64(v: &serde_json::Value, path: &str) -> Option<f64> {
    v.get(path).and_then(|x| x.as_f64())
}

/// Map an EventSub tier code (`"1000"`/`"2000"`/`"3000"`) to a display label;
/// unknown codes pass through as `Tier <code>`.
fn eventsub_tier_label(tier: &str) -> String {
    match tier {
        "1000" => "Tier 1".to_owned(),
        "2000" => "Tier 2".to_owned(),
        "3000" => "Tier 3".to_owned(),
        other => format!("Tier {other}"),
    }
}

/// Parse a Streamlabs alert webhook payload into an [`AlertEvent`].
///
/// Supported payload types (chat-dock ingestable): `donation`. The Streamlabs
/// shape is `{"type": "...", "message": [{"name", "amount",
/// "formatted_amount", "message"}]}`.
pub fn parse_streamlabs_webhook(json: &str) -> Result<AlertEvent, AlertIngestError> {
    let value: serde_json::Value = serde_json::from_str(json.trim())
        .map_err(|e| AlertIngestError::InvalidJson(e.to_string()))?;
    let kind =
        json_string(&value, "type").ok_or_else(|| AlertIngestError::MissingField("type".into()))?;
    match kind {
        "donation" => {
            let items = value
                .get("message")
                .and_then(|m| m.as_array())
                .ok_or_else(|| AlertIngestError::MissingField("message".into()))?;
            let first = items
                .first()
                .ok_or_else(|| AlertIngestError::MissingField("message[0]".into()))?;
            let user = json_string(first, "name")
                .map(str::to_owned)
                .unwrap_or_else(|| "Anonymous".to_owned());
            let amount = json_f64(first, "amount");
            let currency = json_string(first, "currency").map(str::to_owned);
            let message = json_string(first, "message").and_then(AlertEvent::sanitize_message);
            Ok(AlertEvent {
                kind: AlertKind::Donation,
                user,
                recipient: None,
                count: 0,
                tier: None,
                amount,
                currency,
                message,
                timestamp: 0,
            })
        }
        other => Err(AlertIngestError::UnsupportedKind(other.to_owned())),
    }
}

/// Parse a Twitch EventSub webhook notification envelope into an [`AlertEvent`].
///
/// Supported subscription types: `channel.follow` (v2), `channel.subscribe`,
/// `channel.subscription.gift` and `channel.raid`.
pub fn parse_twitch_eventsub_notification(json: &str) -> Result<AlertEvent, AlertIngestError> {
    let value: serde_json::Value = serde_json::from_str(json.trim())
        .map_err(|e| AlertIngestError::InvalidJson(e.to_string()))?;
    let subscription = value
        .get("subscription")
        .ok_or_else(|| AlertIngestError::MissingField("subscription".into()))?;
    let sub_type = json_string(subscription, "type")
        .ok_or_else(|| AlertIngestError::MissingField("subscription.type".into()))?;
    let event = value
        .get("event")
        .ok_or_else(|| AlertIngestError::MissingField("event".into()))?;

    match sub_type {
        "channel.follow" => {
            let user = json_optional_string(event, "user_name")
                .ok_or_else(|| AlertIngestError::MissingField("event.user_name".into()))?;
            Ok(AlertEvent {
                kind: AlertKind::Follow,
                user,
                recipient: None,
                count: 0,
                tier: None,
                amount: None,
                currency: None,
                message: None,
                timestamp: 0,
            })
        }
        "channel.subscribe" => {
            let user =
                json_optional_string(event, "user_name").unwrap_or_else(|| "Anonymous".to_owned());
            let tier = json_optional_string(event, "tier").map(|t| eventsub_tier_label(&t));
            Ok(AlertEvent {
                kind: AlertKind::Subscribe,
                user,
                recipient: None,
                count: 0,
                tier,
                amount: None,
                currency: None,
                message: None,
                timestamp: 0,
            })
        }
        "channel.subscription.gift" => {
            let user =
                json_optional_string(event, "user_name").unwrap_or_else(|| "Anonymous".to_owned());
            let tier = json_optional_string(event, "tier").map(|t| eventsub_tier_label(&t));
            let count = json_u32(event, "total").unwrap_or(1);
            Ok(AlertEvent {
                kind: AlertKind::GiftSub,
                user,
                recipient: json_optional_string(event, "recipient_user_name"),
                count,
                tier,
                amount: None,
                currency: None,
                message: None,
                timestamp: 0,
            })
        }
        "channel.raid" => {
            let user =
                json_optional_string(event, "from_broadcaster_user_name").ok_or_else(|| {
                    AlertIngestError::MissingField("event.from_broadcaster_user_name".into())
                })?;
            let viewers = json_u32(event, "viewers").unwrap_or(0);
            Ok(AlertEvent {
                kind: AlertKind::Raid,
                user,
                recipient: None,
                count: viewers,
                tier: None,
                amount: None,
                currency: None,
                message: None,
                timestamp: 0,
            })
        }
        other => Err(AlertIngestError::UnsupportedKind(other.to_owned())),
    }
}

/// Constant-time equality on byte slices (no early exit on the first
/// differing byte), used so a signature-mismatch answer gives no timing hint
/// about *where* the supplied digest diverges.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

fn extract_hex(signature: &str) -> &str {
    signature.strip_prefix("sha256=").unwrap_or(signature)
}

/// Verify a Twitch EventSub webhook signature (HMAC-SHA-256) against the
/// subscription secret.
///
/// Per the Twitch spec the HMAC input is the concatenation of the
/// `Twitch-Eventsub-Message-Id`, the `Twitch-Eventsub-Message-Timestamp` and
/// the raw request body. The provided signature (`sha256=<hex>`) is compared
/// in constant time.
pub fn verify_twitch_eventsub_signature(
    secret: &[u8],
    message_id: &str,
    message_timestamp: &str,
    body: &str,
    provided_signature: &str,
) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else {
        return false;
    };
    mac.update(message_id.as_bytes());
    mac.update(message_timestamp.as_bytes());
    mac.update(body.as_bytes());
    let expect = mac.finalize().into_bytes();
    // Parse the provided hex digest into bytes for the constant-time compare.
    let hex = extract_hex(provided_signature).to_ascii_lowercase();
    let Some(given) = hex_to_bytes(&hex) else {
        return false;
    };
    let expect: &[u8] = expect.as_ref();
    constant_time_eq(expect, &given)
}

/// Decode a lowercase hex string into raw bytes. `None` when the length is odd
/// or a non-hex character is present.
fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || hex.len() > 4096 {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().as_chunks::<2>().0 {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push(hi << 4 | lo);
    }
    Some(out)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Bounded, purely local alert queue drained by the GUI into the chat dock.
///
/// Mirror of the telemetry reporter's posture: deterministic, testable, and
/// offline by default. Entries are dropped oldest-first when the queue is at
/// capacity; `Debug` renders settings and counters **only**, never event
/// contents.
pub struct AlertIngest {
    enabled: bool,
    capacity: usize,
    entries: VecDeque<AlertEvent>,
    accepted_total: u64,
    skipped_disabled: u64,
    dropped_oldest: u64,
}

impl fmt::Debug for AlertIngest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Privacy: never touch entry contents in Debug output.
        f.debug_struct("AlertIngest")
            .field("enabled", &self.enabled)
            .field("capacity", &self.capacity)
            .field("len", &self.entries.len())
            .field("accepted_total", &self.accepted_total)
            .field("skipped_disabled", &self.skipped_disabled)
            .field("dropped_oldest", &self.dropped_oldest)
            .finish()
    }
}

impl Default for AlertIngest {
    fn default() -> Self {
        Self::new()
    }
}

impl AlertIngest {
    /// New queue, enabled by default, bounded to
    /// [`DEFAULT_ALERT_QUEUE_CAPACITY`].
    pub fn new() -> AlertIngest {
        AlertIngest {
            enabled: true,
            capacity: DEFAULT_ALERT_QUEUE_CAPACITY,
            entries: VecDeque::new(),
            accepted_total: 0,
            skipped_disabled: 0,
            dropped_oldest: 0,
        }
    }

    /// Turn ingestion on/off. Disabled pushes are counted, not queued.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Set the queue capacity. Shrinking drops the oldest entries.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
            self.dropped_oldest += 1;
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Queue an event. Returns `false` (and counts a skip) while disabled.
    pub fn push(&mut self, event: AlertEvent) -> bool {
        if !self.enabled {
            self.skipped_disabled += 1;
            return false;
        }
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
            self.dropped_oldest += 1;
        }
        self.entries.push_back(event);
        self.accepted_total += 1;
        true
    }

    /// Remove and return all queued entries (FIFO), clearing the queue.
    pub fn drain(&mut self) -> Vec<AlertEvent> {
        self.entries.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total events accepted since construction.
    pub fn accepted_total(&self) -> u64 {
        self.accepted_total
    }

    /// Events not queued because ingestion was disabled.
    pub fn skipped_disabled(&self) -> u64 {
        self.skipped_disabled
    }

    /// Events evicted because the queue was at capacity.
    pub fn dropped_oldest(&self) -> u64 {
        self.dropped_oldest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> &'static str {
        r#"{"subscription":{"type":"channel.follow"},"event":{"user_name":"Ada"}}"#
    }

    #[test]
    fn streamlabs_donation_maps_amount_currency_and_message() {
        let event = parse_streamlabs_webhook(
            r#"{"type":"donation","message":[{"name":"Kira","amount":20.0,"currency":"EUR","message":"keep it up!"}]}"#,
        )
        .unwrap();
        assert_eq!(event.kind, AlertKind::Donation);
        assert_eq!(event.user, "Kira");
        assert_eq!(event.amount, Some(20.0));
        assert_eq!(event.currency.as_deref(), Some("EUR"));
        assert_eq!(event.message.as_deref(), Some("keep it up!"));
    }

    #[test]
    fn streamlabs_anonymous_donation_defaults_name() {
        let event =
            parse_streamlabs_webhook(r#"{"type":"donation","message":[{"amount":5}]}"#).unwrap();
        assert_eq!(event.user, "Anonymous");
        assert_eq!(event.amount, Some(5.0));
    }

    #[test]
    fn streamlabs_malformed_payloads_are_rejected() {
        assert!(matches!(
            parse_streamlabs_webhook("not json"),
            Err(AlertIngestError::InvalidJson(_))
        ));
        assert!(matches!(
            parse_streamlabs_webhook(r#"{"type":"host","message":[]}"#),
            Err(AlertIngestError::UnsupportedKind(_))
        ));
        assert!(matches!(
            parse_streamlabs_webhook(r#"{"type":"donation"}"#),
            Err(AlertIngestError::MissingField(_))
        ));
    }

    #[test]
    fn eventsub_follow_is_parsed() {
        let event = parse_twitch_eventsub_notification(
            r#"{"subscription":{"type":"channel.follow","version":"2"},"event":{"user_name":"Ada"}}"#,
        )
        .unwrap();
        assert_eq!(event.kind, AlertKind::Follow);
        assert_eq!(event.user, "Ada");
    }

    #[test]
    fn eventsub_subscribe_maps_tier_and_optional_message() {
        let event = parse_twitch_eventsub_notification(
            r#"{"subscription":{"type":"channel.subscribe"},"event":{"user_name":"Grace","tier":"2000","message":{"text":"love the stream"}}}"#,
        )
        .unwrap();
        assert_eq!(event.kind, AlertKind::Subscribe);
        assert_eq!(event.user, "Grace");
        assert_eq!(event.tier.as_deref(), Some("Tier 2"));
        // subscription message is not part of the flat model here
        assert_eq!(event.message, None);
    }

    #[test]
    fn eventsub_subscribe_tolerates_missing_user() {
        let event = parse_twitch_eventsub_notification(
            r#"{"subscription":{"type":"channel.subscribe"},"event":{"tier":"1000"}}"#,
        )
        .unwrap();
        assert_eq!(event.user, "Anonymous");
    }

    #[test]
    fn eventsub_gift_and_raid_are_parsed() {
        let gift = parse_twitch_eventsub_notification(
            r#"{"subscription":{"type":"channel.subscription.gift"},"event":{"user_name":"Bo","total":5,"tier":"1000","recipient_user_name":"Tess"}}"#,
        )
        .unwrap();
        assert_eq!(gift.kind, AlertKind::GiftSub);
        assert_eq!(gift.user, "Bo");
        assert_eq!(gift.count, 5);
        assert_eq!(gift.recipient.as_deref(), Some("Tess"));
        assert_eq!(gift.tier.as_deref(), Some("Tier 1"));

        let raid = parse_twitch_eventsub_notification(
            r#"{"subscription":{"type":"channel.raid"},"event":{"from_broadcaster_user_name":"Boosted","viewers":42}}"#,
        )
        .unwrap();
        assert_eq!(raid.kind, AlertKind::Raid);
        assert_eq!(raid.user, "Boosted");
        assert_eq!(raid.count, 42);
    }

    #[test]
    fn eventsub_unknown_or_malformed_are_rejected() {
        assert!(matches!(
            parse_twitch_eventsub_notification(
                r#"{"subscription":{"type":"channel.ban"},"event":{}}"#,
            ),
            Err(AlertIngestError::UnsupportedKind(_))
        ));
        assert!(matches!(
            parse_twitch_eventsub_notification(r#"{"subscription":{"type":"channel.raid"}}"#),
            Err(AlertIngestError::MissingField(_))
        ));
        assert!(parse_twitch_eventsub_notification("garbage").is_err());
    }

    #[test]
    fn tier_codes_map_to_labels() {
        assert_eq!(eventsub_tier_label("1000"), "Tier 1");
        assert_eq!(eventsub_tier_label("2000"), "Tier 2");
        assert_eq!(eventsub_tier_label("3000"), "Tier 3");
        assert_eq!(eventsub_tier_label("9000"), "Tier 9000");
    }

    #[test]
    fn signature_verifies_against_the_lockstep_test_vector() {
        // Vector produced with a stock .NET HMACSHA256 over
        // "secret" for message_id "a1b2c3" + timestamp
        // "2026-09-09T12:00:00Z" + streamlabs test body used above.
        assert!(verify_twitch_eventsub_signature(
            b"secret",
            "a1b2c3",
            "2026-09-09T12:00:00Z",
            body(),
            "sha256=e02c3ea14f55867cad054c4df457d286cc2b7198a2941083f072ab217a882cbb",
        ));
    }

    #[test]
    fn signature_rejects_tampering_and_bad_secret() {
        let ok = "sha256=e02c3ea14f55867cad054c4df457d286cc2b7198a2941083f072ab217a882cbb";
        // Tampered body.
        assert!(!verify_twitch_eventsub_signature(
            b"secret",
            "a1b2c3",
            "2026-09-09T12:00:00Z",
            "{\"subscription\":{\"type\":\"channel.follow\"},\"event\":{\"user_name\":\"Mallory\"}}",
            ok,
        ));
        // Wrong secret.
        assert!(!verify_twitch_eventsub_signature(
            b"other",
            "a1b2c3",
            "2026-09-09T12:00:00Z",
            body(),
            ok,
        ));
        // Wrong message id / timestamp.
        assert!(!verify_twitch_eventsub_signature(
            b"secret",
            "zzz",
            "2026-09-09T12:00:00Z",
            body(),
            ok,
        ));
        // Malformed signature hex.
        assert!(!verify_twitch_eventsub_signature(
            b"secret",
            "a1b2c3",
            "2026-09-09T12:00:00Z",
            body(),
            "sha256=zz",
        ));
    }

    #[test]
    fn signature_accepts_without_prefix() {
        assert!(verify_twitch_eventsub_signature(
            b"secret",
            "a1b2c3",
            "2026-09-09T12:00:00Z",
            body(),
            "e02c3ea14f55867cad054c4df457d286cc2b7198a2941083f072ab217a882cbb",
        ));
    }

    #[test]
    fn queue_accepts_and_drains_fifo() {
        let mut ingest = AlertIngest::new();
        assert!(ingest.enabled());
        assert!(ingest.push(AlertEvent::sample_follow()));
        let second = AlertEvent {
            user: "Second".to_owned(),
            ..AlertEvent::sample_follow()
        };
        assert!(ingest.push(second.clone()));
        let drained = ingest.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].user, "PreviewViewer");
        assert_eq!(drained[1], second);
        assert!(ingest.is_empty());
        assert_eq!(ingest.accepted_total(), 2);
    }

    #[test]
    fn disabled_ingest_skips_and_counts() {
        let mut ingest = AlertIngest::new();
        ingest.set_enabled(false);
        assert!(!ingest.push(AlertEvent::sample_follow()));
        assert_eq!(ingest.len(), 0);
        assert_eq!(ingest.skipped_disabled(), 1);
        ingest.set_enabled(true);
        assert!(ingest.push(AlertEvent::sample_follow()));
        assert_eq!(ingest.len(), 1);
    }

    #[test]
    fn bounded_queue_evicts_oldest_and_counts() {
        let mut ingest = AlertIngest::new();
        ingest.set_capacity(2);
        ingest.push(AlertEvent {
            user: "A".to_owned(),
            ..AlertEvent::sample_follow()
        });
        ingest.push(AlertEvent {
            user: "B".to_owned(),
            ..AlertEvent::sample_follow()
        });
        ingest.push(AlertEvent {
            user: "C".to_owned(),
            ..AlertEvent::sample_follow()
        });
        assert_eq!(ingest.dropped_oldest(), 1);
        let drained = ingest.drain();
        assert_eq!(
            drained.iter().map(|e| e.user.as_str()).collect::<Vec<_>>(),
            ["B", "C"]
        );
        // Shrinking capacity evicts too.
        ingest.push(AlertEvent::sample_follow());
        ingest.push(AlertEvent {
            user: "D".to_owned(),
            ..AlertEvent::sample_follow()
        });
        ingest.set_capacity(1);
        assert_eq!(ingest.len(), 1);
        assert_eq!(ingest.dropped_oldest(), 2);
    }

    #[test]
    fn debug_never_leaks_entry_contents() {
        let mut ingest = AlertIngest::new();
        ingest.push(AlertEvent {
            user: "SuperSecretUserXYZ".to_owned(),
            message: Some("hunter2-recipe".to_owned()),
            ..AlertEvent::sample_follow()
        });
        let debug = format!("{ingest:?}");
        assert!(
            !debug.contains("SuperSecretUserXYZ"),
            "Debug leaks the user"
        );
        assert!(!debug.contains("hunter2"), "Debug leaks the message");
        assert!(
            debug.contains("enabled: true") && debug.contains("accepted_total: 1"),
            "Debug must still expose settings/counters"
        );
    }

    #[test]
    fn sanitize_message_strips_control_characters() {
        assert_eq!(
            AlertEvent::sanitize_message("  keep \u{200e}it up!  ").as_deref(),
            Some("keep it up!")
        );
        assert_eq!(AlertEvent::sanitize_message("\u{0000}\n\t"), None);
        assert_eq!(AlertEvent::sanitize_message("  \u{200b}  "), None);
    }
}
