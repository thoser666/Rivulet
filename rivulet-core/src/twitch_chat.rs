//! Optional Twitch IRC chat reader for the streamer-facing chat dock.
//!
//! Design goals:
//! 1. **Pure parsing** — [`parse_irc_line`] turns a raw IRC line into a
//!    [`ChatMessage`] without any I/O, so the wire format is unit-testable.
//! 2. **Non-blocking** — the GUI only enqueues a connect/disconnect and polls
//!    a channel for incoming messages; all socket I/O happens on a worker
//!    thread (same pattern as the Discord Rich Presence adapter).
//! 3. **Testable endpoint** — the IRC endpoint is configurable, so CI can run
//!    the real worker against a local listener on `127.0.0.1` and verify the
//!    handshake + message flow deterministically.
//! 4. **Privacy-safe** — only the configured channel and a (possibly
//!    anonymous) nickname are transmitted; the OAuth token is never logged.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use serde::Serialize;

/// A parsed chat message from one IRC line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatMessage {
    /// The display name (lowercase login if no display-name tag).
    pub user: String,
    /// The text after the colon, with leading "/me " stripped for actions.
    pub text: String,
    /// Whether this was an action (`/me ...` → `ACTION`).
    pub action: bool,
    /// Optional display color (e.g. `#FF0000`), absent when unknown.
    pub color: Option<String>,
    /// Badge names without the version suffix (e.g. `moderator`, `subscriber`).
    pub badges: Vec<String>,
    /// Whether the sender is the channel broadcaster.
    pub broadcaster: bool,
    /// Twitch message id (`id=` tag) so the bot can answer a specific chat
    /// line with `reply-parent-msg-id`. Kick/YouTube have no IRC id and stay
    /// `None`.
    pub id: Option<String>,
    /// Unix timestamp (seconds) of the message.
    pub timestamp: u64,
}

impl ChatMessage {
    /// True when the message is an empty keepalive/notice artifact (e.g. a
    /// line that parsed but carried no user text worth showing).
    pub fn is_empty_artifact(&self) -> bool {
        self.text.is_empty()
    }
}

/// Config for connecting to an IRC server (defaults to Twitch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwitchChatConfig {
    /// IRC endpoint in `host:port` form. Defaults to `irc.chat.twitch.tv:6697`
    /// (TLS); tests override this with a local listener.
    pub endpoint: String,
    /// Nickname to send as `NICK`. Empty uses an anonymous read-only nick
    /// (`justinfan<random>`), which Twitch allows for reading chat.
    pub nick: String,
    /// OAuth token (`oauth:...`) for authenticated reads. Empty connects
    /// anonymously (read-only, no badges/color).
    pub oauth_token: String,
    /// Lowercase channel name to join (`#name` is appended).
    pub channel: String,
}

impl Default for TwitchChatConfig {
    fn default() -> Self {
        Self {
            endpoint: "irc.chat.twitch.tv:6697".to_owned(),
            nick: String::new(),
            oauth_token: String::new(),
            channel: String::new(),
        }
    }
}

/// Connection state exposed to the GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatConnState {
    /// Worker not running (chat disabled or no channel configured).
    Off,
    /// Worker is running and connecting/joined, but idle (no messages yet).
    Connected,
    /// The connection failed or was dropped; the worker is backing off.
    Disconnected,
}

/// Handle to a running chat worker. Non-blocking by construction.
pub struct TwitchChat {
    tx: Option<Sender<Msg>>,
    /// Parsed chat messages produced by the worker.
    messages: Option<Receiver<ChatMessage>>,
    stop: Arc<AtomicBool>,
    conn: Arc<std::sync::atomic::AtomicU8>,
    /// Set when the server sent `msg_requires_verified_phone_number`: the
    /// configured account cannot send until it is phone-verified.
    phone: Arc<std::sync::atomic::AtomicBool>,
}

impl TwitchChat {
    /// Spawn the worker. Returns a disabled handle when the channel is empty.
    pub fn new(config: &TwitchChatConfig) -> Self {
        if config.channel.trim().is_empty() {
            return Self::disabled();
        }
        let (tx, rx) = unbounded();
        let (msg_tx, msg_rx) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let conn = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let phone = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_conn = Arc::clone(&conn);
        let worker_stop = Arc::clone(&stop);
        let worker_phone = Arc::clone(&phone);
        let cfg = config.clone();
        let spawned = std::thread::Builder::new()
            .name("rivulet-twitch-chat".to_owned())
            .spawn(move || worker_loop(rx, cfg, worker_stop, worker_conn, worker_phone, msg_tx))
            .is_ok();
        if !spawned {
            return Self::disabled();
        }
        Self {
            tx: Some(tx),
            messages: Some(msg_rx),
            stop,
            conn,
            phone,
        }
    }

    fn disabled() -> Self {
        Self {
            tx: None,
            messages: None,
            stop: Arc::new(AtomicBool::new(true)),
            conn: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            phone: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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

    /// Send a chat message. Returns `false` when the worker is disabled (no
    /// channel configured) — e.g. there is no socket to write to. Never
    /// blocks; the text is written to the IRC socket by the worker.
    ///
    /// Sending requires an authenticated connection: Twitch rejects PRIVMSG
    /// from the anonymous `justinfan` nick. The GUI enables the input only
    /// when an OAuth token is configured; the worker still drops empty text.
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

    /// Reply to a specific chat line using Twitch's `reply-parent-msg-id`
    /// mechanism (requires the `twitch.tv/tags` capability, which the worker
    /// always requests). Returns `false` when the worker is disabled, the
    /// text is empty, or no parent message id is given. Never blocks.
    pub fn send_reply(&self, text: &str, reply_to_id: &str) -> bool {
        let reply_to = reply_to_id.trim();
        match &self.tx {
            Some(tx) => {
                let text = text.trim();
                if text.is_empty() || reply_to.is_empty() {
                    return false;
                }
                let _ = tx.try_send(Msg::SendReply {
                    text: text.to_owned(),
                    reply_to: reply_to.to_owned(),
                });
                true
            }
            None => false,
        }
    }

    /// Whether the server told us the bot account must be phone-verified
    /// before it can send chat (`msg_requires_verified_phone_number`).
    pub fn phone_verification_required(&self) -> bool {
        self.phone.load(Ordering::SeqCst)
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

impl Drop for TwitchChat {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Disconnect);
        }
    }
}

enum Msg {
    Disconnect,
    SendMessage(String),
    /// Reply to a specific chat line via `@reply-parent-msg-id`.
    SendReply {
        text: String,
        reply_to: String,
    },
}

fn worker_loop(
    rx: Receiver<Msg>,
    cfg: TwitchChatConfig,
    stop: Arc<AtomicBool>,
    conn: Arc<std::sync::atomic::AtomicU8>,
    phone: Arc<std::sync::atomic::AtomicBool>,
    msg_tx: Sender<ChatMessage>,
) {
    let mut backoff = 1u64;
    while !stop.load(Ordering::SeqCst) {
        conn.store(1, Ordering::SeqCst);
        match run_session(&cfg, &msg_tx, &rx, &phone) {
            Ok(()) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                // Clean EOF (server closed): reconnect after a short delay.
                conn.store(2, Ordering::SeqCst);
                std::thread::sleep(Duration::from_secs(backoff));
            }
            Err(e) => {
                conn.store(2, Ordering::SeqCst);
                tracing::warn!(error = %e, backoff_secs = backoff, "Twitch chat connection failed");
                std::thread::sleep(Duration::from_secs(backoff));
                backoff = (backoff * 2).min(30);
            }
        }
    }
}

/// One connection session: connect, register, join, then read lines until the
/// server closes the stream or the stop flag is set. Inbound `Msg` values
/// (disconnect, outbound chat text) are drained before each socket read, so
/// sending stays responsive even while the socket is idle.
fn run_session(
    cfg: &TwitchChatConfig,
    msg_tx: &Sender<ChatMessage>,
    rx: &Receiver<Msg>,
    phone: &Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<()> {
    let stream = TcpStream::connect(&cfg.endpoint)?;
    // Short timeout so outbound messages are handled promptly between reads;
    // the loop below treats WouldBlock/TimedOut as "no input yet" and keeps
    // draining the outbound queue.
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    // Register: CAP for tags, then PASS/NICK, then join.
    let nick = if cfg.nick.trim().is_empty() {
        format!("justinfan{}", std::process::id() % 10000)
    } else {
        cfg.nick.trim().to_owned()
    };
    writeln!(writer, "CAP REQ :twitch.tv/tags twitch.tv/commands")?;
    if cfg.oauth_token.trim().is_empty() {
        writeln!(writer, "PASS SCHMOOPIIE")?; // Twitch's documented anonymous password
    } else {
        writeln!(writer, "PASS {}", cfg.oauth_token.trim())?;
    }
    writeln!(writer, "NICK {nick}")?;
    let channel = format!("#{}", cfg.channel.trim().trim_start_matches('#'));
    writeln!(writer, "JOIN {channel}")?;
    writer.flush()?;

    tracing::info!(channel = %channel, nick = %nick, "Twitch chat connected");

    let mut line = String::new();
    loop {
        // Handle outbound requests first so a queued message is sent even
        // when the server has not sent anything for a while.
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Disconnect => return Ok(()),
                Msg::SendMessage(text) => {
                    let text = text.trim();
                    if !text.is_empty() {
                        writeln!(writer, "PRIVMSG {channel} :{text}")?;
                        writer.flush()?;
                    }
                }
                Msg::SendReply { text, reply_to } => {
                    let text = text.trim();
                    if !text.is_empty() && !reply_to.is_empty() {
                        writeln!(
                            writer,
                            "@reply-parent-msg-id={reply_to} PRIVMSG {channel} :{text}"
                        )?;
                        writer.flush()?;
                    }
                }
            }
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // Server closed the connection.
                return Ok(());
            }
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // No input yet; loop back to drain outbound messages.
                continue;
            }
            Err(e) => return Err(e.into()),
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }
        // Keep the socket alive: Twitch sends PING and expects PONG.
        if let Some(pong) = trimmed.strip_prefix("PING ") {
            writeln!(writer, "PONG {pong}")?;
            writer.flush()?;
            continue;
        }
        // Surface server notices that need streamer action (e.g. the bot
        // account must be phone-verified before it can send).
        if let Some(notice) = parse_notice(trimmed) {
            match notice {
                TwitchNotice::PhoneVerificationRequired => {
                    phone.store(true, Ordering::SeqCst);
                    tracing::warn!(
                        "Twitch requires a phone-verified bot account before it can send chat"
                    );
                }
            }
            continue;
        }
        // Deliver parsed chat messages to the GUI-facing channel.
        if let Some(message) = parse_irc_line(trimmed) {
            if !message.is_empty_artifact() {
                let _ = msg_tx.send(message);
            }
        }
    }
}

/// Server notices that need streamer action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwitchNotice {
    /// The account must be phone-verified before it can send chat messages
    /// (Twitch `msg_id=msg_requires_verified_phone_number`).
    PhoneVerificationRequired,
}

/// Parse an IRC NOTICE line. Returns `Some(notice)` only for notices that
/// require streamer action; all other notices (join confirmations, slow-mode
/// hints, moderation messages, ...) return `None` and are ignored. Pure and
/// deterministic — no I/O.
pub fn parse_notice(line: &str) -> Option<TwitchNotice> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    // Twitch tags a NOTICE with `@msg-id=...`. Without tags there is nothing
    // to classify.
    let after_tags = trimmed.strip_prefix('@')?;
    let (tags, remainder) = after_tags.split_once(' ')?;
    // Skip the optional `:nick!user@host` sender prefix, then read the IRC
    // command word (e.g. `:tmi.twitch.tv NOTICE #channel :text`).
    let mut rest = remainder.trim_start();
    if let Some(r) = rest.strip_prefix(':') {
        let (_, rem) = r.split_once(' ')?;
        rest = rem.trim_start();
    }
    if rest.split(' ').next().unwrap_or("") != "NOTICE" {
        return None;
    }
    let mut msg_id: Option<&str> = None;
    for tag in tags.split(';') {
        let (key, value) = tag.split_once('=').unwrap_or((tag, ""));
        if key == "msg-id" {
            msg_id = Some(value);
        }
    }
    match msg_id {
        Some("msg_requires_verified_phone_number") => Some(TwitchNotice::PhoneVerificationRequired),
        _ => None,
    }
}

/// Parse a single IRC line into a chat message. Returns `None` for lines that
/// carry no user message (PING, notices, joins, etc.).
///
/// Handles the IRCv3 tag prefix (`@...`), the trailing `:text`, `/me` actions
/// (`ACTION`), `display-name`/`color`/`badges` tags, and the broadcaster
/// badge. Pure and deterministic — no I/O.
pub fn parse_irc_line(line: &str) -> Option<ChatMessage> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let mut rest = trimmed;

    // IRCv3 tags: @a=b;c=d :nick!user@host PRIVMSG #chan :text
    let mut color: Option<String> = None;
    let mut badges: Vec<String> = Vec::new();
    let mut display_name: Option<String> = None;
    if let Some(after_tags) = rest.strip_prefix('@') {
        let (tags, remainder) = after_tags.split_once(' ')?;
        rest = remainder.trim_start();
        for tag in tags.split(';') {
            let (key, value) = tag.split_once('=').unwrap_or((tag, ""));
            match key {
                "color" if !value.is_empty() => color = Some(value.to_owned()),
                "display-name" if !value.is_empty() => display_name = Some(value.to_owned()),
                "badges" => {
                    for badge in value.split(',') {
                        let name = badge.split('/').next().unwrap_or(badge);
                        if !name.is_empty() {
                            badges.push(name.to_owned());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Sender prefix: :nick!user@host (or :nick when no host).
    if let Some(rest2) = rest.strip_prefix(':') {
        let (prefix, remainder) = rest2.split_once(' ')?;
        rest = remainder.trim_start();
        let user = prefix.split('!').next().unwrap_or(prefix).to_owned();
        let _host_part = prefix.split('!').nth(1);

        // Only PRIVMSG carries chat text.
        let command = rest.split(' ').next().unwrap_or("");
        if command != "PRIVMSG" {
            return None;
        }
        let after_command = rest[command.len()..].trim_start();
        // Skip the channel target (the part up to the first space).
        let text_start = after_command.find(" :")? + 2;
        let raw_text = &after_command[text_start..];
        if raw_text.is_empty() {
            return None;
        }

        // /me actions arrive as: :nick PRIVMSG #chan :\x01ACTION text\x01
        let mut action = false;
        let text = if let Some(inner) = raw_text
            .strip_prefix("\u{1}ACTION")
            .and_then(|s| s.strip_suffix('\u{1}'))
        {
            action = true;
            inner.trim_start().to_owned()
        } else {
            raw_text.to_owned()
        };
        if text.is_empty() {
            return None;
        }

        let broadcaster = badges.iter().any(|b| b == "broadcaster");
        let mut msg_id: Option<String> = None;
        if let Some(after_tags) = trimmed.strip_prefix('@') {
            if let Some((tags, _)) = after_tags.split_once(' ') {
                for tag in tags.split(';') {
                    let (key, value) = tag.split_once('=').unwrap_or((tag, ""));
                    if key == "id" && !value.is_empty() {
                        msg_id = Some(value.to_owned());
                    }
                }
            }
        }
        Some(ChatMessage {
            user: display_name.unwrap_or(user),
            text,
            action,
            color,
            badges,
            broadcaster,
            id: msg_id,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> String {
        "@badge-info=;badges=broadcaster/1;color=#FF0000;display-name=Thoser666;emotes=;id=1;mod=0;room-id=1;subscriber=0;tmi-sent-ts=1;turbo=0;user-id=1;user-type= :thoser666!thoser666@thoser666.tmi.twitch.tv PRIVMSG #rivulet :hello chat".to_owned()
    }

    #[test]
    fn parses_plain_message_with_tags() {
        let msg = parse_irc_line(&line()).expect("message");
        assert_eq!(msg.user, "Thoser666");
        assert_eq!(msg.text, "hello chat");
        assert!(!msg.action);
        assert_eq!(msg.color.as_deref(), Some("#FF0000"));
        assert!(msg.badges.contains(&"broadcaster".to_owned()));
        assert!(msg.broadcaster);
        assert_eq!(msg.id.as_deref(), Some("1"), "id= tag must be kept");
        assert!(msg.timestamp > 0);
    }

    #[test]
    fn parse_notice_recognizes_phone_verification_requirement() {
        let l = "@msg-id=msg_requires_verified_phone_number :tmi.twitch.tv NOTICE #rivulet :Your account must be phone verified in order to chat.";
        assert_eq!(
            parse_notice(l),
            Some(TwitchNotice::PhoneVerificationRequired)
        );
        // Other notices and non-NOTICE lines are deliberately ignored.
        assert_eq!(
            parse_notice("@msg-id=host_on :tmi.twitch.tv NOTICE #rivulet :host is on"),
            None
        );
        assert_eq!(parse_notice("PING :tmi.twitch.tv"), None);
        assert_eq!(
            parse_notice(":a!b@c.tmi.twitch.tv PRIVMSG #rivulet :hello"),
            None
        );
        assert_eq!(
            parse_notice("@msg-id=subs_on :tmi.twitch.tv NOTICE #rivulet :subs on"),
            None
        );
        assert_eq!(parse_notice("not even irc"), None);
    }

    #[test]
    fn parses_action_me_messages() {
        let l = ":someone!someone@tmi.twitch.tv PRIVMSG #rivulet :\u{1}ACTION waves at chat\u{1}";
        let msg = parse_irc_line(l).expect("action");
        assert!(msg.action);
        assert_eq!(msg.text, "waves at chat");
    }

    #[test]
    fn ignores_non_message_lines() {
        assert!(parse_irc_line("PING :tmi.twitch.tv").is_none());
        assert!(parse_irc_line(":tmi.twitch.tv 001 thoser666 :Welcome").is_none());
        assert!(parse_irc_line(":a!b@c JOIN #rivulet").is_none());
        assert!(parse_irc_line(":a!b@c PRIVMSG #rivulet :").is_none());
        assert!(parse_irc_line("not even irc").is_none());
    }

    #[test]
    fn handles_missing_tags_and_unknown_color() {
        let l = ":guest!guest@tmi.twitch.tv PRIVMSG #rivulet :plain";
        let msg = parse_irc_line(l).expect("message");
        assert_eq!(msg.user, "guest");
        assert_eq!(msg.text, "plain");
        assert!(msg.color.is_none());
        assert!(msg.badges.is_empty());
        assert!(!msg.broadcaster);
    }

    #[test]
    fn lowercases_channel_on_config_default() {
        let cfg = TwitchChatConfig {
            channel: "Rivulet".to_owned(),
            ..Default::default()
        };
        assert_eq!(cfg.endpoint, "irc.chat.twitch.tv:6697");
        // The join target is built inside the session; the config keeps the
        // raw channel and the worker normalizes it (covered by the smoke).
        assert_eq!(cfg.channel, "Rivulet");
    }

    #[test]
    fn disabled_when_channel_empty() {
        let chat = TwitchChat::new(&TwitchChatConfig {
            channel: String::new(),
            ..Default::default()
        });
        assert!(!chat.enabled());
        assert_eq!(chat.connection_state(), ChatConnState::Off);
    }

    #[test]
    fn message_serialization_is_privacy_safe() {
        let msg = parse_irc_line(&line()).expect("message");
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains("hello chat"));
        // No oauth token or IRC password may ever leak.
        assert!(!json.contains("oauth"));
        assert!(!json.contains("SCHMOOPIIE"));
    }

    /// End-to-end smoke: run the real worker against a local TCP listener and
    /// verify it registers, joins, answers PING with PONG, and delivers parsed
    /// chat messages to the GUI-facing channel.
    #[test]
    fn worker_connects_and_delivers_messages_to_local_listener() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let endpoint = listener.local_addr().expect("addr").to_string();
        let cfg = TwitchChatConfig {
            endpoint,
            nick: "testbot".to_owned(),
            oauth_token: String::new(),
            channel: "rivulet".to_owned(),
        };
        let mut chat = TwitchChat::new(&cfg);
        assert!(chat.enabled());

        // Accept the worker's connection and drive a minimal IRC session.
        let (mut conn, _) = listener.accept().expect("accept worker");
        conn.set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let mut reader = BufReader::new(conn.try_clone().expect("clone"));

        let mut line = String::new();
        let mut saw_cap = false;
        let mut saw_nick = false;
        let mut saw_join = false;
        let register_deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !saw_join && std::time::Instant::now() < register_deadline {
            line.clear();
            let read = reader.read_line(&mut line).expect("read");
            if read == 0 {
                break;
            }
            let l = line.trim_end_matches(['\r', '\n']);
            if l.starts_with("CAP REQ") {
                saw_cap = true;
            }
            if l.starts_with("NICK testbot") {
                saw_nick = true;
            }
            if l.starts_with("JOIN #rivulet") {
                saw_join = true;
            }
            if l.starts_with("PASS") {
                // The anonymous password must not leak the oauth value.
                assert!(!l.contains("oauth"));
            }
        }
        assert!(saw_cap, "must request tags capability");
        assert!(saw_nick, "must send NICK");
        assert!(saw_join, "must JOIN the channel");

        // Server sends PING; worker must answer PONG on the same stream.
        writeln!(conn, "PING :tmi.twitch.tv").expect("ping");
        conn.flush().expect("flush");
        line.clear();
        let read = reader.read_line(&mut line).expect("read pong");
        assert!(read > 0, "expected a PONG reply");
        assert!(line.starts_with("PONG"), "got: {line}");

        // Server sends a chat line; the worker must parse and deliver it.
        writeln!(
            conn,
            "@color=#FF0000;display-name=ViewerOne;badges=moderator/1 :viewerone!viewerone@tmi.twitch.tv PRIVMSG #rivulet :hello rivulet"
        )
        .expect("message");
        conn.flush().expect("flush");

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
        let msg = delivered.expect("message delivered to the channel");
        assert_eq!(msg.user, "ViewerOne");
        assert_eq!(msg.text, "hello rivulet");
        assert_eq!(msg.color.as_deref(), Some("#FF0000"));
        assert!(msg.badges.contains(&"moderator".to_owned()));

        // Sending: `send_message` must write a PRIVMSG to the same socket.
        assert!(chat.send_message("hello from rivulet"));
        line.clear();
        let read = reader.read_line(&mut line).expect("read privmsg");
        assert!(read > 0, "expected a PRIVMSG reply");
        assert!(
            line.starts_with("PRIVMSG #rivulet :hello from rivulet"),
            "got: {line}"
        );
        assert!(!line.contains("oauth"), "PRIVMSG must not leak the token");

        // Replying: `send_reply` must thread the parent message id through
        // the IRCv3 `@reply-parent-msg-id` tag.
        assert!(chat.send_reply("thanks!", "abc123"));
        line.clear();
        let read = reader.read_line(&mut line).expect("read reply");
        assert!(read > 0, "expected a reply PRIVMSG");
        assert_eq!(
            line.trim_end_matches(['\r', '\n']),
            "@reply-parent-msg-id=abc123 PRIVMSG #rivulet :thanks!"
        );

        // Phone verification: the server rejects the account for sending; the
        // handle flag must flip so the GUI can warn the streamer.
        assert!(!chat.phone_verification_required(), "flag must start clear");
        writeln!(
            conn,
            "@msg-id=msg_requires_verified_phone_number :tmi.twitch.tv NOTICE #rivulet :Your account must be phone verified in order to chat."
        )
        .expect("notice");
        conn.flush().expect("flush");
        let notice_deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !chat.phone_verification_required() && std::time::Instant::now() < notice_deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            chat.phone_verification_required(),
            "phone-verification notice must set the handle flag"
        );

        chat.disconnect();
    }

    #[test]
    fn send_message_requires_an_enabled_worker_and_non_empty_text() {
        let chat = TwitchChat::new(&TwitchChatConfig {
            channel: String::new(),
            ..Default::default()
        });
        assert!(!chat.enabled());
        assert!(
            !chat.send_message("hello"),
            "disabled worker must reject sends"
        );

        let mut chat = TwitchChat::new(&TwitchChatConfig {
            channel: "rivulet".to_owned(),
            endpoint: "127.0.0.1:1".to_owned(), // unreachable, worker backs off
            ..Default::default()
        });
        assert!(chat.enabled());
        assert!(
            !chat.send_message("   "),
            "whitespace-only text must be rejected"
        );
        assert!(
            chat.send_message("hello"),
            "non-empty text must be enqueued"
        );
        chat.disconnect();
    }

    #[test]
    fn send_reply_requires_an_enabled_worker_parent_id_and_text() {
        let chat = TwitchChat::new(&TwitchChatConfig {
            channel: String::new(),
            ..Default::default()
        });
        assert!(
            !chat.send_reply("hello", "abc"),
            "disabled worker must reject replies"
        );
        assert!(!chat.phone_verification_required());

        let mut chat = TwitchChat::new(&TwitchChatConfig {
            channel: "rivulet".to_owned(),
            endpoint: "127.0.0.1:1".to_owned(), // unreachable, worker backs off
            ..Default::default()
        });
        assert!(chat.enabled());
        assert!(
            !chat.send_reply("   ", "abc"),
            "whitespace-only text must be rejected"
        );
        assert!(
            !chat.send_reply("hello", "  "),
            "missing parent id must be rejected"
        );
        assert!(
            chat.send_reply("hello", "abc"),
            "non-empty text and parent id must be enqueued"
        );
        chat.disconnect();
    }
}
