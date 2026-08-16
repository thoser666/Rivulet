//! Recording performance metrics.
//!
//! While a recording is active, the engine measures how many frames per second
//! are actually pushed into the pipeline, how loaded the encoder is, and how
//! much data has already been written to the output file.
//!
//! The encoder load is the fraction of wall-clock time the encoder spends
//! processing frames: at 30 FPS a frame that takes 16 ms to encode yields
//! roughly 50% load, while 40 ms per frame already overloads the encoder
//! (>100%). Like the stream-health monitor, all rates are computed over a
//! sliding time window so short bursts average out.
//!
//! The time source is injectable so tests can simulate elapsed time
//! deterministically instead of relying on wall-clock sleeps (which are flaky
//! on loaded CI runners).

use std::collections::VecDeque;
use std::time::Instant;

/// A snapshot of the current recording performance metrics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecordingMetrics {
    /// Measured frames per second over the sliding window.
    pub fps: f64,
    /// Encoder load as a percentage of the frame budget. Values above 100.0
    /// mean the encoder cannot keep up with the configured frame rate.
    pub encode_load_percent: f64,
    /// Bytes written to the output file so far.
    pub file_size_bytes: u64,
    /// Total frames pushed into the pipeline since the recording started.
    pub frames_captured: u64,
    /// Recording uptime in seconds.
    pub uptime_secs: u64,
}

/// Tracks recording performance over a sliding time window.
pub struct RecordingStatsMonitor {
    /// Length of the sliding window in seconds.
    window_secs: f64,
    frames_captured: u64,
    file_size_bytes: u64,
    /// Sliding window samples `(elapsed_secs, frames, encode_time_secs)` where
    /// the elapsed value is the clock reading at the time the sample was added.
    window: VecDeque<(f64, u64, f64)>,
    /// Clock source returning seconds since the monitor was created. Defaults
    /// to wall-clock; tests inject a deterministic clock.
    clock: Box<dyn Fn() -> f64 + Send + Sync>,
}

impl RecordingStatsMonitor {
    /// Create a monitor with a default sliding window of 5 seconds.
    pub fn new() -> Self {
        Self::with_window(5.0)
    }

    /// Like [`Self::new`], but with a custom sliding-window length. A shorter
    /// window reacts faster but is noisier; used by tests and embedded setups.
    pub fn with_window(window_secs: f64) -> Self {
        let start = Instant::now();
        Self {
            window_secs,
            frames_captured: 0,
            file_size_bytes: 0,
            window: VecDeque::new(),
            clock: Box::new(move || start.elapsed().as_secs_f64()),
        }
    }

    /// Create a monitor with an injected clock (tests only). The clock returns
    /// seconds since the monitor was created.
    #[cfg(test)]
    fn with_clock(window_secs: f64, clock: impl Fn() -> f64 + Send + Sync + 'static) -> Self {
        Self {
            window_secs,
            frames_captured: 0,
            file_size_bytes: 0,
            window: VecDeque::new(),
            clock: Box::new(clock),
        }
    }

    /// Record one frame pushed into the pipeline.
    ///
    /// `encode_time_secs` is the wall-clock time the encoder spent on this
    /// frame (measured via pad probes). Pass `0.0` when the duration is not
    /// known; the encoder load then stays at zero.
    pub fn record_frame(&mut self, encode_time_secs: f64) {
        self.frames_captured += 1;
        self.window.push_back(((self.clock)(), 1, encode_time_secs));
        self.prune_window();
    }

    /// Add an encode-time sample to the most recent frame.
    ///
    /// Called from the encoder src-pad probe, right after the matching frame
    /// was recorded. Splitting the two calls keeps the probe simple while the
    /// frame count and the encoder load stay aligned on the same window.
    pub fn record_encode_time(&mut self, encode_time_secs: f64) {
        match self.window.back_mut() {
            Some((_, _, encode)) => *encode += encode_time_secs,
            None => self.window.push_back(((self.clock)(), 0, encode_time_secs)),
        }
        self.prune_window();
    }

    /// Add bytes written to the output file (called by the filesink probe).
    pub fn add_file_bytes(&mut self, bytes: u64) {
        self.file_size_bytes = self.file_size_bytes.saturating_add(bytes);
    }

    /// Reset all counters (e.g. when a new recording starts).
    pub fn reset(&mut self) {
        let window_secs = self.window_secs;
        *self = Self::with_window(window_secs);
    }

    /// Current performance metrics.
    ///
    /// Prunes the sliding window first, so callers always observe the current
    /// state even if no frames were pushed recently.
    pub fn stats(&mut self) -> RecordingMetrics {
        self.prune_window();
        let uptime_secs = (self.clock)() as u64;
        let (elapsed, frames, encode_secs) = self.window_bounds();
        let fps = if elapsed > 0.0 {
            frames as f64 / elapsed
        } else {
            0.0
        };
        let encode_load_percent = if elapsed > 0.0 {
            encode_secs / elapsed * 100.0
        } else {
            0.0
        };
        RecordingMetrics {
            fps,
            encode_load_percent,
            file_size_bytes: self.file_size_bytes,
            frames_captured: self.frames_captured,
            uptime_secs,
        }
    }

    /// Drop window samples older than `window_secs`.
    fn prune_window(&mut self) {
        let now = (self.clock)();
        while let Some((t, _, _)) = self.window.front() {
            if now - *t > self.window_secs {
                self.window.pop_front();
            } else {
                break;
            }
        }
    }

    /// Sum the frames and encode times inside the sliding window.
    ///
    /// The elapsed time spans from the oldest retained sample until *now*, so
    /// a steady stream yields a stable rate and a recording that produces
    /// fewer samples within the window is measured accordingly.
    fn window_bounds(&self) -> (f64, u64, f64) {
        if self.window.is_empty() {
            return (0.0, 0, 0.0);
        }
        let first = self.window.front().unwrap().0;
        let elapsed = ((self.clock)() - first).max(1e-9);
        let frames = self.window.iter().map(|s| s.1).sum();
        let encode = self.window.iter().map(|s| s.2).sum();
        (elapsed, frames, encode)
    }
}

impl Default for RecordingStatsMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A manual clock the tests can advance by a fixed number of seconds.
    #[derive(Clone, Default)]
    struct TestClock {
        elapsed: Arc<Mutex<f64>>,
    }

    impl TestClock {
        fn new() -> Self {
            Self::default()
        }

        fn advance(&self, secs: f64) {
            *self.elapsed.lock().unwrap() += secs;
        }

        fn closure(&self) -> impl Fn() -> f64 + Send + Sync + 'static {
            let elapsed = Arc::clone(&self.elapsed);
            move || *elapsed.lock().unwrap()
        }
    }

    #[test]
    fn fresh_monitor_reports_zero_metrics() {
        let mut m = RecordingStatsMonitor::new();
        let s = m.stats();
        assert_eq!(s.fps, 0.0);
        assert_eq!(s.encode_load_percent, 0.0);
        assert_eq!(s.file_size_bytes, 0);
        assert_eq!(s.frames_captured, 0);
        assert_eq!(s.uptime_secs, 0);
    }

    #[test]
    fn frames_accumulate_and_fps_is_estimated() {
        let clock = TestClock::new();
        let mut m = RecordingStatsMonitor::with_clock(5.0, clock.closure());
        for _ in 0..30 {
            m.record_frame(0.0);
            clock.advance(1.0 / 30.0);
        }
        let s = m.stats();
        assert!(s.frames_captured >= 30);
        assert!(s.fps > 0.0, "fps should be positive");
        // 30 frames over exactly 1s -> ~30 FPS.
        assert!(s.fps > 20.0 && s.fps < 40.0, "fps = {}", s.fps);
    }

    #[test]
    fn encode_load_reflects_frame_budget() {
        let clock = TestClock::new();
        let mut m = RecordingStatsMonitor::with_clock(5.0, clock.closure());
        // 20 frames, 40 ms apart, each consuming 20 ms of encode time:
        // encoder busy exactly half the time -> ~50% load.
        for _ in 0..20 {
            m.record_frame(0.020);
            clock.advance(0.040);
        }
        let s = m.stats();
        assert!(
            (s.encode_load_percent - 50.0).abs() < 10.0,
            "load = {}%",
            s.encode_load_percent
        );
    }

    #[test]
    fn encode_load_exceeds_100_when_overloaded() {
        let clock = TestClock::new();
        let mut m = RecordingStatsMonitor::with_clock(5.0, clock.closure());
        // Encoder consumes more than the frame period -> clearly overloaded.
        for _ in 0..20 {
            m.record_frame(0.050);
            clock.advance(0.030);
        }
        let s = m.stats();
        assert!(
            s.encode_load_percent > 100.0,
            "load = {}%",
            s.encode_load_percent
        );
    }

    #[test]
    fn encode_load_is_zero_without_encode_times() {
        let clock = TestClock::new();
        let mut m = RecordingStatsMonitor::with_clock(5.0, clock.closure());
        for _ in 0..10 {
            m.record_frame(0.0);
            clock.advance(0.020);
        }
        let s = m.stats();
        assert!(s.fps > 0.0);
        assert_eq!(s.encode_load_percent, 0.0);
    }

    #[test]
    fn encode_time_samples_merge_into_latest_frame() {
        let mut m = RecordingStatsMonitor::new();
        m.record_frame(0.0);
        m.record_encode_time(0.010);
        m.record_encode_time(0.005);
        // Both samples belong to the single recorded frame.
        let encode = m.window.back().unwrap().2;
        assert!((encode - 0.015).abs() < 1e-12, "encode = {encode}");
    }

    #[test]
    fn file_bytes_accumulate() {
        let mut m = RecordingStatsMonitor::new();
        m.add_file_bytes(1000);
        m.add_file_bytes(4000);
        assert_eq!(m.stats().file_size_bytes, 5000);
    }

    #[test]
    fn reset_clears_counters() {
        let mut m = RecordingStatsMonitor::new();
        m.record_frame(0.010);
        m.add_file_bytes(1234);
        m.reset();
        let s = m.stats();
        assert_eq!(s.frames_captured, 0);
        assert_eq!(s.file_size_bytes, 0);
        assert_eq!(s.encode_load_percent, 0.0);
    }

    #[test]
    fn stalled_recording_drops_fps_to_zero() {
        let clock = TestClock::new();
        let mut m = RecordingStatsMonitor::with_clock(0.2, clock.closure());
        for _ in 0..30 {
            m.record_frame(0.001);
        }
        assert!(m.stats().fps > 0.0);
        clock.advance(0.3);
        let s = m.stats();
        assert_eq!(s.fps, 0.0);
        assert_eq!(s.encode_load_percent, 0.0);
        assert!(s.frames_captured > 0);
    }
}
