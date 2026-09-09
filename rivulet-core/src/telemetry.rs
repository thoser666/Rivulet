//! Opt-in, privacy-friendly usage telemetry.
//!
//! This module only *classifies* events and hands a bounded batch to an
//! optional [`TelemetrySink`]. It never transmits anything by itself, and the
//! shipped build wires **no** network sink: while telemetry is disabled (the
//! default) nothing is recorded at all, and even while enabled a batch only
//! reaches whatever sink the host application installed.
//!
//! All payload fields are numeric codes, enums or booleans. Free-form text
//! (window titles, paths, URLs, stream keys, usernames) is deliberately not
//! part of the event model, so a future sender cannot leak user data even if
//! it only serializes a batch as-is ([`TelemetryBatch`] is JSON round-trip
//! safe and the redaction test pins that the serialized form carries no
//! free-form text).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Error categories the app already distinguishes; emitted with
/// [`TelemetryEvent::RecordingError`]. Serialized as a short snake_case code,
/// never as a free-form message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryErrorKind {
    Engine,
    Capture,
    Output,
    Io,
    Permission,
    #[default]
    Unknown,
}

/// A classified usage event. Only deterministic, identifier-free data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TelemetryEvent {
    /// Fresh application start (reported once per session, only when opted in).
    Startup,
    /// A recording session ended normally (`healthy`) or with an error.
    RecordingStop { duration_secs: u32, healthy: bool },
    /// A recording failed to start or ended with a captured error category.
    RecordingError { error: TelemetryErrorKind },
    /// The streamer switched the active scene.
    SceneSwitch,
    /// A chat connection attempt ended in the given state.
    ChatConnect { ok: bool },
}

/// One flushed batch handed to a [`TelemetrySink`]. Contains only bounded,
/// identifier-free events plus the compile-time app version and platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetryBatch {
    /// Stable application version at collection time.
    pub app_version: String,
    /// `cfg!`-resolved platform code (`"windows"`, `"linux"`, `"macos"` or
    /// `"other"`).
    pub platform: String,
    /// Events collected since the previous flush, in order.
    pub events: Vec<TelemetryEvent>,
}

/// Receives flushed telemetry batches. The shipped build installs no sender;
/// a future, separately-reviewed transport can be plugged in here.
pub trait TelemetrySink {
    fn submit(&mut self, batch: &TelemetryBatch);
}

impl<F> TelemetrySink for F
where
    F: FnMut(&TelemetryBatch),
{
    fn submit(&mut self, batch: &TelemetryBatch) {
        self(batch)
    }
}

/// Bounded, deterministic event collector.
///
/// - Disabled by default; [`Self::set_enabled`] is the only way to start
///   capturing, and disabling clears any pending events.
/// - [`Self::record`] is a no-op while disabled.
/// - Pending events are flushed automatically once `auto_flush_at` is reached
///   so an enabled reporter can never grow without bound. [`Self::flush`]
///   hands the batch to the installed sink (if any) and always clears the
///   queue.
pub struct TelemetryReporter {
    enabled: bool,
    pending: Vec<TelemetryEvent>,
    auto_flush_at: usize,
    sink: Option<Box<dyn TelemetrySink>>,
}

/// Default safety valve: flush at most 128 events before submitting a batch.
pub const DEFAULT_AUTO_FLUSH_AT: usize = 128;

impl Default for TelemetryReporter {
    fn default() -> Self {
        Self {
            enabled: false,
            pending: Vec::new(),
            auto_flush_at: DEFAULT_AUTO_FLUSH_AT,
            sink: None,
        }
    }
}

impl TelemetryReporter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether capturing is currently active (mirrors the persisted opt-in).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Turn capturing on/off. Disabling clears all pending events so nothing
    /// recorded while opted in survives an opt-out.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.pending.clear();
        }
    }

    /// Override the automatic-flush threshold (tests only).
    pub fn set_auto_flush_at(&mut self, threshold: usize) {
        self.auto_flush_at = threshold;
    }

    /// Install the sink that receives flushed batches. The shipped build sets
    /// no sink; only a separately-reviewed transport should do so.
    pub fn set_sink(&mut self, sink: Box<dyn TelemetrySink>) {
        self.sink = Some(sink);
    }

    /// Collect one event. A no-op while disabled.
    pub fn record(&mut self, event: TelemetryEvent) {
        if !self.enabled {
            return;
        }
        self.pending.push(event);
        if self.pending.len() >= self.auto_flush_at {
            self.flush();
        }
    }

    /// Number of collected events not yet handed to the sink (tests/diagnostics).
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Submit the current batch to the installed sink (when present) and
    /// always clear the queue. Empty batches are skipped.
    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let batch = TelemetryBatch {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: platform_code().to_string(),
            events: std::mem::take(&mut self.pending),
        };
        if let Some(sink) = &mut self.sink {
            sink.submit(&batch);
        }
    }
}

impl fmt::Debug for TelemetryReporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelemetryReporter")
            .field("enabled", &self.enabled)
            .field("pending_len", &self.pending.len())
            .field("sink_installed", &self.sink.is_some())
            .finish_non_exhaustive()
    }
}

/// Compile-time platform code for telemetry batches (`"windows"`, `"linux"`,
/// `"macos"` or `"other"`).
pub const fn platform_code() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn collecting_sink(captured: Rc<RefCell<Vec<TelemetryBatch>>>) -> Box<dyn TelemetrySink> {
        Box::new(move |b: &TelemetryBatch| captured.borrow_mut().push(b.clone()))
    }

    fn taken_batches(captured: &Rc<RefCell<Vec<TelemetryBatch>>>) -> Vec<TelemetryBatch> {
        captured.borrow().clone()
    }

    #[test]
    fn disabled_reporter_ignores_every_event() {
        let mut reporter = TelemetryReporter::new();
        assert!(!reporter.enabled(), "telemetry must default to off");
        reporter.record(TelemetryEvent::Startup);
        reporter.record(TelemetryEvent::SceneSwitch);
        assert_eq!(
            reporter.pending_len(),
            0,
            "disabled reporter must not capture"
        );
        reporter.flush();
    }

    #[test]
    fn enabled_reporter_delivers_batches_to_the_sink() {
        let captured = Rc::new(RefCell::new(Vec::new()));
        let mut reporter = TelemetryReporter::new();
        reporter.set_enabled(true);
        reporter.set_sink(collecting_sink(captured.clone()));
        reporter.record(TelemetryEvent::Startup);
        reporter.record(TelemetryEvent::RecordingStop {
            duration_secs: 42,
            healthy: true,
        });
        assert_eq!(reporter.pending_len(), 2);
        reporter.flush();
        assert_eq!(reporter.pending_len(), 0, "flush must clear the queue");
        let batches = taken_batches(&captured);
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(
            batch.events,
            vec![
                TelemetryEvent::Startup,
                TelemetryEvent::RecordingStop {
                    duration_secs: 42,
                    healthy: true,
                },
            ]
        );
        assert_eq!(batch.platform, platform_code());
        assert!(!batch.app_version.is_empty());
    }

    #[test]
    fn flush_skips_empty_batches() {
        let captured = Rc::new(RefCell::new(Vec::new()));
        let mut reporter = TelemetryReporter::new();
        reporter.set_enabled(true);
        reporter.set_sink(collecting_sink(captured.clone()));
        reporter.flush();
        assert!(
            captured.borrow().is_empty(),
            "an empty queue must never reach the sink"
        );
    }

    #[test]
    fn auto_flush_caps_pending_memory() {
        let captured = Rc::new(RefCell::new(Vec::new()));
        let mut reporter = TelemetryReporter::new();
        reporter.set_enabled(true);
        reporter.set_sink(collecting_sink(captured.clone()));
        reporter.set_auto_flush_at(3);
        reporter.record(TelemetryEvent::SceneSwitch);
        reporter.record(TelemetryEvent::SceneSwitch);
        assert_eq!(reporter.pending_len(), 2);
        reporter.record(TelemetryEvent::SceneSwitch);
        assert_eq!(
            reporter.pending_len(),
            0,
            "reaching the threshold must flush automatically"
        );
        let batches = captured.borrow();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].events.len(), 3);
    }

    #[test]
    fn disabling_clears_pending_events() {
        let captured = Rc::new(RefCell::new(Vec::new()));
        let mut reporter = TelemetryReporter::new();
        reporter.set_enabled(true);
        reporter.set_sink(collecting_sink(captured.clone()));
        reporter.record(TelemetryEvent::SceneSwitch);
        assert_eq!(reporter.pending_len(), 1);
        reporter.set_enabled(false);
        assert_eq!(
            reporter.pending_len(),
            0,
            "opting back out must drop everything collected so far"
        );
        reporter.flush();
        assert!(captured.borrow().is_empty());
    }

    #[test]
    fn batch_round_trips_through_json() {
        let batch = TelemetryBatch {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: platform_code().to_string(),
            events: vec![
                TelemetryEvent::Startup,
                TelemetryEvent::RecordingStop {
                    duration_secs: 12,
                    healthy: false,
                },
                TelemetryEvent::RecordingError {
                    error: TelemetryErrorKind::Capture,
                },
                TelemetryEvent::SceneSwitch,
                TelemetryEvent::ChatConnect { ok: true },
            ],
        };
        let json = serde_json::to_string(&batch).expect("batch must serialize");
        let decoded: TelemetryBatch =
            serde_json::from_str(&json).expect("batch must deserialize back");
        assert_eq!(decoded, batch);
    }

    #[test]
    fn serialized_batch_contains_no_freeform_text() {
        let batch = TelemetryBatch {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: platform_code().to_string(),
            events: vec![
                TelemetryEvent::RecordingStop {
                    duration_secs: 7,
                    healthy: true,
                },
                TelemetryEvent::RecordingError {
                    error: TelemetryErrorKind::Io,
                },
            ],
        };
        let json = serde_json::to_string(&batch).expect("batch must serialize");
        for forbidden in ['/', '\\', ' ', '@', '%'] {
            assert!(
                !json.contains(forbidden),
                "compact serialized telemetry must not contain {forbidden:?}: {json}"
            );
        }
        assert!(
            json.is_ascii(),
            "serialized payload must stay ASCII: {json}"
        );
    }

    #[test]
    fn event_kinds_copy_and_compare_deterministically() {
        let a = TelemetryEvent::ChatConnect { ok: false };
        let b = a;
        assert_eq!(a, b);
        assert_ne!(
            a,
            TelemetryEvent::ChatConnect { ok: true },
            "event payloads must compare by value"
        );
    }
}
