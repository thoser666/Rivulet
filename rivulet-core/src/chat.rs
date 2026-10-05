//! Unified multi-platform chat facade for the streamer-facing chat dock.
//!
//! The chat dock can connect to Twitch (IRC), Kick (Pusher WebSocket) and
//! YouTube (Innertube polling). Each platform has its own worker (see
//! `twitch_chat.rs`, `kick_chat.rs`, `youtube_chat.rs`) with a deterministic
//! local-listener smoke test; this module exposes one handle so the GUI does
//! not care which protocol is behind the dock.
//!
//! The endpoint fields default to the real public endpoints; tests override
//! them with local listeners.

use std::sync::Mutex;

use crossbeam_channel::Receiver;

use crate::kick_chat::{KickChat, KickChatConfig};
use crate::rate_limit::{RateLimitConfig, RateLimiter};
use crate::twitch_chat::{ChatConnState, ChatMessage, TwitchChat, TwitchChatConfig};
use crate::youtube_chat::{
    YouTubeChat, YouTubeChatConfig, YouTubeQuotaConfig, YouTubeSendCredentials,
};

/// Supported chat platforms of the dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ChatPlatform {
    #[default]
    Twitch,
    Kick,
    YouTube,
}

impl ChatPlatform {
    /// Human-readable platform name (proper noun, no translation needed).
    pub fn label(&self) -> &'static str {
        match self {
            ChatPlatform::Twitch => "Twitch",
            ChatPlatform::Kick => "Kick",
            ChatPlatform::YouTube => "YouTube",
        }
    }

    /// All platforms in dock order.
    pub fn all() -> [ChatPlatform; 3] {
        [
            ChatPlatform::Twitch,
            ChatPlatform::Kick,
            ChatPlatform::YouTube,
        ]
    }
}

/// Config for connecting the chat dock. Endpoints default to the public
/// platform endpoints and are overridable (tests point them at local
/// listeners); the GUI only sets `platform`, `channel` and `token`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatConfig {
    pub platform: ChatPlatform,
    /// Twitch IRC endpoint (`host:port`). Empty → default.
    pub twitch_endpoint: String,
    /// Kick Pusher WebSocket URL. Empty → default.
    pub kick_ws_endpoint: String,
    /// Kick API base. Empty → default.
    pub kick_api_base: String,
    /// YouTube live-chat page URL prefix. Empty → default.
    pub youtube_page_endpoint: String,
    /// YouTube `get_live_chat` poll URL. Empty → default.
    pub youtube_poll_endpoint: String,
    /// YouTube `liveChatMessages.insert` URL. Empty → default.
    pub youtube_send_endpoint: String,
    /// YouTube Data API key for the official send path. Empty → read-only.
    pub youtube_api_key: String,
    /// `liveChatId` of the running YouTube broadcast. Empty → read-only.
    pub youtube_live_chat_id: String,
    /// Channel: Twitch channel / Kick slug / YouTube video id.
    pub channel: String,
    /// Token: Twitch OAuth (`oauth:...`), Kick session token, or the YouTube
    /// OAuth token with the `youtube.force-ssl` scope.
    pub token: String,
    /// Outbound rate limit. `None` → the platform default (Twitch 20/30 s,
    /// Kick 10/30 s, YouTube serialized at ~50/day to match its API quota).
    pub rate_limit: Option<RateLimitConfig>,
    /// Daily YouTube API quota budget. `None` → the documented 200 units per
    /// insert against a 10 000-unit daily project budget.
    pub youtube_quota: Option<YouTubeQuotaConfig>,
}

impl ChatConfig {
    pub fn new(platform: ChatPlatform, channel: String, token: String) -> Self {
        Self {
            platform,
            twitch_endpoint: String::new(),
            kick_ws_endpoint: String::new(),
            kick_api_base: String::new(),
            youtube_page_endpoint: String::new(),
            youtube_poll_endpoint: String::new(),
            youtube_send_endpoint: String::new(),
            youtube_api_key: String::new(),
            youtube_live_chat_id: String::new(),
            channel,
            token,
            rate_limit: None,
            youtube_quota: None,
        }
    }
}

/// One configured chat account: platform + channel.
///
/// Deliberately token-free: `ChatAccount` clones live in the GUI roster and
/// in every running [`MultiChat`], so keeping the OAuth/session token here
/// would spread a secret across long-lived structs — and CodeQL
/// `rust/cleartext-logging` rightly flags anything derived from it. Tokens
/// are read from the [`ChatTokenStore`] (OS credential vault) at connect
/// time and handed straight to the worker configs; neither the roster nor
/// the config file ever holds a token.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChatAccount {
    pub platform: ChatPlatform,
    /// Twitch channel / Kick slug / YouTube video id.
    pub channel: String,
}

impl ChatAccount {
    pub fn new(platform: ChatPlatform, channel: String) -> Self {
        Self { platform, channel }
    }
}

/// OS-backed storage for chat account tokens (Twitch OAuth, Kick session
/// token). Mirrors [`crate::StreamKeyStore`]: the secret is stored in the OS
/// credential vault and never serialized with the app config or included in
/// diagnostics — the config file keeps only platform + channel.
pub struct ChatTokenStore {
    service: String,
}

impl Default for ChatTokenStore {
    fn default() -> Self {
        Self::new("Rivulet")
    }
}

impl ChatTokenStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// Stable per-account credential key. Includes the platform label and
    /// trimmed channel so several accounts can coexist in one vault.
    pub fn key(platform: ChatPlatform, channel: &str) -> String {
        format!("chat-token/{}/{}", platform.label(), channel.trim())
    }

    /// Credential key of the second secret slot an account can carry: the
    /// YouTube Data API key that pairs with the OAuth token. It is a distinct
    /// vault entry, so neither secret is concatenated with the other and
    /// clearing one cannot damage the other.
    pub fn api_key(platform: ChatPlatform, channel: &str) -> String {
        format!("chat-api-key/{}/{}", platform.label(), channel.trim())
    }

    pub fn save(&self, platform: ChatPlatform, channel: &str, token: &str) -> Result<(), String> {
        keyring::Entry::new(&self.service, &Self::key(platform, channel))
            .map_err(|error| error.to_string())?
            .set_password(token)
            .map_err(|error| error.to_string())
    }

    pub fn load(&self, platform: ChatPlatform, channel: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(&self.service, &Self::key(platform, channel))
            .map_err(|error| error.to_string())?
            .get_password()
        {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Store the YouTube Data API key in its own vault entry.
    pub fn save_api_key(
        &self,
        platform: ChatPlatform,
        channel: &str,
        api_key: &str,
    ) -> Result<(), String> {
        keyring::Entry::new(&self.service, &Self::api_key(platform, channel))
            .map_err(|error| error.to_string())?
            .set_password(api_key)
            .map_err(|error| error.to_string())
    }

    /// Read the YouTube Data API key back (`None` when none was stored).
    pub fn load_api_key(
        &self,
        platform: ChatPlatform,
        channel: &str,
    ) -> Result<Option<String>, String> {
        match keyring::Entry::new(&self.service, &Self::api_key(platform, channel))
            .map_err(|error| error.to_string())?
            .get_password()
        {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn delete(&self, platform: ChatPlatform, channel: &str) -> Result<(), String> {
        match keyring::Entry::new(&self.service, &Self::key(platform, channel))
            .map_err(|error| error.to_string())?
            .delete_credential()
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Combined multi-platform chat facade: one dock, several accounts.
///
/// Each configured account gets its own single-platform worker (the same
/// [`Chat`] machinery the one-platform dock used), so per-platform reconnect
/// behaviour, rate limits and protocol quirks stay isolated. The facade only
/// merges what the dock needs: draining every message stream, broadcasting
/// an outbound message to all capable platforms, routing replies to the
/// platform the parent message came from, and aggregating connection state.
///
/// Accounts are de-duplicated per platform (the first wins): joining the
/// same platform twice would duplicate every line in the dock, which is
/// never what the streamer wants.
pub struct MultiChat {
    workers: Vec<(ChatAccount, Chat)>,
}

impl MultiChat {
    /// Spawn one worker per configured account, resolving each token through
    /// `token_of` at spawn time. Accounts without a channel still produce a
    /// disabled worker so the GUI row can render its "not configured" state
    /// from the same source.
    ///
    /// The token resolver keeps secrets out of the roster: the typical
    /// resolver reads the OS credential vault ([`ChatTokenStore`]) and
    /// returns `""` for accounts without a stored token (read-only).
    pub fn new(accounts: &[ChatAccount], token_of: impl Fn(ChatPlatform, &str) -> String) -> Self {
        let mut workers: Vec<(ChatAccount, Chat)> = Vec::new();
        for account in accounts {
            if account.channel.trim().is_empty() {
                continue;
            }
            if workers
                .iter()
                .any(|(existing, _)| existing.platform == account.platform)
            {
                // Static message on purpose: no account-derived value may
                // flow into a log sink (CodeQL rust/cleartext-logging), and
                // the roster must stay token-free anyway. The GUI refuses
                // duplicate platforms at add time, making this warn purely
                // defensive.
                tracing::warn!("multi-chat: duplicate platform account ignored (first wins)");
                continue;
            }
            let token = token_of(account.platform, account.channel.trim());
            let cfg = ChatConfig::new(account.platform, account.channel.trim().to_owned(), token);
            workers.push((account.clone(), Chat::new(&cfg)));
        }
        Self { workers }
    }

    /// The accounts that actually have a running worker (channel set).
    pub fn accounts(&self) -> impl Iterator<Item = &ChatAccount> {
        self.workers.iter().map(|(account, _)| account)
    }

    /// Whether any worker is actually running.
    pub fn enabled(&self) -> bool {
        self.workers.iter().any(|(_, chat)| chat.enabled())
    }

    /// Number of workers with a configured channel.
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// Per-account connection state for the dock's account rows.
    pub fn connection_states(&self) -> Vec<(ChatPlatform, ChatConnState)> {
        self.workers
            .iter()
            .map(|(account, chat)| (account.platform, chat.connection_state()))
            .collect()
    }

    /// Aggregate connection state: `Connected` when any account is joined,
    /// otherwise `Disconnected` when any account is backing off, otherwise
    /// `Off`. Mirrors the old single-platform status line semantics.
    pub fn connection_state(&self) -> ChatConnState {
        let states = self.connection_states();
        if states.iter().any(|(_, s)| *s == ChatConnState::Connected) {
            ChatConnState::Connected
        } else if states
            .iter()
            .any(|(_, s)| *s == ChatConnState::Disconnected)
        {
            ChatConnState::Disconnected
        } else {
            ChatConnState::Off
        }
    }

    /// Receivers for parsed chat messages, polled by the GUI each frame.
    /// Every message carries its platform tag (the parsers set it), so the
    /// combined dock can badge lines without tracking the source receiver.
    pub fn messages(&self) -> impl Iterator<Item = &Receiver<ChatMessage>> + '_ {
        self.workers.iter().map(|(_, chat)| {
            chat.messages()
                .expect("configured worker always has a receiver")
        })
    }

    /// Receivers for engagement events. Only the Kick worker currently
    /// feeds one (subs/gifts from its Pusher chat stream); the combined
    /// alerts dock drains them alongside the EventSub and Streamlabs
    /// receivers.
    pub fn alert_receivers(
        &self,
    ) -> impl Iterator<Item = &Receiver<crate::alerts_ingest::AlertEvent>> + '_ {
        self.workers.iter().filter_map(|(_, chat)| chat.alerts())
    }

    /// Platform-tagged variant of [`MultiChat::alert_receivers`]: yields each
    /// worker's alert receiver together with its [`ChatAccount`], so the
    /// consumer can attribute drained events to a rate-limit source (one
    /// spamming platform must not starve the others' lanes).
    pub fn alert_receivers_by_platform(
        &self,
    ) -> impl Iterator<Item = (&ChatAccount, &Receiver<crate::alerts_ingest::AlertEvent>)> + '_
    {
        self.workers
            .iter()
            .filter_map(|(account, chat)| chat.alerts().map(|rx| (account, rx)))
    }

    /// Constructor taking full configs (endpoint overrides included), so
    /// tests can point every worker at deterministic local listeners and
    /// advanced setups can tunnel the platform endpoints.
    pub fn from_configs(configs: &[ChatConfig]) -> Self {
        let workers = configs
            .iter()
            .map(|cfg| {
                (
                    ChatAccount::new(cfg.platform, cfg.channel.clone()),
                    Chat::new(cfg),
                )
            })
            .collect();
        Self { workers }
    }

    /// Platforms that can accept outbound messages right now (connected,
    /// token configured, not read-only).
    pub fn sendable_platforms(&self) -> Vec<ChatPlatform> {
        self.workers
            .iter()
            .filter(|(_, chat)| {
                chat.enabled()
                    && chat.can_send()
                    && chat.connection_state() == ChatConnState::Connected
            })
            .map(|(account, _)| account.platform)
            .collect()
    }

    /// Broadcast `text` to every capable worker. Each worker applies its own
    /// platform rate limit. Returns the per-platform outcome in worker order
    /// so the dock can surface partial failures (e.g. Twitch accepted, Kick
    /// rate-limited).
    pub fn send_message(&self, text: &str) -> Vec<(ChatPlatform, bool)> {
        self.workers
            .iter()
            .map(|(account, chat)| (account.platform, chat.send_message(text)))
            .collect()
    }

    /// Reply to a specific chat line on the platform the parent message came
    /// from (only Twitch supports threading). Subject to that platform's
    /// shared rate limiter. Returns `false` when the platform has no running
    /// worker or cannot reply.
    pub fn send_reply(&self, text: &str, reply_to_id: &str, platform: ChatPlatform) -> bool {
        self.workers
            .iter()
            .find(|(account, _)| account.platform == platform)
            .map(|(_, chat)| chat.send_reply(text, reply_to_id))
            .unwrap_or(false)
    }

    /// Whether any worker is configured to send but has spent its YouTube
    /// API quota for today. The dock shows this as "read-only today" — a
    /// budget state, not a configuration error — so the streamer knows the
    /// account is fine and simply has nothing left to spend today.
    pub fn any_quota_exhausted(&self) -> bool {
        self.workers.iter().any(|(_, chat)| chat.quota_exhausted())
    }

    /// Whether any Twitch worker was told by the server that its bot account
    /// must be phone-verified before it can send.
    pub fn phone_verification_required(&self) -> bool {
        self.workers
            .iter()
            .any(|(_, chat)| chat.phone_verification_required())
    }

    /// Outbound rate-limit detail `(remaining, capacity, window_secs)` of the
    /// named platform's worker, for the per-account budget line. `None` when
    /// that platform has no running worker.
    pub fn rate_limit_detail(&self, platform: ChatPlatform) -> Option<(f64, u32, u64)> {
        let chat = &self
            .workers
            .iter()
            .find(|(account, _)| account.platform == platform)?
            .1;
        let cfg = chat.rate_limit_config();
        Some((chat.rate_limit_remaining(), cfg.capacity, cfg.window_secs))
    }

    /// Stop every worker and drop them, mirroring the single-platform GUI
    /// path (workers report their last state until dropped, so clearing is
    /// what actually returns the aggregate to `Off`). Safe to call repeatedly.
    pub fn disconnect_all(&mut self) {
        for (_, chat) in &mut self.workers {
            chat.disconnect();
        }
        self.workers.clear();
    }
}

impl Default for MultiChat {
    fn default() -> Self {
        Self::new(&[], |_, _| String::new())
    }
}

impl std::fmt::Debug for MultiChat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiChat")
            .field(
                "platforms",
                &self
                    .workers
                    .iter()
                    .map(|(account, _)| account.platform)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self::new(ChatPlatform::Twitch, String::new(), String::new())
    }
}

/// Handle to the running chat worker for the selected platform. Non-blocking
/// by construction; mirrors the platform workers. Outbound messages pass
/// through a shared per-platform [`RateLimiter`] so the bot can never burst
/// against a platform limit (Twitch 20/30 s default).
pub struct Chat {
    inner: ChatInner,
    limiter: Mutex<RateLimiter>,
}

enum ChatInner {
    Twitch(TwitchChat),
    Kick(KickChat),
    YouTube(YouTubeChat),
}

impl Chat {
    /// Spawn the worker for `config.platform`. Returns a disabled handle when
    /// no channel is configured.
    pub fn new(config: &ChatConfig) -> Self {
        let inner = match config.platform {
            ChatPlatform::Twitch => ChatInner::Twitch(TwitchChat::new(&TwitchChatConfig {
                endpoint: if config.twitch_endpoint.is_empty() {
                    "irc.chat.twitch.tv:6697".to_owned()
                } else {
                    config.twitch_endpoint.clone()
                },
                nick: String::new(),
                oauth_token: config.token.clone(),
                channel: config.channel.clone(),
            })),
            ChatPlatform::Kick => ChatInner::Kick(KickChat::new(&KickChatConfig {
                ws_endpoint: if config.kick_ws_endpoint.is_empty() {
                    KickChatConfig::default().ws_endpoint
                } else {
                    config.kick_ws_endpoint.clone()
                },
                api_base: if config.kick_api_base.is_empty() {
                    KickChatConfig::default().api_base
                } else {
                    config.kick_api_base.clone()
                },
                channel: config.channel.clone(),
                token: config.token.clone(),
            })),
            ChatPlatform::YouTube => ChatInner::YouTube(YouTubeChat::new(&YouTubeChatConfig {
                page_endpoint: if config.youtube_page_endpoint.is_empty() {
                    YouTubeChatConfig::default().page_endpoint
                } else {
                    config.youtube_page_endpoint.clone()
                },
                poll_endpoint: if config.youtube_poll_endpoint.is_empty() {
                    YouTubeChatConfig::default().poll_endpoint
                } else {
                    config.youtube_poll_endpoint.clone()
                },
                send_endpoint: if config.youtube_send_endpoint.is_empty() {
                    YouTubeChatConfig::default().send_endpoint
                } else {
                    config.youtube_send_endpoint.clone()
                },
                channel: config.channel.clone(),
                live_chat_id: config.youtube_live_chat_id.clone(),
                // The official insert needs *both* halves; a lone token (the
                // common case for a chat account) must not look sendable.
                send_credentials: if config.youtube_api_key.trim().is_empty()
                    || config.token.trim().is_empty()
                {
                    None
                } else {
                    Some(YouTubeSendCredentials::new(
                        config.youtube_api_key.clone(),
                        config.token.clone(),
                    ))
                },
                quota: config.youtube_quota,
            })),
        };
        let limit = config.rate_limit.unwrap_or_else(|| match config.platform {
            ChatPlatform::Twitch => RateLimitConfig::twitch_default(),
            ChatPlatform::Kick => RateLimitConfig::kick_default(),
            ChatPlatform::YouTube => RateLimitConfig::youtube_default(),
        });
        Self {
            inner,
            limiter: Mutex::new(RateLimiter::new(limit)),
        }
    }

    /// Selected platform (for diagnostics and rate-limit reporting).
    pub fn platform(&self) -> ChatPlatform {
        match &self.inner {
            ChatInner::Twitch(_) => ChatPlatform::Twitch,
            ChatInner::Kick(_) => ChatPlatform::Kick,
            ChatInner::YouTube(_) => ChatPlatform::YouTube,
        }
    }

    /// Whether a worker is actually running.
    pub fn enabled(&self) -> bool {
        match &self.inner {
            ChatInner::Twitch(c) => c.enabled(),
            ChatInner::Kick(c) => c.enabled(),
            ChatInner::YouTube(c) => c.enabled(),
        }
    }

    /// Current connection state for the GUI status line.
    pub fn connection_state(&self) -> ChatConnState {
        match &self.inner {
            ChatInner::Twitch(c) => c.connection_state(),
            ChatInner::Kick(c) => c.connection_state(),
            ChatInner::YouTube(c) => c.connection_state(),
        }
    }

    /// Receiver for parsed chat messages, polled by the GUI each frame.
    pub fn messages(&self) -> Option<&Receiver<ChatMessage>> {
        match &self.inner {
            ChatInner::Twitch(c) => c.messages(),
            ChatInner::Kick(c) => c.messages(),
            ChatInner::YouTube(c) => c.messages(),
        }
    }

    /// Receiver for engagement events (Kick subs/gifts, YouTube Super
    /// Chats/Stickers/memberships), when the worker feeds one. Twitch is
    /// `None` — its alerts come from the dedicated EventSub worker.
    pub fn alerts(&self) -> Option<&Receiver<crate::alerts_ingest::AlertEvent>> {
        match &self.inner {
            ChatInner::Twitch(_) => None,
            ChatInner::Kick(c) => c.alerts(),
            ChatInner::YouTube(c) => c.alerts(),
        }
    }

    /// Whether the platform can send chat messages right now.
    ///
    /// Twitch and Kick send whenever a worker runs; YouTube needs the full
    /// official contract (API key, OAuth token, `liveChatId`) *and* quota
    /// left today, so a half-configured or over-budget account correctly
    /// reports itself read-only instead of offering an input that cannot work.
    pub fn can_send(&self) -> bool {
        match &self.inner {
            ChatInner::Twitch(_) | ChatInner::Kick(_) => true,
            ChatInner::YouTube(c) => c.can_send(),
        }
    }

    /// Enqueue a chat message to send. Returns `false` when the worker is
    /// disabled, the platform is read-only, the text is empty, or the
    /// platform rate limit is exhausted.
    pub fn send_message(&self, text: &str) -> bool {
        if !self.can_send() {
            return false;
        }
        {
            let mut limiter = self.limiter.lock().unwrap_or_else(|e| e.into_inner());
            if !limiter.try_acquire() {
                // Static message: no account-derived value in log sinks
                // (CodeQL rust/cleartext-logging).
                tracing::warn!("chat send dropped: platform rate limit exhausted");
                return false;
            }
        }
        match &self.inner {
            ChatInner::Twitch(c) => c.send_message(text),
            ChatInner::Kick(c) => c.send_message(text),
            ChatInner::YouTube(c) => c.send_message(text),
        }
    }

    /// Reply to a specific chat line. Twitch threads via `reply-parent-msg-id`
    /// and YouTube via the insert snippet's `parentId`; Kick has no
    /// IRC-style parent ids and returns `false`. Subject to the same shared
    /// rate limiter as plain sends.
    pub fn send_reply(&self, text: &str, reply_to_id: &str) -> bool {
        if !matches!(self.inner, ChatInner::Twitch(_) | ChatInner::YouTube(_)) {
            return false;
        }
        if reply_to_id.trim().is_empty() {
            return false;
        }
        {
            let mut limiter = self.limiter.lock().unwrap_or_else(|e| e.into_inner());
            if !limiter.try_acquire() {
                // Static message: no account-derived value in log sinks
                // (CodeQL rust/cleartext-logging).
                tracing::warn!("chat reply dropped: platform rate limit exhausted");
                return false;
            }
        }
        match &self.inner {
            ChatInner::Twitch(c) => c.send_reply(text, reply_to_id),
            ChatInner::YouTube(c) => c.send_reply(text, reply_to_id),
            ChatInner::Kick(_) => false,
        }
    }

    /// Whether the account is configured to send but has spent its daily
    /// YouTube API quota — the dock shows "read-only today" rather than a
    /// configuration error. `false` for the other platforms.
    pub fn quota_exhausted(&self) -> bool {
        match &self.inner {
            ChatInner::YouTube(c) => c.quota_exhausted(),
            ChatInner::Twitch(_) | ChatInner::Kick(_) => false,
        }
    }

    /// Whether the active platform's bot account was told by the server that
    /// it must be phone-verified before it can send (Twitch only; the other
    /// platforms have no such requirement).
    pub fn phone_verification_required(&self) -> bool {
        match &self.inner {
            ChatInner::Twitch(c) => c.phone_verification_required(),
            ChatInner::Kick(_) | ChatInner::YouTube(_) => false,
        }
    }

    /// Current outbound rate-limit config for the settings UI.
    pub fn rate_limit_config(&self) -> RateLimitConfig {
        self.limiter
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .config()
    }

    /// Sends still possible right now without waiting (status line).
    pub fn rate_limit_remaining(&self) -> f64 {
        self.limiter
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tokens_available()
    }

    /// Stop the worker. Safe to call repeatedly.
    pub fn disconnect(&mut self) {
        match &mut self.inner {
            ChatInner::Twitch(c) => c.disconnect(),
            ChatInner::Kick(c) => c.disconnect(),
            ChatInner::YouTube(c) => c.disconnect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_when_channel_empty_on_every_platform() {
        for platform in [
            ChatPlatform::Twitch,
            ChatPlatform::Kick,
            ChatPlatform::YouTube,
        ] {
            let chat = Chat::new(&ChatConfig::new(platform, String::new(), String::new()));
            assert!(
                !chat.enabled(),
                "{platform:?} must be disabled without a channel"
            );
            assert_eq!(chat.connection_state(), ChatConnState::Off);
            assert!(!chat.send_message("hello"));
        }
    }

    #[test]
    fn youtube_is_read_only_and_kick_send_requires_worker() {
        let chat = Chat::new(&ChatConfig::new(
            ChatPlatform::YouTube,
            "abc123".to_owned(),
            String::new(),
        ));
        assert!(!chat.can_send(), "YouTube must be read-only");
        assert!(!chat.send_message("hello"));
    }

    #[test]
    fn kick_and_youtube_workers_expose_alert_receivers_twitch_does_not() {
        // The Kick worker always carries the engagement channel (it produces
        // events only when a live subscription/gift arrives).
        let kick = Chat::new(&ChatConfig::new(
            ChatPlatform::Kick,
            "rivulet".to_owned(),
            String::new(),
        ));
        assert!(kick.enabled(), "channel is set");
        assert!(
            kick.alerts().is_some(),
            "the kick worker must expose its engagement receiver"
        );
        // The YouTube worker feeds Super Chats/Stickers/memberships from the
        // same poll feed as its chat messages.
        let youtube = Chat::new(&ChatConfig::new(
            ChatPlatform::YouTube,
            "abc123".to_owned(),
            String::new(),
        ));
        assert!(youtube.enabled(), "video id is set");
        assert!(
            youtube.alerts().is_some(),
            "the youtube worker must expose its engagement receiver"
        );
        // Twitch alerts come from the dedicated EventSub worker — the chat
        // worker emits none.
        let twitch = Chat::new(&ChatConfig::new(
            ChatPlatform::Twitch,
            "rivulet".to_owned(),
            String::new(),
        ));
        assert!(twitch.alerts().is_none());
    }

    #[test]
    fn multichat_collects_kick_and_youtube_alert_receivers() {
        let multi = MultiChat::from_configs(&[
            ChatConfig::new(ChatPlatform::Kick, "kickchannel".to_owned(), String::new()),
            ChatConfig::new(ChatPlatform::YouTube, "abc123".to_owned(), String::new()),
            ChatConfig::new(ChatPlatform::Twitch, "rivulet".to_owned(), String::new()),
        ]);
        let receivers: Vec<_> = multi.alert_receivers().collect();
        assert_eq!(
            receivers.len(),
            2,
            "kick and youtube feed the alert drain, twitch does not"
        );
    }

    #[test]
    fn multichat_tags_alert_receivers_with_their_platform() {
        // The platform-tagged accessor is what lets the GUI attribute drained
        // events to a per-source rate-limit lane (one spamming platform must
        // not starve the others).
        let multi = MultiChat::from_configs(&[
            ChatConfig::new(ChatPlatform::Kick, "kickchannel".to_owned(), String::new()),
            ChatConfig::new(ChatPlatform::YouTube, "abc123".to_owned(), String::new()),
            ChatConfig::new(ChatPlatform::Twitch, "rivulet".to_owned(), String::new()),
        ]);
        let mut tagged: Vec<String> = multi
            .alert_receivers_by_platform()
            .map(|(account, _)| account.platform.label().to_owned())
            .collect();
        tagged.sort();
        assert_eq!(
            tagged,
            vec!["Kick".to_owned(), "YouTube".to_owned()],
            "kick and youtube are tagged, twitch (no receiver) is absent"
        );
    }

    #[test]
    fn platform_default_rate_limits_are_applied() {
        // Twitch: 20/30 s. Kick: 10/30 s (conservative, undocumented API).
        // YouTube: serialized, refilling at the documented ~50/day quota.
        let twitch = Chat::new(&ChatConfig::new(
            ChatPlatform::Twitch,
            "rivulet".to_owned(),
            "oauth:x".to_owned(),
        ));
        assert_eq!(
            twitch.rate_limit_config(),
            crate::rate_limit::RateLimitConfig::twitch_default()
        );
        assert_eq!(twitch.rate_limit_remaining(), 20.0);

        let kick = Chat::new(&ChatConfig::new(
            ChatPlatform::Kick,
            "rivulet".to_owned(),
            "session".to_owned(),
        ));
        assert_eq!(
            kick.rate_limit_config(),
            crate::rate_limit::RateLimitConfig::kick_default()
        );

        let youtube = Chat::new(&ChatConfig::new(
            ChatPlatform::YouTube,
            "abc123".to_owned(),
            String::new(),
        ));
        assert_eq!(
            youtube.rate_limit_config(),
            crate::rate_limit::RateLimitConfig::youtube_default()
        );
    }

    #[test]
    fn send_message_drops_when_custom_rate_limit_is_exhausted() {
        // A real worker would dial irc.chat.twitch.tv; point it at an
        // unreachable loopback so the worker just backs off. The facade-level
        // limiter with capacity 1 must reject the second send before it ever
        // reaches the worker channel.
        let chat = Chat::new(&ChatConfig {
            platform: ChatPlatform::Twitch,
            twitch_endpoint: "127.0.0.1:1".to_owned(),
            channel: "rivulet".to_owned(),
            token: "oauth:x".to_owned(),
            rate_limit: Some(crate::rate_limit::RateLimitConfig {
                capacity: 1,
                window_secs: 30,
            }),
            ..Default::default()
        });
        assert!(chat.enabled());
        assert!(chat.send_message("first"), "burst send must pass");
        // A tiny real-clock refill (< 1 token) is fine; the important part is
        // that the second send is rejected because a full token is missing.
        assert!(
            chat.rate_limit_remaining() < 1.0,
            "bucket must be exhausted after the capacity-1 burst"
        );
        assert!(
            !chat.send_message("second"),
            "second send within the window must be rate-limited"
        );
    }

    #[test]
    fn send_reply_needs_a_parent_id_and_kick_has_no_threading() {
        // An unconfigured YouTube account is an observer: replies are
        // rejected before the limiter.
        let youtube = Chat::new(&ChatConfig::new(
            ChatPlatform::YouTube,
            "abc123".to_owned(),
            String::new(),
        ));
        assert!(!youtube.send_reply("hi", "parent-1"));
        assert!(!youtube.phone_verification_required());

        // Kick has no IRC-style threading: replies are rejected too.
        let kick = Chat::new(&ChatConfig {
            platform: ChatPlatform::Kick,
            channel: "rivulet".to_owned(),
            token: "session".to_owned(),
            ..Default::default()
        });
        assert!(!kick.send_reply("hi", "parent-1"));
        assert!(!kick.phone_verification_required());

        // Twitch: a parent id is mandatory; a valid one is enqueued.
        let mut twitch = Chat::new(&ChatConfig {
            platform: ChatPlatform::Twitch,
            twitch_endpoint: "127.0.0.1:1".to_owned(), // worker backs off
            channel: "rivulet".to_owned(),
            token: "oauth:x".to_owned(),
            ..Default::default()
        });
        assert!(twitch.enabled());
        assert!(!twitch.send_reply("hi", "   "));
        assert!(!twitch.send_reply("   ", "parent-1"));
        assert!(twitch.send_reply("hi", "parent-1"));
        assert!(!twitch.phone_verification_required());
        twitch.disconnect();
    }

    #[test]
    fn read_only_platform_does_not_consume_tokens() {
        let chat = Chat::new(&ChatConfig {
            platform: ChatPlatform::YouTube,
            channel: "abc123".to_owned(),
            rate_limit: Some(crate::rate_limit::RateLimitConfig {
                capacity: 1,
                window_secs: 30,
            }),
            ..Default::default()
        });
        assert!(!chat.send_message("hello"));
        assert_eq!(
            chat.rate_limit_remaining(),
            1.0,
            "read-only rejects must not consume limiter tokens"
        );
    }

    // ── YouTube official send path through the facade ─────────────────

    /// A YouTube account with the full official contract but endpoints that
    /// point nowhere: the facade must report it as sendable and forward the
    /// message to the worker, which then fails the request. That is the
    /// boundary this slice owns \u2014 the transport itself is covered by the
    /// local-listener tests in `youtube_chat`.
    fn youtube_sendable_config() -> ChatConfig {
        ChatConfig {
            platform: ChatPlatform::YouTube,
            channel: "abc123".to_owned(),
            token: "ya29.OAUTH".to_owned(),
            youtube_api_key: "AIzaKEY".to_owned(),
            youtube_live_chat_id: "LC_CHAT".to_owned(),
            youtube_page_endpoint: "http://127.0.0.1:1/live_chat".to_owned(),
            youtube_poll_endpoint: "http://127.0.0.1:1/poll".to_owned(),
            youtube_send_endpoint: "http://127.0.0.1:1/insert".to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn youtube_sends_only_with_the_complete_official_contract() {
        // Nothing configured at all: an observer.
        let bare = Chat::new(&ChatConfig::new(
            ChatPlatform::YouTube,
            "abc123".to_owned(),
            String::new(),
        ));
        assert!(!bare.can_send(), "no credentials means observer mode");
        assert!(!bare.quota_exhausted(), "a config gap is not a quota gap");

        // A token without an API key is still an observer: the official
        // insert needs both, and half a contract must not look sendable.
        let half = Chat::new(&ChatConfig {
            platform: ChatPlatform::YouTube,
            youtube_live_chat_id: "LC_CHAT".to_owned(),
            channel: "abc123".to_owned(),
            token: "ya29.OAUTH".to_owned(),
            youtube_page_endpoint: "http://127.0.0.1:1/live_chat".to_owned(),
            youtube_poll_endpoint: "http://127.0.0.1:1/poll".to_owned(),
            ..Default::default()
        });
        assert!(!half.can_send(), "an OAuth token alone cannot insert");
        assert!(!half.send_message("hi"), "an observer must refuse the send");

        // Credentials but no liveChatId: the insert has no target.
        let no_target = Chat::new(&ChatConfig {
            platform: ChatPlatform::YouTube,
            channel: "abc123".to_owned(),
            token: "ya29.OAUTH".to_owned(),
            youtube_api_key: "AIzaKEY".to_owned(),
            youtube_page_endpoint: "http://127.0.0.1:1/live_chat".to_owned(),
            youtube_poll_endpoint: "http://127.0.0.1:1/poll".to_owned(),
            ..Default::default()
        });
        assert!(
            !no_target.can_send(),
            "insert needs the broadcast's liveChatId"
        );

        // The complete contract: sendable.
        let full = Chat::new(&youtube_sendable_config());
        assert!(full.can_send(), "the full contract must be sendable");
        assert!(
            full.send_message("hello"),
            "a sendable account must forward the message to its worker"
        );
    }

    #[test]
    fn youtube_replies_thread_via_parent_id() {
        let chat = Chat::new(&youtube_sendable_config());
        assert!(
            !chat.send_reply("hi", "   "),
            "a blank parent id is not a reply"
        );
        assert!(chat.send_reply("hi", "PARENT-1"));
    }

    #[test]
    fn a_budget_smaller_than_one_insert_is_read_only_without_faking_a_config_error() {
        // A project whose daily budget cannot even pay for one insert can
        // never send. The dock must report that as a quota state, not as a
        // missing credential.
        let starved = Chat::new(&ChatConfig {
            youtube_quota: Some(crate::youtube_chat::YouTubeQuotaConfig {
                insert_units: 200,
                daily_units: 100,
            }),
            ..youtube_sendable_config()
        });
        assert!(!starved.can_send(), "no send fits into the daily budget");
        assert!(
            starved.quota_exhausted(),
            "this is a quota state, not a configuration gap"
        );
        assert!(!starved.send_message("hi"));
        // A read-only rejection must not charge the limiter either.
        assert_eq!(
            starved.rate_limit_remaining(),
            1.0,
            "a read-only rejection must not consume a limiter token"
        );
    }

    // ── masked auth/scope matrix ──────────────────────────────────────

    #[test]
    fn auth_matrix_reports_presence_but_never_a_value() {
        let rows = crate::youtube_chat::chat_auth_matrix(ChatPlatform::YouTube, true, false);
        let api_key = rows
            .iter()
            .find(|r| r.slot == "api_key")
            .expect("api_key row");
        assert!(api_key.present, "the caller says the key is set");
        let oauth = rows.iter().find(|r| r.slot == "oauth").expect("oauth row");
        assert!(!oauth.present, "the caller says the token is missing");
        // Nothing in the row type can carry a secret: it is a fixed label plus
        // a bool, so the matrix cannot be logged by accident.
        assert!(rows.iter().all(|r| !r.label.contains("AIza")));
    }

    #[test]
    fn auth_matrix_names_the_platform_specific_requirements() {
        let twitch: Vec<_> =
            crate::youtube_chat::chat_auth_matrix(ChatPlatform::Twitch, false, true)
                .into_iter()
                .map(|r| r.slot)
                .collect();
        assert_eq!(twitch, ["oauth", "irc_tags", "phone_verified"]);
        let kick: Vec<_> = crate::youtube_chat::chat_auth_matrix(ChatPlatform::Kick, false, true)
            .into_iter()
            .map(|r| r.slot)
            .collect();
        assert_eq!(kick, ["session"]);
        let youtube: Vec<_> =
            crate::youtube_chat::chat_auth_matrix(ChatPlatform::YouTube, true, true)
                .into_iter()
                .map(|r| r.slot)
                .collect();
        assert_eq!(youtube, ["api_key", "oauth", "live_chat_id"]);
    }

    // ── MultiChat: combined multi-platform dock facade ────────────────

    fn account(platform: ChatPlatform, channel: &str) -> ChatAccount {
        ChatAccount::new(platform, channel.to_owned())
    }

    #[test]
    fn multichat_spawns_one_worker_per_platform() {
        let multi = MultiChat::new(
            &[
                account(ChatPlatform::Twitch, "rivulet"),
                account(ChatPlatform::Kick, "rivulet"),
            ],
            |_, _| String::new(),
        );
        assert_eq!(multi.worker_count(), 2);
        assert!(multi.enabled());
        let platforms: Vec<_> = multi.accounts().map(|a| a.platform).collect();
        assert_eq!(platforms, [ChatPlatform::Twitch, ChatPlatform::Kick]);
        // Worker connection state transitions asynchronously (Off before the
        // worker thread first runs, then Connected/Disconnected), so only
        // structural facts are asserted here; the aggregation semantics are
        // covered by the deterministic loopback test below.
    }

    #[test]
    fn multichat_ignores_empty_channels_and_duplicate_platforms() {
        let multi = MultiChat::new(
            &[
                account(ChatPlatform::Twitch, ""),
                account(ChatPlatform::Twitch, "first"),
                account(ChatPlatform::Twitch, "second"),
                account(ChatPlatform::Kick, "rivulet"),
            ],
            |_, _| String::new(),
        );
        assert_eq!(
            multi.worker_count(),
            2,
            "empty channel and duplicate platform must not spawn workers"
        );
        let twitch = multi
            .accounts()
            .find(|a| a.platform == ChatPlatform::Twitch)
            .expect("twitch worker");
        assert_eq!(twitch.channel, "first", "first account must win");
    }

    #[test]
    fn multichat_empty_account_list_is_off() {
        let multi = MultiChat::new(&[], |_, _| String::new());
        assert_eq!(multi.worker_count(), 0);
        assert!(!multi.enabled());
        assert_eq!(multi.connection_state(), ChatConnState::Off);
        assert!(multi.send_message("hello").is_empty());
        assert!(!multi.send_reply("hi", "parent-1", ChatPlatform::Twitch));
    }

    #[test]
    fn multichat_broadcast_reports_per_platform_outcome() {
        // Both workers are enabled but unreachable; the enqueue itself still
        // succeeds on capable platforms and is refused on read-only YouTube.
        let multi = MultiChat::new(
            &[
                account(ChatPlatform::Twitch, "rivulet"),
                account(ChatPlatform::YouTube, "abc123"),
            ],
            |_, _| String::new(),
        );
        let outcomes = multi.send_message("hello");
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].0, ChatPlatform::Twitch);
        assert!(outcomes[0].1, "twitch enqueue must pass");
        assert_eq!(outcomes[1].0, ChatPlatform::YouTube);
        assert!(
            !outcomes[1].1,
            "an unconfigured youtube account is read-only"
        );
        // Twitch's 20/30 s bucket now sits below 1; YouTube's bucket stays
        // untouched — proving the per-platform rate-limit isolation.
        assert!(multi.rate_limit_detail(ChatPlatform::Twitch).unwrap().0 < 20.0);
        assert_eq!(
            multi.rate_limit_detail(ChatPlatform::YouTube).unwrap(),
            (1.0, 1, 1_728),
            "read-only rejects must not consume the youtube bucket"
        );
    }

    #[test]
    fn multichat_reply_routes_to_the_parent_platform_only() {
        let multi = MultiChat::new(
            &[
                account(ChatPlatform::Twitch, "rivulet"),
                account(ChatPlatform::Kick, "rivulet"),
            ],
            |_, _| String::new(),
        );
        // Twitch accepts the enqueue; Kick has no threading.
        assert!(multi.send_reply("hi", "parent-1", ChatPlatform::Twitch));
        assert!(!multi.send_reply("hi", "parent-1", ChatPlatform::Kick));
        // A platform without a running worker rejects before the limiter.
        assert!(!multi.send_reply("hi", "parent-1", ChatPlatform::YouTube));
        // An empty parent id is rejected on the capable platform too.
        assert!(!multi.send_reply("hi", "   ", ChatPlatform::Twitch));
    }

    #[test]
    fn multichat_rate_limit_details_are_per_platform() {
        let multi = MultiChat::new(
            &[
                account(ChatPlatform::Twitch, "rivulet"),
                account(ChatPlatform::YouTube, "abc123"),
            ],
            |_, _| String::new(),
        );
        let (remaining, capacity, window) = multi.rate_limit_detail(ChatPlatform::Twitch).unwrap();
        assert_eq!((remaining, capacity, window), (20.0, 20, 30));
        assert_eq!(
            multi.rate_limit_detail(ChatPlatform::YouTube).unwrap(),
            (1.0, 1, 1_728)
        );
        assert!(multi.rate_limit_detail(ChatPlatform::Kick).is_none());
    }

    #[test]
    fn multichat_sendable_platforms_excludes_read_only() {
        let multi = MultiChat::new(
            &[
                account(ChatPlatform::Twitch, "rivulet"),
                account(ChatPlatform::YouTube, "abc123"),
            ],
            |_, _| String::new(),
        );
        let sendable = multi.sendable_platforms();
        // The spawn-time connection state races the dial, so the list may or
        // may not contain the capable platform; what must hold always is that
        // the read-only platform is never advertised.
        assert!(!sendable.contains(&ChatPlatform::YouTube));
        assert!(sendable
            .iter()
            .all(|p| *p == ChatPlatform::Twitch || *p == ChatPlatform::Kick));
    }

    #[test]
    fn multichat_disconnect_all_stops_every_worker() {
        let mut multi = MultiChat::new(
            &[
                account(ChatPlatform::Twitch, "rivulet"),
                account(ChatPlatform::Kick, "rivulet"),
            ],
            |_, _| String::new(),
        );
        assert!(multi.enabled());
        multi.disconnect_all();
        assert!(
            !multi.enabled(),
            "workers must be dropped on disconnect_all"
        );
        assert_eq!(multi.worker_count(), 0);
        assert_eq!(multi.connection_state(), ChatConnState::Off);
        assert!(multi.send_message("hello").is_empty());
        // Repeated calls must stay safe.
        multi.disconnect_all();
    }

    #[test]
    fn chat_account_roster_never_carries_a_token() {
        // The roster must stay token-free: `ChatAccount` is cloned into the
        // GUI app state and every running MultiChat, so a token field here
        // would taint everything derived from it (CodeQL
        // rust/cleartext-logging). Tokens live in the OS credential vault
        // and are resolved per-connect via a closure, never on the struct.
        let account = account(ChatPlatform::Twitch, "rivulet");
        let json = serde_json::to_string(&account).expect("serialize account");
        assert_eq!(
            json, r#"{"platform":"Twitch","channel":"rivulet"}"#,
            "serialized account must contain exactly platform + channel"
        );
        let back: ChatAccount = serde_json::from_str(&json).expect("deserialize account");
        assert_eq!(back, account);
    }

    #[test]
    fn multichat_resolves_tokens_through_the_callback() {
        // The resolver closure is the only token source at spawn time: the
        // roster passes platform + channel, the resolver (the GUI reads the
        // OS credential vault) returns the token. The other tests on this
        // page use an empty resolver, mirroring read-only accounts.
        let multi = MultiChat::new(
            &[account(ChatPlatform::Twitch, "rivulet")],
            |platform, channel| {
                assert_eq!(platform, ChatPlatform::Twitch);
                assert_eq!(channel, "rivulet");
                "oauth:from-resolver".to_owned()
            },
        );
        assert_eq!(multi.worker_count(), 1);
    }

    #[test]
    fn chat_token_store_keys_are_stable_per_platform_and_channel() {
        let twitch = ChatTokenStore::key(ChatPlatform::Twitch, " rivulet ");
        assert_eq!(twitch, "chat-token/Twitch/rivulet");
        assert_ne!(
            twitch,
            ChatTokenStore::key(ChatPlatform::Kick, "rivulet"),
            "platforms must not share credential entries"
        );
        assert_ne!(
            ChatTokenStore::key(ChatPlatform::Twitch, "a"),
            ChatTokenStore::key(ChatPlatform::Twitch, "b"),
            "channels must not share credential entries"
        );
    }
}
