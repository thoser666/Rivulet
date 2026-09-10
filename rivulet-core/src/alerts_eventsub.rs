//! Outbound **Twitch EventSub WebSocket** transport for the alert chat dock
//! (M5 alerts).
//!
//! Where [`crate::alerts_webhook`] is an *inbound* loopback endpoint (real
//! Twitch/Streamlabs deliveries always need a forwarder or HTTPS terminator in
//! front), this module dials **out to Twitch** (`wss://eventsub.wss.twitch.tv/ws`,
//! via `tungstenite` with the `rustls-tls-webpki-roots` feature) and keeps a
//! `channel.follow` / `channel.subscribe` /
//! `channel.subscription.gift` / `channel.raid` WebSocket session alive with
//! automatic reconnect. A live setup needs **no forwarder and no shared
//! secret** — a Twitch user access token (masked in Settings) plus the
//! broadcaster ID is enough to create the subscriptions against the Helix API.
//!
//! The protocol is implemented as pure, deterministically testable functions
//! ([`parse_eventsub_ws_message`], [`build_subscription_body`]); the network
//! half (WebSocket + Helix subscription POSTs) runs on a dedicated worker
//! thread with backoff/reconnect, mirroring [`crate::kick_chat::KickChat`].
//! Privacy posture matches the rest of the alerts stack: the token and client
//! ID are never logged, never `Debug`-printed and never embedded in events;
//! unknown/rejected frames are only counted.

use std::fmt;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::WebSocket;

use crate::alerts_ingest::{parse_twitch_eventsub_notification, AlertEvent, AlertIngestError};

/// Default Twitch EventSub WebSocket endpoint.
pub const DEFAULT_EVENTSUB_WS_ENDPOINT: &str = "wss://eventsub.wss.twitch.tv/ws";

/// Default Twitch Helix API base URL (subscription creation).
pub const DEFAULT_TWITCH_API_BASE: &str = "https://api.twitch.tv/helix";

/// Subscription types kept alive per session, matching the
/// [`crate::alerts_ingest::AlertKind`]s the local parsers support. Donation
/// stays on the Streamlabs webhook (Streamlabs has no WebSocket transport).
/// Format: `(subscription_type, version)`.
pub const EVENTSUB_ALERT_SUBSCRIPTIONS: &[(&str, &str)] = &[
    ("channel.follow", "2"),
    ("channel.subscribe", "1"),
    ("channel.subscription.gift", "1"),
    ("channel.raid", "1"),
];

/// Read timeout on the EventSub socket so the worker can honour `stop`
/// between reads (a blocking read would leak the thread on disable).
const SOCKET_READ_TIMEOUT: Duration = Duration::from_millis(500);

/// Grace added on top of the server-advertised keepalive timeout before the
/// worker declares the session dead and reconnects.
const KEEPALIVE_GRACE: Duration = Duration::from_secs(5);

/// Upper bound of the event channel between the worker and the GUI.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Worker settings persisted by the GUI (token masked, never logged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsubWsConfig {
    /// Twitch application client ID. Empty disables outgoing subscriptions.
    pub client_id: String,
    /// Twitch user access token for subscription creation (sent to the Helix
    /// API over TLS only). Empty disables the transport in the GUI.
    pub token: String,
    /// Target broadcaster numeric user ID the alerts belong to.
    pub broadcaster_id: String,
    /// EventSub WebSocket endpoint (Twitch default; tests point at a local
    /// puppet).
    pub ws_endpoint: String,
    /// Twitch Helix API base (subscription creation; tests use a local stub).
    pub api_base: String,
}

impl Default for EventsubWsConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            token: String::new(),
            broadcaster_id: String::new(),
            ws_endpoint: DEFAULT_EVENTSUB_WS_ENDPOINT.to_owned(),
            api_base: DEFAULT_TWITCH_API_BASE.to_owned(),
        }
    }
}

/// Errors raised while parsing an EventSub WebSocket frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventsubWsError {
    /// The frame is not valid JSON.
    InvalidJson(String),
    /// A required protocol field is missing.
    MissingField(String),
    /// `metadata.message_type` is not a known EventSub WS message type.
    UnknownMessageType(String),
    /// A notification's subscription type is not an ingestable alert kind.
    UnsupportedSubscriptionType(String),
}

impl fmt::Display for EventsubWsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(detail) => {
                write!(f, "EventSub WebSocket frame is not valid JSON: {detail}")
            }
            Self::MissingField(field) => {
                write!(f, "EventSub WebSocket frame is missing `{field}`")
            }
            Self::UnknownMessageType(kind) => {
                write!(
                    f,
                    "EventSub WebSocket message type is not supported: {kind}"
                )
            }
            Self::UnsupportedSubscriptionType(kind) => {
                write!(f, "EventSub notification type is not ingestable: {kind}")
            }
        }
    }
}

impl std::error::Error for EventsubWsError {}

/// One parsed EventSub WebSocket protocol message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventsubWsMessage {
    /// `session_welcome` — the session is connected and carries a session ID
    /// the subscriptions must be (re)created against.
    Welcome {
        session_id: String,
        keepalive_timeout_seconds: u64,
        reconnect_url: Option<String>,
    },
    /// `session_keepalive` — application-level heartbeat resetting the
    /// keepalive clock (Twitch also sends ws-level PING which tungstenite
    /// answers automatically).
    Keepalive,
    /// `session_reconnect` — the server asks the client to reconnect to the
    /// given URL (the old session is dropped after reconnecting).
    Reconnect { reconnect_url: String },
    /// `revocation` — Twitch revoked one of our subscriptions (scope or
    /// permission change); recorded through counters.
    Revoked { subscription_type: String },
    /// `notification` — one of our subscriptions fired; parsed into an
    /// [`AlertEvent`] via the existing webhook parser (the WS `payload`
    /// object reuses the `subscription` + `event` shape).
    Notification(AlertEvent),
}

/// Parse one raw EventSub WebSocket text frame into a protocol message.
///
/// Network-free and fully deterministic; every message type is pinned against
/// realistic Twitch payload vectors in the test suite.
pub fn parse_eventsub_ws_message(json: &str) -> Result<EventsubWsMessage, EventsubWsError> {
    let value: serde_json::Value = serde_json::from_str(json.trim())
        .map_err(|e| EventsubWsError::InvalidJson(e.to_string()))?;
    let message_type = value
        .get("metadata")
        .and_then(|m| m.get("message_type"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| EventsubWsError::MissingField("metadata.message_type".into()))?;
    let payload = value.get("payload");
    match message_type {
        "session_welcome" => {
            let session = payload
                .and_then(|p| p.get("session"))
                .ok_or_else(|| EventsubWsError::MissingField("payload.session".into()))?;
            let session_id = session
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| EventsubWsError::MissingField("payload.session.id".into()))?
                .to_owned();
            let keepalive_timeout_seconds = session
                .get("keepalive_timeout_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(10)
                .max(1);
            let reconnect_url = session.get("reconnect_url").and_then(|v| v.as_str());
            Ok(EventsubWsMessage::Welcome {
                session_id,
                keepalive_timeout_seconds,
                reconnect_url: reconnect_url.map(str::to_owned),
            })
        }
        "session_keepalive" => Ok(EventsubWsMessage::Keepalive),
        "session_reconnect" => {
            let reconnect_url = payload
                .and_then(|p| p.get("session"))
                .and_then(|s| s.get("reconnect_url"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    EventsubWsError::MissingField("payload.session.reconnect_url".into())
                })?
                .to_owned();
            Ok(EventsubWsMessage::Reconnect { reconnect_url })
        }
        "revocation" => {
            let subscription_type = payload
                .and_then(|p| p.get("subscription"))
                .and_then(|s| s.get("type"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| EventsubWsError::MissingField("payload.subscription.type".into()))?
                .to_owned();
            Ok(EventsubWsMessage::Revoked { subscription_type })
        }
        "notification" => match parse_twitch_eventsub_notification(
            &payload
                .ok_or_else(|| EventsubWsError::MissingField("payload".into()))?
                .to_string(),
        ) {
            Ok(event) => Ok(EventsubWsMessage::Notification(event)),
            Err(AlertIngestError::InvalidJson(detail)) => Err(EventsubWsError::InvalidJson(detail)),
            Err(AlertIngestError::UnsupportedKind(kind)) => {
                Err(EventsubWsError::UnsupportedSubscriptionType(kind))
            }
            Err(AlertIngestError::MissingField(field)) => Err(EventsubWsError::MissingField(field)),
            Err(AlertIngestError::SignatureMismatch) => {
                Err(EventsubWsError::MissingField("signature".into()))
            }
        },
        other => Err(EventsubWsError::UnknownMessageType(other.to_owned())),
    }
}

/// Build the Helix subscription-create body for one alert subscription.
/// Deterministic shape: `type` + `version` + `condition.broadcaster_user_id` +
/// `transport` (websocket, bound to the active `session_id`).
pub fn build_subscription_body(
    subscription_type: &str,
    version: &str,
    broadcaster_id: &str,
    session_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "type": subscription_type,
        "version": version,
        "condition": { "broadcaster_user_id": broadcaster_id },
        "transport": { "method": "websocket", "session_id": session_id },
    })
}

/// Shared worker counters (diagnostics only; never contents or credentials).
#[derive(Default)]
struct WsCounters {
    delivered: AtomicU64,
    rejected: AtomicU64,
    subscription_created: AtomicU64,
    subscription_error: AtomicU64,
    revocation: AtomicU64,
    reconnect: AtomicU64,
}

impl WsCounters {
    fn snapshot(&self) -> (u64, u64, u64, u64, u64, u64) {
        (
            self.delivered.load(Ordering::Relaxed),
            self.rejected.load(Ordering::Relaxed),
            self.subscription_created.load(Ordering::Relaxed),
            self.subscription_error.load(Ordering::Relaxed),
            self.revocation.load(Ordering::Relaxed),
            self.reconnect.load(Ordering::Relaxed),
        )
    }
}

/// A running outbound EventSub WebSocket transport. Drop or call
/// [`EventsubReceiver::shutdown`] to stop the worker and close the socket.
pub struct EventsubReceiver {
    config: EventsubWsConfig,
    stop: Arc<AtomicBool>,
    conn: Arc<AtomicU8>,
    counters: Arc<WsCounters>,
    events: Receiver<AlertEvent>,
    thread: Option<thread::JoinHandle<()>>,
}

impl fmt::Debug for EventsubReceiver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Privacy: connection state and counters only — never the token,
        // never the client ID, never event contents.
        let (delivered, rejected, subscription_created, subscription_error, revocation, reconnect) =
            self.counters.snapshot();
        f.debug_struct("EventsubReceiver")
            .field("connected", &self.connected())
            .field(
                "credentials_configured",
                &(!self.config.client_id.is_empty()
                    && !self.config.token.is_empty()
                    && !self.config.broadcaster_id.is_empty()),
            )
            .field("delivered", &delivered)
            .field("rejected", &rejected)
            .field("subscription_created", &subscription_created)
            .field("subscription_error", &subscription_error)
            .field("revocation", &revocation)
            .field("reconnect", &reconnect)
            .finish()
    }
}

impl EventsubReceiver {
    /// Start the EventSub worker thread (connect failures are asynchronous
    /// and surfaced through counters/`connected()` plus reconnect backoff).
    /// Missing credentials never dial out: the worker stays off.
    pub fn start(config: EventsubWsConfig) -> EventsubReceiver {
        let (events_tx, events_rx) =
            crossbeam_channel::bounded::<AlertEvent>(EVENT_CHANNEL_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let conn = Arc::new(AtomicU8::new(0));
        let counters = Arc::new(WsCounters::default());
        let thread = thread::Builder::new()
            .name("alerts-eventsub".into())
            .spawn({
                let stop = stop.clone();
                let conn = conn.clone();
                let counters = counters.clone();
                let worker_config = config.clone();
                move || worker_loop(worker_config, stop, conn, counters, events_tx)
            })
            .ok();
        EventsubReceiver {
            config,
            stop,
            conn,
            counters,
            events: events_rx,
            thread,
        }
    }

    /// Events parsed since start, drained by the GUI into the alert queue.
    pub fn events(&self) -> &Receiver<AlertEvent> {
        &self.events
    }

    /// Whether a session is currently established (`true` between the Moment a
    /// socket connects and the session ends or the transport stops).
    pub fn connected(&self) -> bool {
        self.conn.load(Ordering::SeqCst) == 1
    }

    /// Diagnostics counters: `(delivered, rejected, subscription_created,
    /// subscription_error, revocation, reconnect)`.
    pub fn stats(&self) -> (u64, u64, u64, u64, u64, u64) {
        self.counters.snapshot()
    }

    /// Stop the worker and join its thread (safe to call repeatedly; the
    /// 500 ms socket read timeout keeps the join prompt).
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for EventsubReceiver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// What a session run ended with; the worker decides reconnection policy.
enum SessionOutcome {
    /// Clean server-side close or explicit stop: reconnect with backoff.
    Disconnected,
    /// Server-requested reconnect to the given URL: reconnect promptly.
    Reconnect(String),
    /// Socket/protocol failure: reconnect with backoff.
    Failed(anyhow::Error),
}

fn worker_loop(
    cfg: EventsubWsConfig,
    stop: Arc<AtomicBool>,
    conn: Arc<AtomicU8>,
    counters: Arc<WsCounters>,
    events_tx: Sender<AlertEvent>,
) {
    if cfg.client_id.is_empty() || cfg.token.is_empty() || cfg.broadcaster_id.is_empty() {
        // Missing credentials: nothing to dial. The GUI gates the toggle on
        // credentials and surface the notes instead.
        conn.store(0, Ordering::SeqCst);
        return;
    }
    let mut endpoint = cfg.ws_endpoint.clone();
    let mut backoff = Duration::from_secs(1);
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        conn.store(1, Ordering::SeqCst);
        match run_session(&cfg, &endpoint, &stop, &counters, &events_tx) {
            SessionOutcome::Disconnected => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                conn.store(2, Ordering::SeqCst);
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
            SessionOutcome::Reconnect(url) => {
                conn.store(2, Ordering::SeqCst);
                counters.reconnect.fetch_add(1, Ordering::Relaxed);
                endpoint = url;
                backoff = Duration::from_secs(1);
                std::thread::sleep(Duration::from_secs(2));
            }
            SessionOutcome::Failed(error) => {
                conn.store(2, Ordering::SeqCst);
                tracing::warn!(
                    error = %error,
                    backoff_secs = backoff.as_secs(),
                    "EventSub WebSocket session failed"
                );
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
}

/// Establish one EventSub WebSocket session and pump messages until the
/// connection ends. Subscriptions are (re)created the first time a session ID
/// is seen. Ping frames are answered automatically by tungstenite.
fn run_session(
    cfg: &EventsubWsConfig,
    endpoint: &str,
    stop: &AtomicBool,
    counters: &Arc<WsCounters>,
    events_tx: &Sender<AlertEvent>,
) -> SessionOutcome {
    let (mut ws, _) = match tungstenite::connect(endpoint) {
        Ok(socket) => socket,
        Err(error) => {
            return SessionOutcome::Failed(anyhow::anyhow!(
                "EventSub WebSocket handshake failed: {error}"
            ));
        }
    };
    if let Err(error) = apply_read_timeout(&ws, SOCKET_READ_TIMEOUT) {
        return SessionOutcome::Failed(anyhow::anyhow!(
            "EventSub socket timeout setup failed: {error}"
        ));
    }
    let mut subscribed_session: Option<String> = None;
    let mut keepalive_timeout = Duration::from_secs(10);
    let mut last_message = Instant::now();
    tracing::info!(endpoint, "EventSub WebSocket connected");

    loop {
        if stop.load(Ordering::SeqCst) {
            return SessionOutcome::Disconnected;
        }
        if last_message.elapsed() > keepalive_timeout.saturating_add(KEEPALIVE_GRACE) {
            counters.reconnect.fetch_add(1, Ordering::Relaxed);
            return SessionOutcome::Failed(anyhow::anyhow!("EventSub keepalive timed out"));
        }
        match ws.read() {
            Ok(tungstenite::Message::Text(text)) => {
                last_message = Instant::now();
                match parse_eventsub_ws_message(text.as_str()) {
                    Ok(EventsubWsMessage::Welcome {
                        session_id,
                        keepalive_timeout_seconds,
                        ..
                    }) => {
                        keepalive_timeout = Duration::from_secs(keepalive_timeout_seconds.max(1));
                        if subscribed_session.as_deref() != Some(session_id.as_str()) {
                            create_alert_subscriptions(cfg, &session_id, counters);
                            subscribed_session = Some(session_id.clone());
                        }
                    }
                    Ok(EventsubWsMessage::Keepalive) => {}
                    Ok(EventsubWsMessage::Reconnect { reconnect_url }) => {
                        return SessionOutcome::Reconnect(reconnect_url);
                    }
                    Ok(EventsubWsMessage::Revoked { subscription_type }) => {
                        counters.revocation.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!(
                            subscription_type,
                            "EventSub subscription revoked by Twitch"
                        );
                    }
                    Ok(EventsubWsMessage::Notification(event)) => {
                        let _ = events_tx.send(event);
                        counters.delivered.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        counters.rejected.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!(error = %error, "EventSub frame rejected");
                    }
                }
            }
            Ok(tungstenite::Message::Close(_)) => return SessionOutcome::Disconnected,
            Ok(tungstenite::Message::Ping(_)) => {}
            Ok(tungstenite::Message::Pong(_)) => {}
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(tungstenite::Error::ConnectionClosed) => return SessionOutcome::Disconnected,
            Err(error) => {
                return SessionOutcome::Failed(anyhow::anyhow!(
                    "EventSub WebSocket error: {error}"
                ));
            }
        }
    }
}

/// Create the four alert subscriptions against the Helix API for the active
/// session. Sequential and blocking by design; failures are counted and
/// logged (never fatal — the next reconnect re-creates them).
fn create_alert_subscriptions(cfg: &EventsubWsConfig, session_id: &str, counters: &WsCounters) {
    let api = format!(
        "{}/eventsub/subscriptions",
        cfg.api_base.trim_end_matches('/')
    );
    for (subscription_type, version) in EVENTSUB_ALERT_SUBSCRIPTIONS {
        let body =
            build_subscription_body(subscription_type, version, &cfg.broadcaster_id, session_id);
        let response = ureq::post(&api)
            .header("Client-Id", cfg.client_id.as_str())
            .header("Authorization", format!("Bearer {}", cfg.token))
            .header("Content-Type", "application/json")
            .send_json(body);
        match response {
            Ok(_) => {
                counters
                    .subscription_created
                    .fetch_add(1, Ordering::Relaxed);
            }
            Err(error) => {
                counters.subscription_error.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    subscription_type,
                    error = %error,
                    "EventSub subscription create failed"
                );
            }
        }
    }
}

/// Put the 500 ms read timeout on the underlying TCP stream so `stop` is seen
/// promptly. `tungstenite::connect` does not expose the socket before the
/// handshake; the timeout therefore lives on the connected stream.
fn apply_read_timeout(
    ws: &WebSocket<MaybeTlsStream<TcpStream>>,
    timeout: Duration,
) -> std::io::Result<()> {
    match ws.get_ref() {
        MaybeTlsStream::Plain(tcp) => tcp.set_read_timeout(Some(timeout)),
        MaybeTlsStream::Rustls(tls) => tls.get_ref().set_read_timeout(Some(timeout)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts_ingest::AlertKind;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration as StdDuration;

    const WELCOME: &str = r#"{
        "metadata": {"message_type": "session_welcome", "message_timestamp": "2026-09-10T00:00:00Z", "subscription_type": null, "subscription_version": null},
        "payload": {"session": {"id": "SES_abc123", "status": "connected", "connected_at": "2026-09-10T00:00:00Z", "keepalive_timeout_seconds": 30, "reconnect_url": null}}
    }"#;

    const KEEPALIVE: &str = r#"{
        "metadata": {"message_type": "session_keepalive", "message_timestamp": "2026-09-10T00:00:10Z", "subscription_type": null, "subscription_version": null},
        "payload": {"session": {"connected_at": "2026-09-10T00:00:00Z", "keepalive_timeout_seconds": 30, "reconnect_url": null}}
    }"#;

    const RECONNECT: &str = r#"{
        "metadata": {"message_type": "session_reconnect", "message_timestamp": "2026-09-10T00:01:00Z", "subscription_type": null, "subscription_version": null},
        "payload": {"session": {"id": "SES_abc123", "status": "connected", "keepalive_timeout_seconds": 30, "reconnect_url": "wss://eventsub.wss.twitch.tv/ws?reconnect=reshmuxz50mfg6zhkw8ui0xr5idumpyb0o8fm55c4tdg7d9gia"}}
    }"#;

    const REVOKE: &str = r#"{
        "metadata": {"message_type": "revocation", "message_timestamp": "2026-09-10T00:02:00Z", "subscription_type": "channel.follow", "subscription_version": "2"},
        "payload": {"subscription": {"id": "sub_1", "type": "channel.follow", "version": "2", "status": "revoked"}}
    }"#;

    const NOTIFICATION: &str = r#"{
        "metadata": {"message_type": "notification", "message_timestamp": "2026-09-10T00:03:00Z", "subscription_type": "channel.follow", "subscription_version": "2"},
        "payload": {
            "subscription": {"id": "sub_1", "type": "channel.follow", "version": "2", "status": "enabled"},
            "event": {"user_id": "456", "user_login": "ada", "user_name": "Ada", "broadcaster_user_id": "123", "followed_at": "2026-09-10T00:03:00Z"}
        }
    }"#;

    #[test]
    fn parses_welcome_session() {
        let message = parse_eventsub_ws_message(WELCOME).expect("parse");
        assert_eq!(
            message,
            EventsubWsMessage::Welcome {
                session_id: "SES_abc123".to_owned(),
                keepalive_timeout_seconds: 30,
                reconnect_url: None,
            }
        );
    }

    #[test]
    fn parses_keepalive() {
        assert_eq!(
            parse_eventsub_ws_message(KEEPALIVE).expect("parse"),
            EventsubWsMessage::Keepalive
        );
    }

    #[test]
    fn parses_reconnect() {
        let message = parse_eventsub_ws_message(RECONNECT).expect("parse");
        assert!(matches!(
            message,
            EventsubWsMessage::Reconnect { reconnect_url }
                if reconnect_url.contains("?reconnect=")
        ));
    }

    #[test]
    fn parses_revocation() {
        assert_eq!(
            parse_eventsub_ws_message(REVOKE).expect("parse"),
            EventsubWsMessage::Revoked {
                subscription_type: "channel.follow".to_owned(),
            }
        );
    }

    #[test]
    fn parses_follow_notification_into_an_alert_event() {
        let message = parse_eventsub_ws_message(NOTIFICATION).expect("parse");
        match message {
            EventsubWsMessage::Notification(event) => {
                assert_eq!(event.kind, AlertKind::Follow);
                assert_eq!(event.user, "Ada");
            }
            other => panic!("expected notification, got {other:?}"),
        }
    }

    #[test]
    fn welcome_defaults_keepalive_to_ten() {
        let without_timeout = WELCOME.replace("\"keepalive_timeout_seconds\": 30, ", "");
        let message = parse_eventsub_ws_message(&without_timeout).expect("parse");
        assert_eq!(
            message,
            EventsubWsMessage::Welcome {
                session_id: "SES_abc123".to_owned(),
                keepalive_timeout_seconds: 10,
                reconnect_url: None,
            }
        );
    }

    #[test]
    fn rejects_malformed_json_and_unknown_types() {
        assert!(matches!(
            parse_eventsub_ws_message("not json"),
            Err(EventsubWsError::InvalidJson(_))
        ));
        assert!(matches!(
            parse_eventsub_ws_message(r#"{"metadata": {"message_type": "mystery"}}"#),
            Err(EventsubWsError::UnknownMessageType(t)) if t == "mystery"
        ));
    }

    #[test]
    fn build_subscription_body_has_requested_shape() {
        let body = build_subscription_body("channel.follow", "2", "123", "SES_abc123");
        assert_eq!(body["type"], "channel.follow");
        assert_eq!(body["version"], "2");
        assert_eq!(body["condition"]["broadcaster_user_id"], "123");
        assert_eq!(body["transport"]["method"], "websocket");
        assert_eq!(body["transport"]["session_id"], "SES_abc123");
    }

    #[test]
    fn config_defaults_point_at_twitch() {
        let config = EventsubWsConfig::default();
        assert_eq!(config.ws_endpoint, DEFAULT_EVENTSUB_WS_ENDPOINT);
        assert_eq!(config.api_base, DEFAULT_TWITCH_API_BASE);
        assert!(config.client_id.is_empty());
        assert!(config.token.is_empty());
    }

    #[test]
    fn debug_never_includes_token_or_client_id() {
        let receiver = EventsubReceiver::start(EventsubWsConfig {
            client_id: "my-client-id".to_owned(),
            token: "super-secret-token".to_owned(),
            broadcaster_id: "123".to_owned(),
            ws_endpoint: "ws://127.0.0.1:9".to_owned(),
            api_base: "http://127.0.0.1:9".to_owned(),
        });
        let debug = format!("{receiver:?}");
        assert!(!debug.contains("my-client-id"), "{debug}");
        assert!(!debug.contains("super-secret-token"), "{debug}");
        assert!(debug.contains("connected: false"), "{debug}");
        drop(receiver);
    }

    /// Read one complete HTTP request from the socket before answering, so the
    /// local stub never drops the connection while the client is still sending
    /// (Windows turns that into `WSAECONNRESET` instead of a clean FIN; same
    /// fixture pattern as the Kick/YouTube worker tests).
    fn drain_http_request(stream: &mut std::net::TcpStream) -> std::io::Result<()> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Ok(());
            }
            buf.extend_from_slice(&chunk[..n]);
        };
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
    fn delivers_notification_from_a_local_websocket_puppet() {
        // Puppet EventSub WebSocket server: accept one client, push a welcome
        // then a follow notification, then close when the client disconnects.
        let (port_tx, port_rx) = crossbeam_channel::bounded::<u16>(1);
        let server = std::thread::Builder::new()
            .name("test-eventsub-puppet".into())
            .spawn(move || {
                let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind puppet");
                let port = listener.local_addr().expect("addr").port();
                let _ = port_tx.try_send(port);
                let (stream, _) = listener.accept().expect("accept");
                let mut ws = tungstenite::accept(stream).expect("ws accept");
                ws.send(tungstenite::Message::Text(WELCOME.into()))
                    .expect("welcome");
                ws.send(tungstenite::Message::Text(NOTIFICATION.into()))
                    .expect("notification");
                // Give the client a moment to consume, then drain until the
                // client disconnects (clean FIN, no RST).
                std::thread::sleep(StdDuration::from_millis(200));
                while ws.read().is_ok() {}
            })
            .expect("spawn puppet");

        let ws_port = port_rx
            .recv_timeout(StdDuration::from_secs(5))
            .expect("ws port");

        // Local pseudo-Helix stub answering 202 for the subscription POSTs.
        let (api_port_tx, api_port_rx) = crossbeam_channel::bounded::<u16>(1);
        let api_stub = std::thread::Builder::new()
            .name("test-eventsub-api".into())
            .spawn(move || {
                let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind api stub");
                let port = listener.local_addr().expect("addr").port();
                let _ = api_port_tx.try_send(port);
                listener.set_nonblocking(true).expect("nonblocking");
                let deadline = std::time::Instant::now() + StdDuration::from_secs(5);
                let mut handled = 0;
                while std::time::Instant::now() < deadline {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = drain_http_request(&mut stream);
                            let _ = stream
                                .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 2\r\n\r\n{}");
                            handled += 1;
                        }
                        Err(_) => {
                            std::thread::sleep(StdDuration::from_millis(10));
                        }
                    }
                }
                assert!(
                    handled >= EVENTSUB_ALERT_SUBSCRIPTIONS.len(),
                    "stub handled {handled}"
                );
            })
            .expect("spawn api stub");

        let api_port = api_port_rx
            .recv_timeout(StdDuration::from_secs(5))
            .expect("api port");

        let receiver = EventsubReceiver::start(EventsubWsConfig {
            client_id: "test-client".to_owned(),
            token: "test-token".to_owned(),
            broadcaster_id: "123".to_owned(),
            ws_endpoint: format!("ws://127.0.0.1:{ws_port}"),
            api_base: format!("http://127.0.0.1:{api_port}"),
        });

        let event = receiver
            .events()
            .recv_timeout(StdDuration::from_secs(10))
            .expect("event queued");
        assert_eq!(event.kind, AlertKind::Follow);
        assert_eq!(event.user, "Ada");
        let (delivered, rejected, created, error, _, _) = receiver.stats();
        assert_eq!(delivered, 1);
        assert_eq!(rejected, 0);
        assert_eq!(created, EVENTSUB_ALERT_SUBSCRIPTIONS.len() as u64);
        assert_eq!(error, 0);

        drop(receiver);
        let _ = server.join();
        let _ = api_stub.join();
    }

    #[test]
    fn missing_credentials_never_dial_out() {
        let receiver = EventsubReceiver::start(EventsubWsConfig::default());
        std::thread::sleep(StdDuration::from_millis(50));
        assert!(!receiver.connected());
        assert_eq!(receiver.stats().0, 0);
        drop(receiver);
    }
}
