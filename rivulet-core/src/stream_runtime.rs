//! Runtime policies shared by streaming transports.

use std::time::Duration;

use crate::{AdaptiveBitrate, StreamHealthStatus, StreamStats};

/// A deterministic controller that avoids bitrate flapping during transient
/// health changes. Callers supply a monotonic elapsed time so it is easy to
/// test and safe to drive from a GStreamer/GUI scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdaptiveBitrateController {
    pub cooldown: Duration,
    pub required_samples: u8,
    pending: Option<(StreamHealthStatus, u8)>,
    last_change: Option<Duration>,
    last_change_info: Option<BitrateChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitrateChange {
    pub from_kbps: u32,
    pub to_kbps: u32,
    pub status: StreamHealthStatus,
}

impl Default for AdaptiveBitrateController {
    fn default() -> Self {
        Self {
            cooldown: Duration::from_secs(5),
            required_samples: 2,
            pending: None,
            last_change: None,
            last_change_info: None,
        }
    }
}

impl AdaptiveBitrateController {
    pub fn new(cooldown: Duration, required_samples: u8) -> Self {
        Self {
            cooldown,
            required_samples: required_samples.max(1),
            last_change_info: None,
            ..Self::default()
        }
    }

    pub fn last_change(&self) -> Option<BitrateChange> {
        self.last_change_info
    }

    pub fn update_from_telemetry(
        &mut self,
        policy: AdaptiveBitrate,
        current_kbps: &mut u32,
        telemetry: &StreamStats,
        now: Duration,
    ) -> bool {
        self.update(policy, current_kbps, telemetry.status, now)
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
        let previous = *current_kbps;
        let next = policy.next_bitrate(previous, status);
        self.pending = None;
        if next == previous {
            return false;
        }
        *current_kbps = next;
        self.last_change = Some(now);
        self.last_change_info = Some(BitrateChange {
            from_kbps: previous,
            to_kbps: next,
            status,
        });
        true
    }
}

/// Keeps the configured delay stable across a temporary sink reconnect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelaySupervisor {
    configured: Duration,
    buffered: Duration,
    max_buffer: Duration,
    dropped: u64,
    underflows: u64,
}

impl DelaySupervisor {
    pub fn new(configured: Duration, max_buffer: Duration) -> Self {
        let max_buffer = max_buffer.max(configured);
        Self {
            configured: configured.min(max_buffer),
            buffered: Duration::ZERO,
            max_buffer,
            dropped: 0,
            underflows: 0,
        }
    }

    pub fn configured(&self) -> Duration {
        self.configured
    }
    pub fn buffered(&self) -> Duration {
        self.buffered
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn underflows(&self) -> u64 {
        self.underflows
    }

    pub fn fill_ratio(&self) -> f64 {
        if self.max_buffer.is_zero() {
            0.0
        } else {
            (self.buffered.as_secs_f64() / self.max_buffer.as_secs_f64()).clamp(0.0, 1.0)
        }
    }

    /// Record an underflow when a delayed branch has no media available.
    pub fn record_underflow(&mut self) {
        self.underflows = self.underflows.saturating_add(1);
    }

    pub fn push(&mut self, amount: Duration) {
        let next = self.buffered.saturating_add(amount);
        if next > self.max_buffer {
            self.dropped = self.dropped.saturating_add(1);
        }
        self.buffered = next.min(self.max_buffer);
    }

    pub fn consume(&mut self, amount: Duration) {
        if amount > self.buffered {
            self.record_underflow();
        }
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
    fn telemetry_update_uses_observed_health_status() {
        let policy = AdaptiveBitrate::new(1000, 3000, 500);
        let mut controller = AdaptiveBitrateController::new(Duration::ZERO, 1);
        let mut bitrate = 2000;
        let mut telemetry = StreamStats::offline();
        telemetry.status = StreamHealthStatus::Poor;
        telemetry.sink_latency_ms = Some(250.0);
        telemetry.queue_fill_ratio = Some(0.95);
        assert!(controller.update_from_telemetry(
            policy,
            &mut bitrate,
            &telemetry,
            Duration::from_secs(1)
        ));
        assert_eq!(bitrate, 1500);
        assert_eq!(
            controller.last_change(),
            Some(BitrateChange {
                from_kbps: 2000,
                to_kbps: 1500,
                status: StreamHealthStatus::Poor,
            })
        );
    }

    #[test]
    fn bitrate_change_diagnostics_are_empty_until_a_change() {
        let mut controller = AdaptiveBitrateController::new(Duration::ZERO, 1);
        let policy = AdaptiveBitrate::new(1000, 3000, 500);
        let mut bitrate = 2000;
        assert!(!controller.update(
            policy,
            &mut bitrate,
            StreamHealthStatus::Warning,
            Duration::ZERO
        ));
        assert_eq!(controller.last_change(), None);
    }

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
        assert_eq!(delay.dropped(), 1);
        delay.on_disconnect();
        assert_eq!(delay.on_reconnect(), Duration::from_secs(10));
        delay.consume(Duration::from_secs(4));
        assert_eq!(delay.buffered(), Duration::from_secs(6));
        assert!(delay.fill_ratio() > 0.0);
        delay.consume(Duration::from_secs(10));
        assert_eq!(delay.underflows(), 1);
    }
}
