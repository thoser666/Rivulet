//! Shared per-platform outbound rate limiter (token bucket).
//!
//! The chat bot must never burst against a platform's rate limits: Twitch
//! drops messages and can lock an account out for 30 minutes when a
//! non-broadcaster/mod/VIP account exceeds 20 messages per 30 seconds
//! (global, not per channel); Kick publishes no limits at all (undocumented
//! API), so a conservative self-throttle is the only safe behavior; YouTube's
//! official `liveChatMessages.insert` costs ~200 quota units against a daily
//! budget of 10 000. This module implements the single token-bucket primitive
//! the platform workers and the `Chat` facade gate their sends with.
//!
//! The bucket is clock-injectable so tests are deterministic (a manual clock
//! advances in fixed steps instead of sleeping); the default clock is a
//! monotonic seconds counter.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Tokens-per-second refill schedule for one platform send path.
///
/// `capacity` is the burst size (how many sends may happen back-to-back) and
/// `window_secs` is the refill window: the bucket refills `capacity` tokens
/// over `window_secs` seconds. Defaults encode each platform's documented or
/// conservative behavior (see [`RateLimitConfig::twitch_default`] etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum burst of sends without waiting.
    pub capacity: u32,
    /// Seconds over which the full capacity refills.
    pub window_secs: u64,
}

impl RateLimitConfig {
    /// Twitch: 20 messages / 30 s for non-broadcaster/mod/VIP accounts,
    /// enforced globally per account (https://dev.twitch.tv/docs/chat/).
    pub const fn twitch_default() -> Self {
        Self {
            capacity: 20,
            window_secs: 30,
        }
    }

    /// Kick: no published limits, so stay well below the Twitch ceiling —
    /// 10 messages / 30 s is a deliberately conservative default for the
    /// undocumented API.
    pub const fn kick_default() -> Self {
        Self {
            capacity: 10,
            window_secs: 30,
        }
    }

    /// YouTube: `liveChatMessages.insert` costs ~200 quota units against a
    /// 10 000-unit daily budget → ~50 sends/day. A burst of 1 (send only when
    /// a token is available) with the daily refill rate never overdraws the
    /// quota.
    pub const fn youtube_default() -> Self {
        Self {
            capacity: 1,
            window_secs: 86_400,
        }
    }

    /// Per-second refill rate derived from the window.
    pub fn refill_per_sec(&self) -> f64 {
        self.capacity as f64 / self.window_secs.max(1) as f64
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self::twitch_default()
    }
}

type Clock = Box<dyn Fn() -> f64 + Send + Sync>;

/// A token bucket that refills continuously and never exceeds `capacity`.
///
/// The bucket starts full (a fresh worker may send its burst immediately) and
/// refills at `config.refill_per_sec()` tokens per second up to the capacity.
/// `try_acquire` consumes one token when available; it never blocks.
pub struct RateLimiter {
    config: RateLimitConfig,
    tokens: f64,
    last: f64,
    now: Clock,
}

fn monotonic_secs() -> f64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
}

impl RateLimiter {
    /// Create a bucket with `config`, starting full, using a monotonic clock.
    pub fn new(config: RateLimitConfig) -> Self {
        Self::with_clock(config, monotonic_secs)
    }

    /// Create a bucket with an explicit clock (seconds since an arbitrary
    /// origin). Tests inject a manual clock to advance time deterministically.
    pub fn with_clock(
        config: RateLimitConfig,
        now: impl Fn() -> f64 + Send + Sync + 'static,
    ) -> Self {
        let now_secs = now();
        Self {
            tokens: config.capacity as f64,
            last: now_secs,
            config,
            now: Box::new(now),
        }
    }

    /// Current config (capacity/window), e.g. for the settings UI.
    pub fn config(&self) -> RateLimitConfig {
        self.config
    }

    /// Consume one token if available; refills up to capacity first.
    /// Returns `false` when the bucket is empty (the caller should skip the
    /// send or queue it for the next token).
    pub fn try_acquire(&mut self) -> bool {
        let now = (self.now)();
        let elapsed = (now - self.last).max(0.0);
        self.tokens =
            (self.tokens + elapsed * self.config.refill_per_sec()).min(self.config.capacity as f64);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// How many sends are currently possible without waiting (for status UI).
    pub fn tokens_available(&self) -> f64 {
        let now = (self.now)();
        let elapsed = (now - self.last).max(0.0);
        (self.tokens + elapsed * self.config.refill_per_sec()).min(self.config.capacity as f64)
    }

    /// Reset the bucket to full.
    pub fn reset(&mut self) {
        self.tokens = self.config.capacity as f64;
        self.last = (self.now)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// A shared manual clock in milliseconds so the closure is `Send + Sync`
    /// (like the production clock) while tests control time deterministically.
    fn manual_clock() -> (Arc<AtomicU64>, impl Fn() -> f64) {
        let ms = Arc::new(AtomicU64::new(0));
        let tick = Arc::clone(&ms);
        let clock = move || tick.load(Ordering::Relaxed) as f64 / 1000.0;
        (ms, clock)
    }

    #[test]
    fn twitch_default_is_20_per_30_seconds() {
        let config = RateLimitConfig::twitch_default();
        assert_eq!((config.capacity, config.window_secs), (20, 30));
        assert!((config.refill_per_sec() - 20.0 / 30.0).abs() < 1e-9);
        assert_eq!(RateLimitConfig::default(), config);
    }

    #[test]
    fn kick_default_is_conservative() {
        let config = RateLimitConfig::kick_default();
        assert_eq!((config.capacity, config.window_secs), (10, 30));
        assert!(
            config.refill_per_sec() < RateLimitConfig::twitch_default().refill_per_sec(),
            "Kick must throttle harder than Twitch (undocumented limits)"
        );
    }

    #[test]
    fn youtube_default_is_quota_bounded() {
        let config = RateLimitConfig::youtube_default();
        assert_eq!(config.capacity, 1, "YouTube sends must be serialized");
        let per_day = config.refill_per_sec() * 86_400.0;
        assert!(
            (per_day - 1.0).abs() < 1e-9,
            "one token per day at the documented ~50-sends quota budget"
        );
    }

    #[test]
    fn burst_is_limited_to_capacity() {
        let (cell, clock) = manual_clock();
        let mut bucket = RateLimiter::with_clock(RateLimitConfig::twitch_default(), clock);
        for _ in 0..20 {
            assert!(bucket.try_acquire(), "capacity burst must succeed");
        }
        assert!(
            !bucket.try_acquire(),
            "the 21st send within the window must be rejected"
        );
        // Time does not pass in the manual clock: still empty.
        assert!(!bucket.try_acquire());
        assert_eq!(bucket.tokens_available(), 0.0);
        // The bucket never goes negative.
        cell.store(100_000, Ordering::Relaxed);
        assert!(bucket.tokens_available() <= 20.0);
    }

    #[test]
    fn tokens_refill_over_the_window() {
        let (cell, clock) = manual_clock();
        let mut bucket = RateLimiter::with_clock(RateLimitConfig::twitch_default(), clock);
        // Empty the bucket.
        for _ in 0..20 {
            assert!(bucket.try_acquire());
        }
        assert!(!bucket.try_acquire());
        // 15 seconds later exactly half the capacity has refilled.
        cell.store(15_000, Ordering::Relaxed);
        assert!((bucket.tokens_available() - 10.0).abs() < 1e-9);
        // A full window restores the full burst.
        cell.store(30_000, Ordering::Relaxed);
        assert!((bucket.tokens_available() - 20.0).abs() < 1e-9);
        for _ in 0..20 {
            assert!(bucket.try_acquire());
        }
        assert!(!bucket.try_acquire());
    }

    #[test]
    fn refill_never_exceeds_capacity() {
        let (cell, clock) = manual_clock();
        let mut bucket = RateLimiter::with_clock(RateLimitConfig::twitch_default(), clock);
        for _ in 0..20 {
            assert!(bucket.try_acquire());
        }
        cell.store(3_600_000, Ordering::Relaxed); // one hour later
        assert!(
            (bucket.tokens_available() - 20.0).abs() < 1e-9,
            "tokens must cap at capacity even after long idle"
        );
    }

    #[test]
    fn reset_restores_full_burst() {
        let (_, clock) = manual_clock();
        let mut bucket = RateLimiter::with_clock(RateLimitConfig::twitch_default(), clock);
        for _ in 0..20 {
            assert!(bucket.try_acquire());
        }
        assert!(!bucket.try_acquire());
        bucket.reset();
        for _ in 0..20 {
            assert!(bucket.try_acquire());
        }
    }

    #[test]
    fn fractional_refill_accumulates() {
        let (cell, clock) = manual_clock();
        let mut bucket = RateLimiter::with_clock(RateLimitConfig::kick_default(), clock);
        for _ in 0..10 {
            assert!(bucket.try_acquire());
        }
        assert!(!bucket.try_acquire());
        // Kick refills 10 tokens / 30 s = 1/3 token per second.
        cell.store(3_000, Ordering::Relaxed);
        assert!((bucket.tokens_available() - 1.0).abs() < 1e-9);
        assert!(bucket.try_acquire());
        assert!(!bucket.try_acquire());
    }
}
