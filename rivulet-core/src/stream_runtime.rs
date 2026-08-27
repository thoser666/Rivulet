//! Runtime policies shared by streaming transports.

use std::time::Duration;

use crate::{AdaptiveBitrate, StreamHealthStatus};

/// A deterministic controller that avoids bitrate flapping during transient
/// health changes. Callers supply a monotonic elapsed time so it is easy to
/// test and safe to drive from a GStreamer/GUI scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdaptiveBitrateController {
    pub cooldown: Duration,
    pub required_samples: u8,
    pending: Option<(StreamHealthStatus, u8)>,
    last_change: Option<Duration>,
}

impl Default for AdaptiveBitrateController {
    fn default() -> Self {
        Self {
            cooldown: Duration::from_secs(5),
            required_samples: 2,
            pending: None,
            last_change: None,
        }
    }
}

impl AdaptiveBitrateController {
    pub fn new(cooldown: Duration, required_samples: u8) -> Self {
        Self {
            cooldown,
            required_samples: required_samples.max(1),
            ..Self::default()
        }
    }

    pub fn update(
        &mut self,
        policy: AdaptiveBitrate,
        current_kbps: &mut u32,
        status: StreamHealthStatus,
        now: Duration,
    ) -> bool {
        if !policy.enabled || !matches!(status, StreamHealthStatus::Poor | StreamHealthStatus::Good)
        {
            self.pending = None;
            return false;
        }
        let samples = match self.pending {
            Some((previous, count)) if previous == status => count.saturating_add(1),
            _ => 1,
        };
        self.pending = Some((status, samples));
        if samples < self.required_samples
            || self
                .last_change
                .is_some_and(|last| now.saturating_sub(last) < self.cooldown)
        {
            return false;
        }
        let next = policy.next_bitrate(*current_kbps, status);
        self.pending = None;
        if next == *current_kbps {
            return false;
        }
        *current_kbps = next;
        self.last_change = Some(now);
        true
    }
}

/// Keeps the configured delay stable across a temporary sink reconnect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelaySupervisor {
    configured: Duration,
    buffered: Duration,
    max_buffer: Duration,
}

impl DelaySupervisor {
    pub fn new(configured: Duration, max_buffer: Duration) -> Self {
        let max_buffer = max_buffer.max(configured);
        Self {
            configured: configured.min(max_buffer),
            buffered: Duration::ZERO,
            max_buffer,
        }
    }

    pub fn configured(&self) -> Duration {
        self.configured
    }
    pub fn buffered(&self) -> Duration {
        self.buffered
    }

    pub fn push(&mut self, amount: Duration) {
        self.buffered = self.buffered.saturating_add(amount).min(self.max_buffer);
    }

    pub fn consume(&mut self, amount: Duration) {
        self.buffered = self.buffered.saturating_sub(amount);
    }

    pub fn on_disconnect(&mut self) {
        // Preserve buffered media; the reconnect path can drain it once live.
    }

    pub fn on_reconnect(&mut self) -> Duration {
        let retained = self.buffered.min(self.configured);
        self.buffered = retained;
        retained
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrate_controller_requires_stable_samples_and_cooldown() {
        let policy = AdaptiveBitrate::new(1000, 3000, 500);
        let mut controller = AdaptiveBitrateController::new(Duration::from_secs(5), 2);
        let mut bitrate = 2000;
        assert!(!controller.update(
            policy,
            &mut bitrate,
            StreamHealthStatus::Poor,
            Duration::ZERO
        ));
        assert!(controller.update(
            policy,
            &mut bitrate,
            StreamHealthStatus::Poor,
            Duration::from_secs(1)
        ));
        assert_eq!(bitrate, 1500);
        assert!(!controller.update(
            policy,
            &mut bitrate,
            StreamHealthStatus::Good,
            Duration::from_secs(2)
        ));
        assert!(controller.update(
            policy,
            &mut bitrate,
            StreamHealthStatus::Good,
            Duration::from_secs(7)
        ));
        assert_eq!(bitrate, 2000);
    }

    #[test]
    fn delay_supervisor_preserves_and_bounds_buffer_on_reconnect() {
        let mut delay = DelaySupervisor::new(Duration::from_secs(10), Duration::from_secs(15));
        delay.push(Duration::from_secs(20));
        assert_eq!(delay.buffered(), Duration::from_secs(15));
        delay.on_disconnect();
        assert_eq!(delay.on_reconnect(), Duration::from_secs(10));
        delay.consume(Duration::from_secs(4));
        assert_eq!(delay.buffered(), Duration::from_secs(6));
    }
}
