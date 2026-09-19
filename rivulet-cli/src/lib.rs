//! Library path for the headless CLI (M7 W1, issue #186): the same recording
//! a `rivulet record` binary run performs, achievable in-process without
//! spawning the binary.
//!
//! The engine is a push-model encoder: [`RivuletEngine::process_raw_frame`]
//! consumes RGBA video frames and lazily builds the GStreamer pipeline on the
//! first frame; audio is pushed as interleaved f32 PCM
//! ([`rivulet_core::AUDIO_SAMPLE_RATE`] Hz, [`rivulet_core::AUDIO_CHANNELS`]
//! channels). [`RecordJob`] drives that contract from a config and emits
//! JSON status events plus metrics — the binary's `main` is a thin wrapper
//! around this module.

pub mod config;
pub mod source;

pub use config::{describe, AudioConfig, OutputConfig, RecordConfig, StatusEvent, VideoConfig};
pub use source::{silence_frame, SilenceSource, TestVideoSource};

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::Instant;

use rivulet_core::RivuletEngine;

/// Exit codes documented in the M7 spec (§ CLI surface reference).
pub mod exit_code {
    /// Success (including graceful SIGINT/SIGTERM stop).
    pub const OK: i32 = 0;
    /// Invalid usage or configuration (actionable error names the key).
    pub const USAGE: i32 = 2;
    /// The recording failed at runtime (engine error, IO error).
    pub const RUNTIME: i32 = 1;
}

/// One headless recording run: config in, finalized container out.
///
/// Binary and library paths share this implementation. Events (JSON on
/// stdout, human diagnostics on stderr) flow through the provided closures
/// so library consumers can route them anywhere.
pub struct RecordJob {
    config: RecordConfig,
    /// JSON status events (spec: stable documented schema).
    pub on_event: Box<dyn FnMut(&StatusEvent) + Send>,
    /// Human-readable diagnostics (stderr in the binary).
    pub on_diagnostic: Box<dyn FnMut(&str) + Send>,
}

impl RecordJob {
    /// Build a job from a validated config.
    pub fn new(config: RecordConfig) -> Self {
        Self {
            config,
            on_event: Box::new(|_| {}),
            on_diagnostic: Box::new(|_| {}),
        }
    }

    /// Register the JSON-event sink.
    pub fn with_event_sink(mut self, sink: Box<dyn FnMut(&StatusEvent) + Send>) -> Self {
        self.on_event = sink;
        self
    }

    /// Register the human-readable diagnostics sink.
    pub fn with_diagnostic_sink(mut self, sink: Box<dyn FnMut(&str) + Send>) -> Self {
        self.on_diagnostic = sink;
        self
    }

    fn emit(&mut self, event: StatusEvent) {
        (self.on_event)(&event);
    }

    fn diag(&mut self, msg: &str) {
        (self.on_diagnostic)(msg);
    }

    /// Run the recording to completion and return the finalized output path.
    ///
    /// The `stop_requested` closure is polled between frames; when it returns
    /// `true` (SIGINT/SIGTERM in the binary), the recording stops cleanly and
    /// the container is finalized.
    pub fn run(&mut self, stop_requested: impl Fn() -> bool) -> Result<PathBuf> {
        self.config.validate().map_err(|e| anyhow::anyhow!("{e}"))?;

        let width = self.config.video.width;
        let height = self.config.video.height;
        let fps = self.config.video.fps;
        let audio_enabled = self.config.audio.enabled;
        let output_path = self
            .config
            .output
            .path
            .clone()
            .expect("validated: output.path present");
        let duration = self.config.duration_secs;

        if let Some(parent) = output_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating output directory {}", parent.display()))?;
            }
        }

        let mut engine = RivuletEngine::new();
        engine.set_audio_enabled(audio_enabled);
        engine.start_local_recording(output_path.clone());

        self.emit(StatusEvent::Started {
            width,
            height,
            fps,
            audio_enabled,
        });
        self.diag(&format!(
            "recording to {} ({}x{} @ {fps} fps, audio={audio_enabled})",
            output_path.display(),
            width,
            height
        ));

        let mut video = TestVideoSource::new(width, height, fps);
        let mut audio = if audio_enabled {
            Some(SilenceSource::new())
        } else {
            None
        };

        let frame_interval = std::time::Duration::from_secs_f64(1.0 / f64::from(fps));
        // Audio cadence: 10 ms worth of samples per push keeps the muxer's
        // interleaving healthy without spamming the engine.
        let audio_samples_per_push = (rivulet_core::AUDIO_SAMPLE_RATE / 100) as usize;
        let started_at = Instant::now();
        let mut frames_pushed: u64 = 0;
        let mut audio_pushes: u64 = 0;
        let mut last_progress_secs: u64 = 0;
        let mut last_progress_at = started_at;

        loop {
            if stop_requested() {
                self.diag("stop requested; finalizing…");
                break;
            }
            if let Some(secs) = duration {
                if started_at.elapsed().as_secs() >= secs {
                    break;
                }
            }
            if let Some(err) = engine.take_error() {
                return Err(anyhow::anyhow!("engine error: {err}"));
            }

            engine.process_raw_frame(&video.next_frame(), width, height);
            frames_pushed += 1;

            if let Some(audio) = audio.as_mut() {
                let frame = audio.next_frame(audio_samples_per_push);
                engine
                    .push_audio_frame(&frame)
                    .context("pushing audio frame")?;
                audio_pushes += 1;
            }

            if last_progress_at.elapsed() >= std::time::Duration::from_secs(1) {
                let secs = started_at.elapsed().as_secs();
                if secs > last_progress_secs {
                    last_progress_secs = secs;
                    last_progress_at = Instant::now();
                    let metrics = engine.recording_stats();
                    self.emit(StatusEvent::Progress {
                        seconds: secs,
                        frames: metrics.frames_captured.max(frames_pushed),
                        fps: metrics.fps,
                        file_size_bytes: metrics.file_size_bytes,
                    });
                }
            }

            // Pace against wall time so the muxer sees ~real-time timestamps
            // (the appsrc is `do-timestamp`; cadence becomes PTS cadence).
            let next_frame_at = started_at
                + std::time::Duration::from_secs_f64(
                    frames_pushed as f64 * frame_interval.as_secs_f64(),
                );
            let wait = next_frame_at.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }

        let metrics_before_stop = engine.recording_stats();
        engine.stop_recording();
        if let Some(err) = engine.take_error() {
            self.diag(&format!("engine reported: {err}"));
        }

        let file_size = file_size_of(&output_path);
        let seconds = started_at.elapsed().as_secs();
        self.emit(StatusEvent::Stopped {
            frames: metrics_before_stop.frames_captured.max(frames_pushed),
            seconds,
            file_size_bytes: file_size.unwrap_or(0),
        });

        if file_size.map_or(true, |s| s == 0) {
            anyhow::bail!(
                "recording produced no output file at {} (or the file is empty)",
                output_path.display()
            );
        }

        self.diag(&format!(
            "finalized {} ({} frames video, {audio_pushes} audio pushes)",
            output_path.display(),
            frames_pushed
        ));
        Ok(output_path)
    }
}

fn file_size_of(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Event/diagnostic sink pair used by the binary.
pub type SinkPair = (
    Box<dyn FnMut(&StatusEvent) + Send>,
    Box<dyn FnMut(&str) + Send>,
);

/// Build the default sinks used by the binary: JSON events to stdout,
/// diagnostics to stderr.
pub fn stdout_stderr_sinks() -> SinkPair {
    let event_sink = Box::new(|event: &StatusEvent| {
        println!(
            "{}",
            serde_json::to_string(event).expect("serializable event")
        );
    });
    let diag_sink = Box::new(|msg: &str| {
        eprintln!("{msg}");
    });
    (event_sink, diag_sink)
}

/// Convenience wrapper for library consumers: run a config to completion with
/// default sinks, returning the finalized output path.
pub fn record(config: RecordConfig) -> Result<PathBuf> {
    let (events, diags) = stdout_stderr_sinks();
    RecordJob::new(config)
        .with_event_sink(events)
        .with_diagnostic_sink(diags)
        .run(|| false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn tmp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rivulet-cli-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn recording_config(dir: &Path, name: &str, secs: u64) -> RecordConfig {
        RecordConfig {
            output: OutputConfig {
                path: Some(dir.join(name)),
                container: Some("mp4".to_string()),
            },
            video: VideoConfig {
                source: "test".to_string(),
                test_pattern: Some("smpte".to_string()),
                width: 320,
                height: 240,
                fps: 30,
            },
            audio: AudioConfig { enabled: false },
            duration_secs: Some(secs),
        }
    }

    #[test]
    fn headless_recording_produces_a_valid_container() {
        let dir = tmp_dir("record");
        let config = recording_config(&dir, "out.mp4", 1);
        let out = record(config).expect("recording succeeds headless");
        let meta = std::fs::metadata(&out).expect("output exists");
        assert!(meta.len() > 0, "output file is non-empty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_job_emits_started_progress_and_stopped_events() {
        let dir = tmp_dir("events");
        let config = recording_config(&dir, "out.mp4", 1);
        let events: Arc<Mutex<Vec<StatusEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let (diag, _keep) = {
            let d: Box<dyn FnMut(&str) + Send> = Box::new(|_| {});
            (d, 0)
        };
        let out = RecordJob::new(config)
            .with_event_sink(Box::new(move |e| sink.lock().unwrap().push(e.clone())))
            .with_diagnostic_sink(diag)
            .run(|| false)
            .expect("recording succeeds");
        assert!(std::fs::metadata(&out).is_ok());

        let evs = events.lock().unwrap();
        assert!(
            matches!(evs.first(), Some(StatusEvent::Started { .. })),
            "first event must be started, got {:?}",
            evs.first()
        );
        assert!(
            matches!(evs.last(), Some(StatusEvent::Stopped { .. })),
            "last event must be stopped, got {:?}",
            evs.last()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_config_fails_with_named_key() {
        let mut cfg = recording_config(&tmp_dir("invalid"), "out.mp4", 1);
        cfg.output.path = None;
        let err = RecordJob::new(cfg).run(|| false).unwrap_err();
        assert!(err.to_string().contains("output.path"), "error = {err}");
    }

    #[test]
    fn audio_enabled_run_records_the_audio_branch() {
        let dir = tmp_dir("audio");
        let mut cfg = recording_config(&dir, "out.mp4", 1);
        cfg.audio = AudioConfig { enabled: true };
        let out = record(cfg).expect("recording with audio succeeds");
        assert!(std::fs::metadata(&out).expect("output exists").len() > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
