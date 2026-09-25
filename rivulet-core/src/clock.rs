//! Injectable engine clock (M7 W2a, issue #187 — deterministic pipeline).
//!
//! The engine is a push-model encoder: historically every appsrc ran with
//! `is-live=true do-timestamp=true`, so PTS came from the wall clock at push
//! time — reproducible only by real-time pacing. This module makes the time
//! source injectable: a [`SystemClock`] preserves today's behavior, a
//! [`VirtualClock`] makes a run time-scriptable so identical inputs produce
//! identical PTS sequences regardless of real-time speed (the reproducible-run
//! contract in `docs/m7-automation.md` § W2a / § Nondeterminism inventory).
//!
//! The clock only drives *video PTS stamping*; audio PTS stays derived from
//! sample counts at the fixed engine rate ([`crate::AUDIO_SAMPLE_RATE`] Hz),
//! so the audio cadence is clock-independent by construction.

use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Which clock source is driving the session.
///
/// Reported in the machine-readable run report (spec: § Nondeterminism
/// inventory — "Engine clock … deterministic under the virtual clock").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockMode {
    /// Wall-clock pacing (engine default; PTS = push time via
    /// `do-timestamp`).
    System,
    /// Scripted virtual time; PTS advance is decided by the caller
    /// (`advance_ns` / `step_frames` / `hold`), not by real time.
    Virtual,
}

impl ClockMode {
    /// Lower-case name used in machine-readable reports.
    pub fn as_str(self) -> &'static str {
        match self {
            ClockMode::System => "system",
            ClockMode::Virtual => "virtual",
        }
    }
}

/// Injected time source for engine PTS stamping.
///
/// Implementations must return monotonically non-decreasing values in
/// nanoseconds of *engine run time* (time since the session started). The
/// engine captures the session base time and stamps buffers at
/// `base + now_ns()`.
pub trait EngineClock: Send + Sync {
    /// Current engine run time in nanoseconds. Monotonic: two calls must
    /// satisfy `t2 >= t1` when ordered in time.
    fn now_ns(&self) -> u64;

    /// Which mode this clock reports itself as (run-report surface).
    fn mode(&self) -> ClockMode;

    /// Downcast support so shared handles can expose their concrete type
    /// (the engine's [`VirtualClock`] driver surface).
    fn as_any(&self) -> &dyn std::any::Any;
}

/// The default clock: wall-clock pacing, exactly today's behavior.
///
/// Time is measured from the clock's own creation instant; the engine turns
/// that into engine run time by subtracting the session start reading, so
/// epoch choice never leaks into PTS.
#[derive(Debug)]
pub struct SystemClock {
    epoch: Instant,
}

impl SystemClock {
    /// Create a system clock whose run time starts now.
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineClock for SystemClock {
    fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    fn mode(&self) -> ClockMode {
        ClockMode::System
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A scriptable clock for tests and rendering.
///
/// Time does not move on its own — the driver decides when and by how much
/// the clock advances, so a run is fully time-scriptable (advance, hold,
/// step — spec § W2a) and the PTS cadence is exact rather than paced by
/// real time.
///
/// The clock is monotonic by construction: every mutator only ever moves
/// time forward (saturating at `u64::MAX`), so the engine's non-decreasing
/// PTS contract holds without extra bookkeeping.
#[derive(Debug, Default, Clone)]
pub struct VirtualClock {
    state: Arc<Mutex<u64>>,
}

impl VirtualClock {
    /// Create a virtual clock at run time zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the clock by exactly `ns` nanoseconds.
    pub fn advance_ns(&self, ns: u64) {
        let mut now = self.state.lock().unwrap_or_else(|e| e.into_inner());
        *now = now.saturating_add(ns);
    }

    /// Advance the clock by exactly `frames` frame intervals (`1/fps`
    /// seconds each).
    ///
    /// The total is computed in one rational division (`1e9 * frames * den /
    /// num` in `u128`) so repeated stepping produces an exact, deterministic
    /// PTS cadence across machines — no f64 drift, and no accumulated
    /// per-step truncation error.
    pub fn step_frames(&self, frames: u64, fps: (u32, u32)) {
        let (num, den) = (fps.0.max(1) as u128, fps.1.max(1) as u128);
        let total_ns = (1_000_000_000u128 * frames as u128 * den / num) as u64;
        self.advance_ns(total_ns);
    }

    /// Freeze the clock: subsequent reads return the current value until the
    /// next advance. Held time is not dropped — buffers stamped during a
    /// hold all carry the held timestamp.
    pub fn hold(&self) {
        // Holding is the absence of advancing; the method exists so callers
        // can express intent and so the API surface from the spec (advance,
        // hold, step) is complete.
    }

    /// Current virtual run time in nanoseconds.
    pub fn peek_ns(&self) -> u64 {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl EngineClock for VirtualClock {
    fn now_ns(&self) -> u64 {
        self.peek_ns()
    }

    fn mode(&self) -> ClockMode {
        ClockMode::Virtual
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Exact frame interval in nanoseconds for a rational FPS.
///
/// `interval_ns = 1_000_000_000 * den / num` for `num/den` frames per second
/// (e.g. 33_366_666 ns for NTSC 30000/1001), computed in `u128` with the
/// degenerate rates clamped to a valid interval instead of returning 0.
pub fn frame_interval_ns(fps: (u32, u32)) -> u64 {
    let (num, den) = (fps.0.max(1) as u128, fps.1.max(1) as u128);
    (1_000_000_000u128 * den / num) as u64
}

/// Shared clock handle the engine holds. Cloned around freely; the mode is
/// captured with the value.
#[derive(Clone)]
pub struct SharedClock(Arc<dyn EngineClock>);

impl SharedClock {
    /// Wrap any clock implementation.
    pub fn new(clock: Arc<dyn EngineClock>) -> Self {
        Self(clock)
    }

    /// The system clock (engine default).
    pub fn system() -> Self {
        Self::new(Arc::new(SystemClock::new()))
    }

    /// The wrapped clock as a [`VirtualClock`], when one is configured.
    /// Driver surface for tests and rendering: advance/step/hold between
    /// frame pushes to script the session time.
    pub fn as_virtual(&self) -> Option<&VirtualClock> {
        self.0.as_any().downcast_ref::<VirtualClock>()
    }
}

impl std::fmt::Debug for SharedClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedClock")
            .field("mode", &self.0.mode())
            .finish()
    }
}

impl EngineClock for SharedClock {
    fn now_ns(&self) -> u64 {
        self.0.now_ns()
    }

    fn mode(&self) -> ClockMode {
        self.0.mode()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self.0.as_any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_mode_is_system() {
        assert_eq!(SystemClock::new().mode(), ClockMode::System);
    }

    #[test]
    fn system_clock_now_is_monotonic() {
        let clock = SystemClock::new();
        let a = clock.now_ns();
        let b = clock.now_ns();
        assert!(b >= a, "system clock must be monotonic ({a} -> {b})");
    }

    #[test]
    fn virtual_clock_starts_at_zero() {
        assert_eq!(VirtualClock::new().peek_ns(), 0);
        assert_eq!(VirtualClock::new().now_ns(), 0);
    }

    #[test]
    fn virtual_clock_mode_is_virtual() {
        assert_eq!(VirtualClock::new().mode(), ClockMode::Virtual);
    }

    #[test]
    fn virtual_clock_advance_is_exact() {
        let clock = VirtualClock::new();
        clock.advance_ns(1_000);
        clock.advance_ns(250);
        assert_eq!(clock.peek_ns(), 1_250);
    }

    #[test]
    fn virtual_clock_step_frames_is_exactly_one_frame_interval() {
        let clock = VirtualClock::new();
        clock.step_frames(1, (30, 1));
        assert_eq!(clock.peek_ns(), 33_333_333); // 1e9 / 30, truncated

        let clock = VirtualClock::new();
        clock.step_frames(3, (60, 1));
        assert_eq!(clock.peek_ns(), 50_000_000); // 3 * (1e9 / 60)
    }

    #[test]
    fn virtual_clock_step_frames_handles_fractional_fps() {
        // 30000/1001 NTSC: one frame = 1e9 * 1001 / 30000 ns.
        let interval = frame_interval_ns((30000, 1001));
        assert_eq!(interval, 33_366_666);

        let clock = VirtualClock::new();
        clock.step_frames(1001, (30000, 1001));
        // One rational division over all frames, not accumulated per-step
        // rounding: 1e9 * 1001 * 1001 / 30000 ns.
        assert_eq!(clock.peek_ns(), 33_400_033_333);
    }

    #[test]
    fn virtual_clock_hold_freezes_time() {
        let clock = VirtualClock::new();
        clock.advance_ns(5_000);
        clock.hold();
        assert_eq!(clock.peek_ns(), 5_000);
        assert_eq!(clock.now_ns(), 5_000, "hold keeps reporting held time");
        clock.advance_ns(1);
        assert_eq!(clock.peek_ns(), 5_001);
    }

    #[test]
    fn virtual_clock_never_goes_backwards() {
        let clock = VirtualClock::new();
        clock.advance_ns(10_000);
        let before = clock.now_ns();
        clock.advance_ns(0);
        assert_eq!(clock.now_ns(), before);
        // No mutator exists that could decrease time; saturating advance is
        // the only direction.
        clock.advance_ns(u64::MAX);
        assert_eq!(clock.now_ns(), u64::MAX);
    }

    #[test]
    fn virtual_clock_advance_saturates_instead_of_overflowing() {
        let clock = VirtualClock::new();
        clock.advance_ns(u64::MAX);
        clock.advance_ns(1);
        assert_eq!(clock.peek_ns(), u64::MAX);
    }

    #[test]
    fn shared_clock_delegates_mode_and_time() {
        let virtual_clock = VirtualClock::new();
        let shared = SharedClock::new(Arc::new(virtual_clock.clone()));
        assert_eq!(shared.mode(), ClockMode::Virtual);
        assert_eq!(shared.now_ns(), 0);
        virtual_clock.advance_ns(42);
        assert_eq!(shared.now_ns(), 42);

        let system = SharedClock::system();
        assert_eq!(system.mode(), ClockMode::System);
    }
    #[test]
    fn shared_clock_debug_reports_mode() {
        let debug = format!("{:?}", SharedClock::system());
        assert!(
            debug.contains("System"),
            "debug must expose the mode: {debug}"
        );
    }

    #[test]
    fn frame_interval_ns_guards_against_zero() {
        // Degenerate rates clamp to a valid interval instead of returning 0.
        assert_eq!(frame_interval_ns((0, 1)), 1_000_000_000);
        assert_eq!(frame_interval_ns((30, 0)), 33_333_333);
    }

    #[test]
    fn clock_mode_strings_are_stable() {
        // Machine-readable run-report surface; renaming breaks consumers.
        assert_eq!(ClockMode::System.as_str(), "system");
        assert_eq!(ClockMode::Virtual.as_str(), "virtual");
    }
}
