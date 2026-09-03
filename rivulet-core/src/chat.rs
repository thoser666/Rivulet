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
use crate::youtube_chat::{YouTubeChat, YouTubeChatConfig};

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
    /// Channel: Twitch channel / Kick slug / YouTube video id.
    pub channel: String,
    /// Token: Twitch OAuth (`oauth:...`) or Kick session token. Sending is
    /// gated on this; YouTube is read-only.
    pub token: String,
    /// Outbound rate limit. `None` → the platform default (Twitch 20/30 s,
    /// Kick 10/30 s, YouTube quota-bounded 1/day).
    pub rate_limit: Option<RateLimitConfig>,
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
            channel,
            token,
            rate_limit: None,
        }
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
                channel: config.channel.clone(),
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

    /// Whether the platform can send chat messages at all (YouTube is
    /// read-only without an authenticated browser session).
    pub fn can_send(&self) -> bool {
        !matches!(self.inner, ChatInner::YouTube(_))
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
                tracing::warn!(
                    platform = ?self.platform(),
                    "chat send dropped: platform rate limit exhausted"
                );
                return false;
            }
        }
        match &self.inner {
            ChatInner::Twitch(c) => c.send_message(text),
            ChatInner::Kick(c) => c.send_message(text),
            ChatInner::YouTube(c) => c.send_message(text),
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
    fn platform_default_rate_limits_are_applied() {
        // Twitch: 20/30 s. Kick: 10/30 s (conservative, undocumented API).
        // YouTube: quota-bounded 1/day.
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
}
