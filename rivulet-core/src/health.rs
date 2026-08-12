//! Stream health and network statistics.
//!
//! While a stream is active, the engine tracks how many video frames were sent
//! to the sink and how many had to be dropped, together with the accumulated
//! byte count. A sliding window over recent samples is used to estimate the
//! current bitrate and frame rate, from which a [`StreamHealthStatus`] is
//! derived (similar to OBS' "Stream Health" indicator).

use std::collections::VecDeque;
use std::time::Instant;

/// Health status of an active stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamHealthStatus {
    /// No stream is currently active.
    Offline,
    /// The stream just started; not enough data to judge yet.
    Connecting,
    /// Healthy: negligible dropped frames, bitrate near the target.
    Good,
    /// Degraded: notable dropped frames or a clearly lower bitrate.
    Warning,
    /// Critical: many dropped frames or the bitrate collapsed.
    Poor,
}

/// A snapshot of the current stream statistics.
#[derive(Debug, Clone)]
pub struct StreamStats {
    /// Derived health status.
    pub status: StreamHealthStatus,
    /// Total frames successfully pushed into the pipeline.
    pub frames_sent: u64,
    /// Total frames that could not be pushed (dropped).
    pub frames_dropped: u64,
    /// Total bytes pushed (payload size).
    pub bytes_sent: u64,
    /// Estimated current bitrate in kbit/s (sliding window).
    pub kbps: f64,
    /// Estimated current frame rate (sliding window).
    pub fps: f64,
    /// Uptime in seconds since the monitor was created.
    pub uptime_secs: u64,
    /// Fraction of dropped frames over all sent+dropped frames (0.0 .. 1.0).
    pub dropped_ratio: f64,
}

impl StreamStats {
    /// Statistics for a stream that is not running.
    pub fn offline() -> Self {
        Self {
            status: StreamHealthStatus::Offline,
            frames_sent: 0,
            frames_dropped: 0,
            bytes_sent: 0,
            kbps: 0.0,
            fps: 0.0,
            uptime_secs: 0,
            dropped_ratio: 0.0,
        }
    }
}

/// Tracks stream statistics over a sliding time window and derives a health
/// status from the dropped-frame ratio and the current bitrate.
pub struct StreamHealthMonitor {
    /// Target bitrate in kbit/s used to judge whether the bitrate collapsed.
    target_bitrate_kbps: f64,
    /// Target frame rate used to judge whether the frame rate collapsed.
    target_fps: f64,
    /// Length of the sliding window in seconds.
    window_secs: f64,
    start: Instant,
    frames_sent: u64,
    frames_dropped: u64,
    bytes_sent: u64,
    /// Sliding window samples `(timestamp, bytes, frames)`.
    window: VecDeque<(Instant, u64, u64)>,
}

impl StreamHealthMonitor {
    /// Create a monitor. `target_bitrate_kbps` and `target_fps` are the values
    /// the encoder is configured for and serve as the reference for judging
    /// whether the stream collapsed.
    pub fn new(target_bitrate_kbps: f64, target_fps: f64) -> Self {
        Self::with_window(target_bitrate_kbps, target_fps, 5.0)
    }

    /// Like [`Self::new`], but with a custom sliding-window length. A shorter
    /// window reacts faster but is noisier; used by tests and embedded setups.
    pub fn with_window(target_bitrate_kbps: f64, target_fps: f64, window_secs: f64) -> Self {
        Self {
            target_bitrate_kbps,
            target_fps,
            window_secs,
            start: Instant::now(),
            frames_sent: 0,
            frames_dropped: 0,
            bytes_sent: 0,
            window: VecDeque::new(),
        }
    }

    /// Record a successfully pushed frame with its payload size in bytes.
    pub fn record_frame_sent(&mut self, bytes: usize) {
        self.frames_sent += 1;
        self.bytes_sent = self.bytes_sent.saturating_add(bytes as u64);
        self.window.push_back((Instant::now(), bytes as u64, 1));
        self.prune_window();
    }

    /// Record a frame that could not be pushed into the pipeline.
    pub fn record_frame_dropped(&mut self) {
        self.frames_dropped += 1;
        self.prune_window();
    }

    /// Reset all counters (e.g. when a new stream starts).
    pub fn reset(&mut self) {
        *self = Self::new(self.target_bitrate_kbps, self.target_fps);
    }

    /// Current statistics and derived health status.
    ///
    /// Prunes the sliding window first, so callers always observe the current
    /// state even if no frames were pushed recently.
    pub fn stats(&mut self) -> StreamStats {
        self.prune_window();
        let uptime_secs = self.start.elapsed().as_secs();
        let total_frames = self.frames_sent + self.frames_dropped;
        let dropped_ratio = if total_frames > 0 {
            self.frames_dropped as f64 / total_frames as f64
        } else {
            0.0
        };

        let (elapsed, window_bytes, window_frames) = self.window_bounds();
        let (kbps, fps) = if !self.window.is_empty() && elapsed > 0.0 {
            (
                window_bytes as f64 * 8.0 / elapsed / 1000.0,
                window_frames as f64 / elapsed,
            )
        } else {
            (0.0, 0.0)
        };

        let status = if self.frames_sent == 0 && uptime_secs < 5 {
            StreamHealthStatus::Connecting
        } else if dropped_ratio > 0.05 {
            StreamHealthStatus::Poor
        } else if dropped_ratio > 0.01 {
            StreamHealthStatus::Warning
        } else if !self.window.is_empty()
            && (self.target_fps > 0.0 && fps < self.target_fps * 0.5
                || self.target_bitrate_kbps > 0.0 && kbps < self.target_bitrate_kbps * 0.5)
        {
            // Data flows, but far below the configured target: the encoder or
            // network is struggling.
            StreamHealthStatus::Warning
        } else if self.window.is_empty() && self.frames_sent > 0 {
            // Frames flowed before, but none within the sliding window: the
            // stream is stalled and the health must drop accordingly.
            StreamHealthStatus::Poor
        } else {
            StreamHealthStatus::Good
        };

        StreamStats {
            status,
            frames_sent: self.frames_sent,
            frames_dropped: self.frames_dropped,
            bytes_sent: self.bytes_sent,
            kbps,
            fps,
            uptime_secs,
            dropped_ratio,
        }
    }

    /// Drop window samples older than `window_secs`.
    fn prune_window(&mut self) {
        while let Some((t, _, _)) = self.window.front() {
            if t.elapsed().as_secs_f64() > self.window_secs {
                self.window.pop_front();
            } else {
                break;
            }
        }
    }

    /// Sum the bytes/frames inside the sliding window.
    ///
    /// The elapsed time spans from the oldest retained sample until *now*, so
    /// a steady stream yields a stable rate estimate and a stream that
    /// produces fewer samples within the window is measured accordingly.
    fn window_bounds(&self) -> (f64, u64, u64) {
        if self.window.is_empty() {
            return (0.0, 0, 0);
        }
        let first = self.window.front().unwrap().0;
        let elapsed = first.elapsed().as_secs_f64();
        let bytes = self.window.iter().map(|s| s.1).sum();
        let frames = self.window.iter().map(|s| s.2).sum();
        (elapsed.max(1e-9), bytes, frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn fresh_monitor_is_connecting() {
        let mut m = StreamHealthMonitor::new(2_500.0, 30.0);
        assert_eq!(m.stats().status, StreamHealthStatus::Connecting);
        assert_eq!(m.stats().frames_sent, 0);
        assert_eq!(m.stats().dropped_ratio, 0.0);
    }

    #[test]
    fn sent_frames_accumulate() {
        let mut m = StreamHealthMonitor::new(2_500.0, 30.0);
        for _ in 0..10 {
            m.record_frame_sent(4096);
        }
        let s = m.stats();
        assert_eq!(s.frames_sent, 10);
        assert_eq!(s.bytes_sent, 40960);
        assert_eq!(s.frames_dropped, 0);
    }

    #[test]
    fn dropped_frames_increase_ratio_and_health() {
        let mut m = StreamHealthMonitor::new(2_500.0, 30.0);
        for _ in 0..100 {
            m.record_frame_sent(4096);
        }
        for _ in 0..10 {
            m.record_frame_dropped();
        }
        let s = m.stats();
        assert_eq!(s.frames_dropped, 10);
        assert!((s.dropped_ratio - 0.0909).abs() < 0.001);
        assert_eq!(s.status, StreamHealthStatus::Poor);
    }

    #[test]
    fn low_dropped_ratio_warns() {
        let mut m = StreamHealthMonitor::new(2_500.0, 30.0);
        for _ in 0..100 {
            m.record_frame_sent(4096);
        }
        for _ in 0..2 {
            m.record_frame_dropped();
        }
        let s = m.stats();
        assert_eq!(s.status, StreamHealthStatus::Warning);
    }

    #[test]
    fn good_stream_stays_good() {
        let mut m = StreamHealthMonitor::new(2_500.0, 30.0);
        for _ in 0..300 {
            m.record_frame_sent(4096);
        }
        assert_eq!(m.stats().status, StreamHealthStatus::Good);
    }

    #[test]
    fn reset_clears_counters() {
        let mut m = StreamHealthMonitor::new(2_500.0, 30.0);
        for _ in 0..100 {
            m.record_frame_sent(4096);
        }
        m.reset();
        let s = m.stats();
        assert_eq!(s.frames_sent, 0);
        assert_eq!(s.bytes_sent, 0);
        assert_eq!(s.frames_dropped, 0);
    }

    #[test]
    fn bitrate_estimate_is_positive() {
        let mut m = StreamHealthMonitor::new(2_500.0, 30.0);
        // Simulate ~2.5 Mbit/s over one second.
        let bytes_per_frame = 2500 * 1000 / 8 / 30;
        for _ in 0..30 {
            m.record_frame_sent(bytes_per_frame);
            thread::sleep(Duration::from_millis(33));
        }
        let s = m.stats();
        assert!(s.kbps > 0.0, "kbps should be positive");
        assert!(s.fps > 0.0, "fps should be positive");
        // With 5s window and only ~1s of data the estimate may be lower, but
        // it must clearly indicate active data flow.
        assert!(s.frames_sent >= 30);
    }

    #[test]
    fn low_bitrate_warns_while_still_flowing() {
        let mut m = StreamHealthMonitor::with_window(2_500.0, 30.0, 1.0);
        // Flow at ~64 kbit/s, far below the 2.5 Mbit/s target. Three frames
        // one second apart keep the window non-empty; the low-bitrate branch
        // must flag Warning.
        for _ in 0..3 {
            m.record_frame_sent(1024);
            thread::sleep(Duration::from_millis(500));
        }
        let s = m.stats();
        assert_eq!(s.status, StreamHealthStatus::Warning);
        assert!(s.kbps < 2_500.0 / 2.0, "bitrate should be below target/2");
    }

    #[test]
    fn stalled_stream_reports_poor() {
        let mut m = StreamHealthMonitor::with_window(2_500.0, 30.0, 0.2);
        for _ in 0..30 {
            m.record_frame_sent(4096);
        }
        assert_eq!(m.stats().status, StreamHealthStatus::Good);

        // No frames within the (short) window => stream is considered stalled.
        thread::sleep(Duration::from_millis(300));
        let s = m.stats();
        assert!(s.frames_sent > 0, "frames must have been recorded");
        assert_eq!(s.status, StreamHealthStatus::Poor);
    }

    #[test]
    fn offline_stats_have_offline_status() {
        let s = StreamStats::offline();
        assert_eq!(s.status, StreamHealthStatus::Offline);
        assert_eq!(s.frames_sent, 0);
        assert_eq!(s.kbps, 0.0);
    }
}
