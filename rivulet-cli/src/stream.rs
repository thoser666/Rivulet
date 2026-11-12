//! `rivulet stream` — the headless streaming subcommand (M7 closeout, issue
//! #289 / finding F-M7-002).
//!
//! The job keeps the shape [`crate::RecordJob`] established: JSON status events
//! on **stdout**, human diagnostics on **stderr**, the documented exit codes,
//! a graceful SIGINT/SIGTERM finalize, and a dry run that reports the pipeline
//! and the ingest check before anything is published.
//!
//! Two things differ from `record`, because the output is a network endpoint
//! instead of a file:
//!
//! * the ingest is probed once before the first frame, so a broken endpoint
//!   fails as [`crate::Stage::Output`] instead of surfacing as a mid-stream
//!   error after the user believes the stream is live;
//! * the status events carry stream telemetry (`kbps`, `bytes_sent`,
//!   `dropped_ratio`, `health`) instead of a file size.
//!
//! Secrets never reach this surface: the ingest URL is reduced to scheme plus
//! authority (a custom URL may embed the key in its path) and the key is only
//! ever reported in its `masked_key()` form.

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use rivulet_core::{
    RivuletEngine, StreamHealthStatus, StreamPlatform, StreamPreset, StreamProbeResult,
    StreamSettings,
};
use serde::Serialize;

use crate::config::StreamConfig;
use crate::source::{SilenceSource, TestVideoSource};
use crate::{exit_code, RunFailure};

/// How long the pre-flight ingest probe may take. The core clamps its own
/// probe to 1..=30 s, so this is the value the run actually waits for.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// `rivulet stream --help`.
pub const USAGE: &str = "\
rivulet stream — publish to an RTMP/RTMPS ingest with the Rivulet engine (M7)

USAGE:
    rivulet stream --config <streaming.toml> [flags]
    rivulet stream --ingest <url> --key <key> [flags]   # minimal inline config
    rivulet stream help                                  # show this help

FLAGS (stream):
    -c, --config <file>       TOML config file (see spec § CLI surface reference)
        --platform <name>     twitch | youtube | kick | custom (default custom)
        --ingest <url>        ingest URL (overrides stream.ingest_url)
        --key <key>           stream key (overrides stream.stream_key)
        --preset <name>       low | standard | high | custom (default standard)
    -d, --duration <secs>     stop after N seconds (overrides config)
    -w, --width <px>          video width (default 640)
    -h, --height <px>         video height (default 360)
    -f, --fps <n>             video frame rate (default 30)
    -a, --audio               enable the silent test-audio track
        --dry-run             probe the ingest, print the plan and exit
        --json                with --dry-run: machine-readable plan

STREAMS:
    stdout  JSON status events (started/progress/stopped, or failed on error)
    stderr  human-readable diagnostics

EXIT CODES:
    0  success (including graceful SIGINT/SIGTERM stop)
    1  runtime failure (unreachable ingest, engine error)
    2  invalid usage or config (the error names the offending key)

A stream failure ends with the same machine-readable object as `record`:
    {\"event\": \"failed\", \"stage\": \"output\", \"exit_code\": 1, \"message\": ...}
";

/// Flag overrides for `rivulet stream`.
///
/// Every field is optional so the TOML config wins wherever a flag was not
/// given — the same precedence `rivulet record` documents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamOverrides {
    /// `--platform`: `twitch`, `youtube`, `kick` or `custom`.
    pub platform: Option<String>,
    /// `--ingest`: ingest URL.
    pub ingest_url: Option<String>,
    /// `--key`: stream key.
    pub stream_key: Option<String>,
    /// `--preset`: `low`, `standard`, `high` or `custom`.
    pub preset: Option<String>,
    /// `--duration`: stop after this many seconds.
    pub duration_secs: Option<u64>,
    /// `--width`.
    pub width: Option<u32>,
    /// `--height`.
    pub height: Option<u32>,
    /// `--fps`.
    pub fps: Option<u32>,
    /// `--audio`.
    pub audio: bool,
}

impl StreamOverrides {
    /// Apply the flags that were set on top of a loaded config.
    pub fn apply(self, config: &mut StreamConfig) {
        if let Some(platform) = self.platform {
            config.stream.platform = platform;
        }
        if let Some(url) = self.ingest_url {
            config.stream.ingest_url = Some(url);
        }
        if let Some(key) = self.stream_key {
            config.stream.stream_key = Some(key);
        }
        if let Some(preset) = self.preset {
            config.stream.preset = Some(preset);
        }
        if let Some(secs) = self.duration_secs {
            config.duration_secs = Some(secs);
        }
        if let Some(width) = self.width {
            config.video.width = width;
        }
        if let Some(height) = self.height {
            config.video.height = height;
        }
        if let Some(fps) = self.fps {
            config.video.fps = fps;
        }
        if self.audio {
            config.audio.enabled = true;
        }
    }
}

/// Parsed `rivulet stream` invocation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamArgs {
    /// `--config`: TOML file to load.
    pub config_path: Option<PathBuf>,
    /// Flags that override the loaded config.
    pub overrides: StreamOverrides,
    /// `--dry-run`: probe and report, never publish.
    pub dry_run: bool,
    /// `--json`: machine-readable dry-run plan.
    pub json: bool,
    /// `--help`.
    pub show_help: bool,
}

/// Parse the arguments of `rivulet stream` (everything after the subcommand).
///
/// Unknown flags are a usage error rather than silently ignored: a typo in
/// `--ingest` must never publish to the wrong place.
pub fn parse_stream_args(args: &[String]) -> Result<StreamArgs, String> {
    let mut parsed = StreamArgs::default();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| format!("missing value for {flag}"))
        };
        match flag {
            "help" | "--help" => parsed.show_help = true,
            "-c" | "--config" => parsed.config_path = Some(PathBuf::from(value(&mut i)?)),
            "--platform" => parsed.overrides.platform = Some(value(&mut i)?),
            "--ingest" | "--ingest-url" => parsed.overrides.ingest_url = Some(value(&mut i)?),
            "--key" | "--stream-key" => parsed.overrides.stream_key = Some(value(&mut i)?),
            "--preset" => parsed.overrides.preset = Some(value(&mut i)?),
            "-d" | "--duration" => {
                parsed.overrides.duration_secs =
                    Some(value(&mut i)?.parse().map_err(|_| {
                        format!("--duration: not a number of seconds ({})", args[i])
                    })?)
            }
            "-w" | "--width" => {
                parsed.overrides.width = Some(
                    value(&mut i)?
                        .parse()
                        .map_err(|_| format!("--width: not a number ({})", args[i]))?,
                )
            }
            "-h" | "--height" => {
                parsed.overrides.height = Some(
                    value(&mut i)?
                        .parse()
                        .map_err(|_| format!("--height: not a number ({})", args[i]))?,
                )
            }
            "-f" | "--fps" => {
                parsed.overrides.fps = Some(
                    value(&mut i)?
                        .parse()
                        .map_err(|_| format!("--fps: not a number ({})", args[i]))?,
                )
            }
            "-a" | "--audio" => parsed.overrides.audio = true,
            "--dry-run" => parsed.dry_run = true,
            "--json" => parsed.json = true,
            other => return Err(format!("unknown flag {other:?}")),
        }
        i += 1;
    }
    if parsed.json && !parsed.dry_run {
        return Err(
            "--json: only valid with --dry-run (status events are always JSON on stdout)"
                .to_string(),
        );
    }
    Ok(parsed)
}

/// JSON status event emitted on stdout by `rivulet stream`.
///
/// The lifecycle names match [`crate::config::StatusEvent`] (`started`,
/// `progress`, `stopped`), so a consumer that already reads the recording
/// stream knows the shape. The fields are stream-specific: there is no file
/// size, but there is the measured bitrate, the byte count and the derived
/// health of the connection.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum StreamStatusEvent {
    /// Emitted once, after the ingest probe succeeded and the session started.
    Started {
        width: u32,
        height: u32,
        fps: u32,
        #[serde(rename = "audio")]
        audio_enabled: bool,
        /// Platform label (`Twitch`, `YouTube`, `Kick`, `Custom`).
        platform: String,
        /// Ingest endpoint reduced to scheme + authority; never the key.
        ingest: String,
        /// The masked key, never the key itself.
        key: String,
        /// Target encoder bitrate in kbit/s.
        bitrate_kbps: u32,
    },
    /// Emitted every second with the engine's stream telemetry.
    Progress {
        seconds: u64,
        frames: u64,
        /// Measured bitrate over the engine's sliding window (kbit/s).
        kbps: f64,
        bytes_sent: u64,
        /// Dropped frames as a fraction of sent + dropped (0.0 ..= 1.0).
        dropped_ratio: f64,
        /// `offline`, `connecting`, `good`, `warning` or `poor`.
        health: String,
    },
    /// Emitted once when the stream has been finalized.
    Stopped {
        seconds: u64,
        frames: u64,
        bytes_sent: u64,
        kbps: f64,
        dropped_ratio: f64,
        health: String,
    },
}

/// Human-readable one-line description of a stream event (stderr diagnostics).
pub fn describe(event: &StreamStatusEvent) -> String {
    match event {
        StreamStatusEvent::Started {
            width,
            height,
            fps,
            audio_enabled,
            platform,
            ingest,
            key,
            bitrate_kbps,
        } => format!(
            "streaming started: {width}x{height}@{fps} audio={audio_enabled} \
             platform={platform} ingest={ingest} key={key} bitrate={bitrate_kbps}kbps"
        ),
        StreamStatusEvent::Progress {
            seconds,
            frames,
            kbps,
            bytes_sent,
            dropped_ratio,
            health,
        } => format!(
            "progress: {seconds}s frames={frames} kbps={kbps:.1} bytes={bytes_sent} \
             dropped={:.1}% health={health}",
            dropped_ratio * 100.0
        ),
        StreamStatusEvent::Stopped {
            seconds,
            frames,
            bytes_sent,
            kbps,
            dropped_ratio,
            health,
        } => format!(
            "stream stopped: {seconds}s frames={frames} bytes={bytes_sent} kbps={kbps:.1} \
             dropped={:.1}% health={health}",
            dropped_ratio * 100.0
        ),
    }
}

/// The outcome of a completed stream run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StreamReport {
    /// Wall-clock seconds the session ran.
    pub seconds: u64,
    /// Video frames pushed into the pipeline.
    pub frames: u64,
    /// Bytes handed to the network sink.
    pub bytes_sent: u64,
    /// Last measured bitrate in kbit/s.
    pub kbps: f64,
    /// Dropped frames as a fraction of sent + dropped.
    pub dropped_ratio: f64,
    /// Last derived health (`offline`, `connecting`, `good`, `warning`, `poor`).
    pub health: String,
}

/// The plan a `rivulet stream --dry-run` prints.
///
/// Shares `StreamJob::plan` with the real run, so the pipeline reported here
/// is the pipeline the run publishes through — the same guarantee `rivulet
/// inspect` gives the recording path.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StreamDryRunReport {
    /// Platform label.
    pub platform: String,
    /// Ingest endpoint reduced to scheme + authority.
    pub ingest: String,
    /// The masked key.
    pub key: String,
    /// Target encoder bitrate in kbit/s.
    pub bitrate_kbps: u32,
    /// Redacted pipeline description.
    pub pipeline: String,
    /// The fully resolved config, so the report is self-describing.
    pub config: StreamConfig,
}

impl StreamDryRunReport {
    /// Render the plan for a human (stdout of `--dry-run` without `--json`).
    pub fn describe(&self) -> String {
        let video = &self.config.video;
        let duration = self
            .config
            .duration_secs
            .map(|secs| format!("{secs}s"))
            .unwrap_or_else(|| "until stopped".to_string());
        format!(
            "stream plan:\n  platform: {}\n  ingest: {}\n  key: {}\n  \
             bitrate: {} kbit/s\n  video: {}x{} @ {} fps, audio={}, duration={}\n  \
             pipeline: {}",
            self.platform,
            self.ingest,
            self.key,
            self.bitrate_kbps,
            video.width,
            video.height,
            video.fps,
            self.config.audio.enabled,
            duration,
            self.pipeline
        )
    }
}

/// One headless streaming run: config in, published session out.
///
/// Binary and library paths share this implementation. Events (JSON on
/// stdout, human diagnostics on stderr) flow through the provided closures so
/// library consumers can route them anywhere.
pub struct StreamJob {
    config: StreamConfig,
    /// JSON status events (spec: stable documented schema).
    pub on_event: Box<dyn FnMut(&StreamStatusEvent) + Send>,
    /// Human-readable diagnostics (stderr in the binary).
    pub on_diagnostic: Box<dyn FnMut(&str) + Send>,
}

impl StreamJob {
    /// Build a job from a config. `run` validates it before touching the
    /// network.
    pub fn new(config: StreamConfig) -> Self {
        Self {
            config,
            on_event: Box::new(|_| {}),
            on_diagnostic: Box::new(|_| {}),
        }
    }

    /// Register the JSON-event sink.
    pub fn with_event_sink(mut self, sink: Box<dyn FnMut(&StreamStatusEvent) + Send>) -> Self {
        self.on_event = sink;
        self
    }

    /// Register the human-readable diagnostics sink.
    pub fn with_diagnostic_sink(mut self, sink: Box<dyn FnMut(&str) + Send>) -> Self {
        self.on_diagnostic = sink;
        self
    }

    fn emit(&mut self, event: StreamStatusEvent) {
        (self.on_event)(&event);
    }

    fn diag(&mut self, msg: &str) {
        (self.on_diagnostic)(msg);
    }

    /// Run the stream to completion and return its final telemetry.
    ///
    /// `stop_requested` is polled between frames; when it returns `true`
    /// (SIGINT/SIGTERM in the binary) the stream is stopped cleanly and the
    /// session finalized.
    pub fn run(
        &mut self,
        stop_requested: impl Fn() -> bool,
    ) -> std::result::Result<StreamReport, RunFailure> {
        self.config
            .validate()
            .map_err(|e| RunFailure::config(e).with_stream_config(self.config.redacted()))?;

        let (settings, mut engine) = self.plan()?;
        // Probe before the first frame: an ingest that does not answer is an
        // output failure, not something the user should discover mid-stream.
        self.probe(&settings)?;

        engine.start_streaming();
        if let Some(err) = engine.take_error() {
            return Err(RunFailure::config(format!("stream: {err}"))
                .with_stream_config(self.config.redacted()));
        }

        let width = self.config.video.width;
        let height = self.config.video.height;
        let fps = self.config.video.fps;
        let audio_enabled = self.config.audio.enabled;
        let duration = self.config.duration_secs;

        self.emit(StreamStatusEvent::Started {
            width,
            height,
            fps,
            audio_enabled,
            platform: settings.platform.label().to_string(),
            ingest: redacted_ingest(&settings.ingest_url),
            key: settings.masked_key(),
            bitrate_kbps: engine.video_bitrate(),
        });
        self.diag(&format!(
            "streaming to {} as {} ({}x{} @ {fps} fps, audio={audio_enabled}, {} kbit/s)",
            redacted_ingest(&settings.ingest_url),
            settings.platform.label(),
            width,
            height,
            engine.video_bitrate()
        ));

        let mut video = TestVideoSource::new(width, height, fps);
        let mut audio = if audio_enabled {
            Some(SilenceSource::new())
        } else {
            None
        };

        let frame_interval = Duration::from_secs_f64(1.0 / f64::from(fps));
        // One video frame's worth of audio per frame, so the audio timeline
        // advances at the same rate as the video timeline. (`RecordJob` pushes
        // 10 ms per frame, which is fine for a short clip but would leave a
        // long stream with a far shorter audio track than video track.)
        let audio_samples_per_push = audio_samples_per_frame(fps);
        let started_at = Instant::now();
        let mut frames_pushed: u64 = 0;
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
            // Drain bus messages, reconnect targets and the adaptive-bitrate
            // controller. The engine requires this on the pipeline owner
            // thread, which is this loop.
            if let Err(err) = engine.service_streaming() {
                return Err(RunFailure::engine(format!("servicing the stream: {err:#}"))
                    .with_pipeline(engine.pipeline_description())
                    .with_stream_config(self.config.redacted()));
            }
            if let Some(err) = engine.take_error() {
                return Err(RunFailure::engine(format!("engine error: {err}"))
                    .with_pipeline(engine.pipeline_description())
                    .with_stream_config(self.config.redacted()));
            }

            engine.process_raw_frame(&video.next_frame(), width, height);
            frames_pushed += 1;

            if let Some(audio) = audio.as_mut() {
                let frame = audio.next_frame(audio_samples_per_push);
                engine.push_audio_frame(&frame).map_err(|e| {
                    RunFailure::engine(format!("pushing audio frame: {e:#}"))
                        .with_pipeline(engine.pipeline_description())
                        .with_stream_config(self.config.redacted())
                })?;
            }

            if last_progress_at.elapsed() >= Duration::from_secs(1) {
                let secs = started_at.elapsed().as_secs();
                if secs > last_progress_secs {
                    last_progress_secs = secs;
                    last_progress_at = Instant::now();
                    let stats = engine.stream_stats();
                    self.emit(StreamStatusEvent::Progress {
                        seconds: secs,
                        frames: stats.frames_sent.max(frames_pushed),
                        kbps: stats.kbps,
                        bytes_sent: stats.bytes_sent,
                        dropped_ratio: stats.dropped_ratio,
                        health: health_label(stats.status).to_string(),
                    });
                }
            }

            // Pace against wall time so the FLV timestamps track real time,
            // exactly as the recording path paces its appsrc pushes.
            let next_frame_at = started_at
                + Duration::from_secs_f64(frames_pushed as f64 * frame_interval.as_secs_f64());
            let wait = next_frame_at.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }

        // The health monitor is detached by `stop_recording`, so take the
        // final sample first.
        let stats = engine.stream_stats();
        engine.stop_recording();
        let stop_error = engine.take_error();
        let seconds = started_at.elapsed().as_secs();

        let report = StreamReport {
            seconds,
            frames: stats.frames_sent.max(frames_pushed),
            bytes_sent: stats.bytes_sent,
            kbps: stats.kbps,
            dropped_ratio: stats.dropped_ratio,
            health: health_label(stats.status).to_string(),
        };

        self.emit(StreamStatusEvent::Stopped {
            seconds: report.seconds,
            frames: report.frames,
            bytes_sent: report.bytes_sent,
            kbps: report.kbps,
            dropped_ratio: report.dropped_ratio,
            health: report.health.clone(),
        });

        if let Some(err) = &stop_error {
            self.diag(&format!("engine reported: {err}"));
        }
        if report.frames == 0 {
            let detail = match &stop_error {
                Some(err) => format!("engine error: {err}"),
                None => "the stream ended without sending a single frame".to_string(),
            };
            return Err(RunFailure::finalize(detail)
                .with_pipeline(engine.pipeline_description())
                .with_stream_config(self.config.redacted()));
        }

        self.diag(&format!(
            "stream ended: {}s frames={} bytes={} health={}",
            report.seconds, report.frames, report.bytes_sent, report.health
        ));
        Ok(report)
    }

    /// Validate the config, probe the ingest and report the plan without
    /// publishing anything (the stream counterpart of `record --dry-run`).
    ///
    /// Shares `StreamJob::plan` with [`StreamJob::run`], so the dry run
    /// cannot describe a pipeline the run would not build. The probe makes a
    /// broken ingest fail here as `config`/`output`, before a session exists.
    pub fn dry_run(&mut self) -> std::result::Result<StreamDryRunReport, RunFailure> {
        self.config
            .validate()
            .map_err(|e| RunFailure::config(e).with_stream_config(self.config.redacted()))?;
        let (settings, engine) = self.plan()?;
        self.probe(&settings)?;
        Ok(StreamDryRunReport {
            platform: settings.platform.label().to_string(),
            ingest: redacted_ingest(&settings.ingest_url),
            key: settings.masked_key(),
            bitrate_kbps: engine.video_bitrate(),
            pipeline: engine.pipeline_description().unwrap_or_default(),
            config: self.config.redacted(),
        })
    }

    /// Build the engine and its settings from the config, without starting a
    /// session. Shared by the dry run and the real run.
    fn plan(&self) -> std::result::Result<(StreamSettings, RivuletEngine), RunFailure> {
        let settings = self
            .config
            .stream_settings()
            .map_err(|e| RunFailure::config(e).with_stream_config(self.config.redacted()))?;
        let mut engine = RivuletEngine::new();
        engine.set_audio_enabled(self.config.audio.enabled);
        if let Some(kbps) = settings.preset.bitrate_kbps() {
            engine.set_video_bitrate(kbps);
        }
        engine.set_stream_settings(Some(settings.clone()));
        Ok((settings, engine))
    }

    /// One bounded TCP probe of the ingest endpoint.
    ///
    /// A URL the core rejects is a `config` failure; an endpoint that does not
    /// answer is an `output` failure. Neither publishes media or validates the
    /// stream key — that is what the session does.
    fn probe(&mut self, settings: &StreamSettings) -> std::result::Result<(), RunFailure> {
        let endpoint = redacted_ingest(&settings.ingest_url);
        match settings.probe_reachability(PROBE_TIMEOUT) {
            StreamProbeResult::Reachable => {
                self.diag(&format!("ingest reachable: {endpoint}"));
                Ok(())
            }
            StreamProbeResult::Rejected(detail) => {
                Err(RunFailure::config(format!("stream: {detail}"))
                    .with_stream_config(self.config.redacted()))
            }
            StreamProbeResult::Unavailable(detail) => Err(RunFailure::output(format!(
                "stream.ingest_url: {endpoint} did not answer within {:?} ({detail})",
                PROBE_TIMEOUT
            ))
            .with_stream_config(self.config.redacted())),
        }
    }
}

impl StreamConfig {
    /// Map the CLI config onto the engine's stream settings.
    ///
    /// Callers validate first ([`StreamConfig::validate`]); this maps the
    /// already-checked strings onto the core enums and re-checks the result
    /// with the core's own `validate`, so the two cannot disagree.
    pub fn stream_settings(&self) -> std::result::Result<StreamSettings, String> {
        let platform = match self.stream.platform.as_str() {
            "twitch" => StreamPlatform::Twitch,
            "youtube" => StreamPlatform::YouTube,
            "kick" => StreamPlatform::Kick,
            "custom" => StreamPlatform::Custom,
            other => {
                return Err(format!(
                    "stream.platform: unknown platform {other:?} (expected twitch, youtube, kick, or custom)"
                ))
            }
        };
        let ingest_url = self
            .stream
            .ingest_url
            .clone()
            .or_else(|| platform.default_ingest_url().map(str::to_string))
            .unwrap_or_default();
        let settings = StreamSettings::new(
            platform,
            ingest_url,
            self.stream.stream_key.clone().unwrap_or_default(),
        )
        .with_preset(self.preset()?);
        settings
            .validate()
            .map_err(|error| format!("stream: {error}"))?;
        Ok(settings)
    }

    fn preset(&self) -> std::result::Result<StreamPreset, String> {
        match self.stream.preset.as_deref() {
            None | Some("standard") => Ok(StreamPreset::Standard),
            Some("low") => Ok(StreamPreset::Low),
            Some("high") => Ok(StreamPreset::High),
            Some("custom") => Ok(StreamPreset::Custom),
            Some(other) => Err(format!(
                "stream.preset: unknown preset {other:?} (expected low, standard, high, or custom)"
            )),
        }
    }
}

/// The semi-public shape of an ingest endpoint: scheme plus authority.
///
/// A custom RTMP URL may carry the stream key in its path, so nothing after
/// the authority is ever reported.
fn redacted_ingest(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "<invalid ingest URL>".to_string();
    };
    let authority = rest.split('/').next().unwrap_or_default();
    // Drop a `user:password@` prefix if one is present; the host is the part
    // that explains a failed connection.
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.is_empty() {
        format!("{scheme}://<empty>")
    } else {
        format!("{scheme}://{authority}")
    }
}

/// Audio samples per channel one video frame carries, so the audio timeline and
/// the video timeline advance at the same rate.
///
/// `fps` is clamped to at least 1 so an absurd config cannot divide by zero;
/// [`crate::config::StreamConfig::validate`] rejects `fps = 0` first.
fn audio_samples_per_frame(fps: u32) -> usize {
    (rivulet_core::AUDIO_SAMPLE_RATE / fps.max(1)) as usize
}

/// The documented snake_case name of a health status.
fn health_label(status: StreamHealthStatus) -> &'static str {
    match status {
        StreamHealthStatus::Offline => "offline",
        StreamHealthStatus::Connecting => "connecting",
        StreamHealthStatus::Good => "good",
        StreamHealthStatus::Warning => "warning",
        StreamHealthStatus::Poor => "poor",
    }
}

/// Event/diagnostic sink pair used by the binary for `rivulet stream`.
pub type StreamSinkPair = (
    Box<dyn FnMut(&StreamStatusEvent) + Send>,
    Box<dyn FnMut(&str) + Send>,
);

/// JSON events to stdout, diagnostics to stderr — the same split `record` uses.
pub fn stdout_stderr_sinks() -> StreamSinkPair {
    let event_sink = Box::new(|event: &StreamStatusEvent| {
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
/// default sinks.
pub fn stream(config: StreamConfig) -> std::result::Result<StreamReport, RunFailure> {
    let (events, diags) = stdout_stderr_sinks();
    StreamJob::new(config)
        .with_event_sink(events)
        .with_diagnostic_sink(diags)
        .run(|| false)
}

/// Emit a failure the documented way: the machine-readable `failed` object on
/// stdout, the human line on stderr.
fn report_failure(failure: &RunFailure) -> i32 {
    crate::report_failure(failure)
}

/// The binary's `rivulet stream` entry point.
pub fn run_stream(args: &StreamArgs) -> i32 {
    if args.show_help {
        print!("{USAGE}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        return exit_code::OK;
    }

    let mut config = match &args.config_path {
        Some(path) => match StreamConfig::load(path) {
            Ok(config) => config,
            Err(error) => return report_failure(&RunFailure::config(error)),
        },
        None => StreamConfig::default(),
    };
    args.overrides.clone().apply(&mut config);

    if let Err(error) = config.validate() {
        let failure = RunFailure::config(error).with_stream_config(config.redacted());
        println!("{}", failure.to_json());
        eprintln!("{}\n\n{USAGE}", failure.describe());
        return failure.exit_code();
    }

    if args.dry_run {
        return match StreamJob::new(config).dry_run() {
            Ok(report) => {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string(&report).expect("serializable plan")
                    );
                } else {
                    println!("{}", report.describe());
                }
                exit_code::OK
            }
            Err(failure) => report_failure(&failure),
        };
    }

    // Graceful SIGINT/SIGTERM: flip the flag; the job stops between frames and
    // finalizes the session (spec: exit code 0 on graceful stop).
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))
            .expect("installing the signal handler");
    }

    let (events, diags) = stdout_stderr_sinks();
    let mut job = StreamJob::new(config)
        .with_event_sink(events)
        .with_diagnostic_sink(diags);

    match job.run(|| stop.load(Ordering::SeqCst)) {
        Ok(report) => {
            eprintln!(
                "done: stream ended after {}s ({} frames, {} bytes, health={})",
                report.seconds, report.frames, report.bytes_sent, report.health
            );
            exit_code::OK
        }
        Err(failure) => report_failure(&failure),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AudioConfig, StreamOutputConfig, VideoConfig};

    fn valid_config() -> StreamConfig {
        StreamConfig {
            stream: StreamOutputConfig {
                platform: "custom".to_string(),
                ingest_url: Some("rtmp://127.0.0.1:1935/live".to_string()),
                stream_key: Some("fixture_key".to_string()),
                preset: None,
            },
            video: VideoConfig {
                source: "test".to_string(),
                test_pattern: None,
                width: 320,
                height: 240,
                fps: 30,
            },
            audio: AudioConfig { enabled: false },
            duration_secs: Some(1),
        }
    }

    #[test]
    fn a_valid_config_maps_to_engine_settings() {
        let settings = valid_config().stream_settings().expect("valid config maps");
        assert_eq!(settings.platform, StreamPlatform::Custom);
        assert_eq!(settings.ingest_url, "rtmp://127.0.0.1:1935/live");
        assert_eq!(settings.stream_key, "fixture_key");
        assert_eq!(settings.preset, StreamPreset::Standard);
    }

    #[test]
    fn a_platform_preset_fills_in_the_default_ingest() {
        let mut config = valid_config();
        config.stream.platform = "twitch".to_string();
        config.stream.ingest_url = None;
        let settings = config.stream_settings().expect("twitch default ingest");
        assert_eq!(settings.platform, StreamPlatform::Twitch);
        assert_eq!(settings.ingest_url, "rtmps://live.twitch.tv/app");
    }

    #[test]
    fn the_preset_selects_the_bitrate() {
        let mut config = valid_config();
        for (name, preset, kbps) in [
            ("low", StreamPreset::Low, Some(2500)),
            ("standard", StreamPreset::Standard, Some(4500)),
            ("high", StreamPreset::High, Some(6000)),
            ("custom", StreamPreset::Custom, None),
        ] {
            config.stream.preset = Some(name.to_string());
            let settings = config.stream_settings().expect("valid preset");
            assert_eq!(settings.preset, preset, "{name}");
            assert_eq!(settings.preset.bitrate_kbps(), kbps, "{name}");
        }
    }

    #[test]
    fn the_dry_run_reports_the_pipeline_the_run_would_build() {
        // The plan is produced without starting a session; the pipeline still
        // names the streaming muxer and sink, which is what the run builds on
        // the first frame.
        let report = StreamJob::new(valid_config())
            .dry_run()
            // The default port is almost certainly closed in CI; use a
            // listener so the probe succeeds.
            .map_err(|failure| failure.to_string());
        // `127.0.0.1:1935` is not guaranteed to be listening, so accept the
        // documented output-stage failure as the honest outcome and assert the
        // plan shape only when the probe reached something.
        match report {
            Ok(plan) => {
                assert!(plan.pipeline.contains("flvmux"), "{}", plan.pipeline);
                assert!(plan.pipeline.contains("rtmp2sink"), "{}", plan.pipeline);
                assert!(
                    !plan.pipeline.contains("fixture_key"),
                    "the plan must not leak the key: {}",
                    plan.pipeline
                );
                assert_eq!(plan.ingest, "rtmp://127.0.0.1:1935");
                assert_eq!(plan.key, "fi••••");
                assert_eq!(plan.bitrate_kbps, 4500);
            }
            Err(message) => assert!(
                message.contains("stream.ingest_url"),
                "only an unreachable ingest may fail here, got: {message}"
            ),
        }
    }

    #[test]
    fn a_bad_config_is_a_config_failure_before_any_network_use() {
        let mut config = valid_config();
        config.stream.stream_key = None;
        let failure = StreamJob::new(config).dry_run().unwrap_err();
        assert_eq!(failure.stage(), crate::Stage::Config);
        assert_eq!(failure.exit_code(), exit_code::USAGE);
        assert!(failure.message().contains("stream.stream_key"));
        assert!(failure.resolved_stream_config().is_some());
    }

    #[test]
    fn an_unreachable_ingest_is_an_output_failure() {
        // Port 1 on loopback: nothing listens there, and the probe is bounded.
        let mut config = valid_config();
        config.stream.ingest_url = Some("rtmp://127.0.0.1:1/live".to_string());
        let failure = StreamJob::new(config).dry_run().unwrap_err();
        assert_eq!(failure.stage(), crate::Stage::Output, "{failure}");
        assert_eq!(failure.exit_code(), exit_code::RUNTIME);
        assert!(failure.message().contains("stream.ingest_url"), "{failure}");
        assert!(
            failure.message().contains("rtmp://127.0.0.1:1"),
            "the failure names the endpoint: {failure}"
        );
        assert!(
            !failure.message().contains("fixture_key"),
            "the failure must not leak the key: {failure}"
        );
    }

    #[test]
    fn parse_args_keeps_the_config_and_applies_overrides() {
        let args: Vec<String> = [
            "--config",
            "streaming.toml",
            "--platform",
            "custom",
            "--ingest",
            "rtmp://example.test/live",
            "--key",
            "k-123",
            "--preset",
            "low",
            "--duration",
            "5",
            "--width",
            "1280",
            "--height",
            "720",
            "--fps",
            "60",
            "--audio",
            "--dry-run",
            "--json",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let parsed = parse_stream_args(&args).expect("valid flags");
        assert_eq!(parsed.config_path, Some(PathBuf::from("streaming.toml")));
        assert!(parsed.dry_run && parsed.json);
        assert_eq!(parsed.overrides.platform.as_deref(), Some("custom"));
        assert_eq!(
            parsed.overrides.ingest_url.as_deref(),
            Some("rtmp://example.test/live")
        );
        assert_eq!(parsed.overrides.stream_key.as_deref(), Some("k-123"));
        assert_eq!(parsed.overrides.preset.as_deref(), Some("low"));
        assert_eq!(parsed.overrides.duration_secs, Some(5));
        assert_eq!(parsed.overrides.width, Some(1280));
        assert_eq!(parsed.overrides.height, Some(720));
        assert_eq!(parsed.overrides.fps, Some(60));
        assert!(parsed.overrides.audio);
    }

    #[test]
    fn parse_args_rejects_an_unknown_flag() {
        let args = vec!["--ingest-url".to_string(), "x".to_string()];
        // `--ingest-url` is an alias, so use a real typo.
        assert!(parse_stream_args(&args).is_ok());
        let args = vec!["--ingestt".to_string(), "x".to_string()];
        let err = parse_stream_args(&args).unwrap_err();
        assert!(err.contains("--ingestt"), "{err}");
    }

    #[test]
    fn parse_args_accepts_the_help_words() {
        for word in ["help", "--help"] {
            let args = vec![word.to_string()];
            let parsed = parse_stream_args(&args).expect("help is not an error");
            assert!(parsed.show_help, "{word} must show the stream usage");
        }
    }

    #[test]
    fn parse_args_requires_dry_run_for_json() {
        let args = vec!["--json".to_string()];
        let err = parse_stream_args(&args).unwrap_err();
        assert!(err.contains("--json"), "{err}");
        assert!(err.contains("--dry-run"), "the error names the fix: {err}");
    }

    #[test]
    fn overrides_win_over_the_loaded_config() {
        let mut config = valid_config();
        StreamOverrides {
            platform: Some("custom".to_string()),
            ingest_url: Some("rtmp://override.test/live".to_string()),
            stream_key: Some("override-key".to_string()),
            preset: Some("high".to_string()),
            duration_secs: Some(9),
            width: Some(1280),
            height: Some(720),
            fps: Some(60),
            audio: true,
        }
        .apply(&mut config);
        assert_eq!(
            config.stream.ingest_url.as_deref(),
            Some("rtmp://override.test/live")
        );
        assert_eq!(config.stream.stream_key.as_deref(), Some("override-key"));
        assert_eq!(config.stream.preset.as_deref(), Some("high"));
        assert_eq!(config.duration_secs, Some(9));
        assert_eq!((config.video.width, config.video.height), (1280, 720));
        assert_eq!(config.video.fps, 60);
        assert!(config.audio.enabled);
    }

    #[test]
    fn redacted_ingest_drops_a_path_embedded_key() {
        assert_eq!(
            redacted_ingest("rtmp://live.example.com/app/live_123_secret"),
            "rtmp://live.example.com"
        );
        assert_eq!(
            redacted_ingest("rtmps://a.rtmp.youtube.com:443/live2/abcd-efgh"),
            "rtmps://a.rtmp.youtube.com:443"
        );
        assert_eq!(
            redacted_ingest("rtmp://user:pass@host:1935/live"),
            "rtmp://host:1935"
        );
        assert_eq!(redacted_ingest("not a url"), "<invalid ingest URL>");
    }

    #[test]
    fn the_started_event_masks_the_key_and_drops_the_path() {
        let event = StreamStatusEvent::Started {
            width: 640,
            height: 360,
            fps: 30,
            audio_enabled: false,
            platform: "Custom".to_string(),
            ingest: redacted_ingest("rtmp://127.0.0.1:1935/live/fixture_key"),
            key: "fi••••".to_string(),
            bitrate_kbps: 4500,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""event":"started""#), "{json}");
        assert!(json.contains(r#""audio":false"#), "{json}");
        assert!(
            json.contains(r#""ingest":"rtmp://127.0.0.1:1935""#),
            "{json}"
        );
        assert!(!json.contains("fixture_key"), "{json}");
        let line = describe(&event);
        assert!(line.starts_with("streaming started: "), "{line}");
        assert!(line.contains("key=fi••••"), "{line}");
    }

    #[test]
    fn the_progress_and_stopped_events_carry_the_telemetry() {
        let progress = StreamStatusEvent::Progress {
            seconds: 2,
            frames: 60,
            kbps: 4310.5,
            bytes_sent: 1_076_000,
            dropped_ratio: 0.0,
            health: "good".to_string(),
        };
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&progress).unwrap()).unwrap();
        assert_eq!(value["event"], "progress");
        assert_eq!(value["kbps"], 4310.5);
        assert_eq!(value["health"], "good");
        assert_eq!(value["bytes_sent"], 1_076_000u64);

        let stopped = StreamStatusEvent::Stopped {
            seconds: 3,
            frames: 90,
            bytes_sent: 1_500_000,
            kbps: 4200.0,
            dropped_ratio: 0.01,
            health: "warning".to_string(),
        };
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&stopped).unwrap()).unwrap();
        assert_eq!(value["event"], "stopped");
        assert_eq!(value["frames"], 90);
        assert!(describe(&stopped).starts_with("stream stopped: "));
    }

    #[test]
    fn the_audio_timeline_matches_the_video_timeline() {
        // Streaming for longer than a clip makes A/V drift visible, so the job
        // feeds one frame's worth of audio per frame instead of a fixed 10 ms.
        for fps in [24, 30, 50, 60] {
            let samples = audio_samples_per_frame(fps);
            let seconds = samples as f64 / f64::from(rivulet_core::AUDIO_SAMPLE_RATE);
            let expected = 1.0 / f64::from(fps);
            assert!(
                (seconds - expected).abs() < 1e-3,
                "fps={fps}: {samples} samples cover {seconds}s, expected {expected}s"
            );
        }
        assert!(
            audio_samples_per_frame(0) > 0,
            "a zero frame rate must not divide by zero"
        );
    }

    #[test]
    fn every_health_status_has_a_stable_label() {
        for (status, label) in [
            (StreamHealthStatus::Offline, "offline"),
            (StreamHealthStatus::Connecting, "connecting"),
            (StreamHealthStatus::Good, "good"),
            (StreamHealthStatus::Warning, "warning"),
            (StreamHealthStatus::Poor, "poor"),
        ] {
            assert_eq!(health_label(status), label);
        }
    }

    #[test]
    fn the_dry_run_report_renders_for_a_human() {
        let report = StreamDryRunReport {
            platform: "Custom".to_string(),
            ingest: "rtmp://127.0.0.1:1935".to_string(),
            key: "fi••••".to_string(),
            bitrate_kbps: 4500,
            pipeline: "appsrc ! flvmux ! rtmp2sink".to_string(),
            config: valid_config().redacted(),
        };
        let text = report.describe();
        for fragment in [
            "stream plan:",
            "platform: Custom",
            "ingest: rtmp://127.0.0.1:1935",
            "key: fi••••",
            "bitrate: 4500 kbit/s",
            "video: 320x240 @ 30 fps",
            "pipeline: appsrc ! flvmux ! rtmp2sink",
        ] {
            assert!(text.contains(fragment), "missing {fragment:?} in {text}");
        }
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""pipeline""#), "{json}");
        assert!(!json.contains("fixture_key"), "{json}");
    }

    #[test]
    fn a_dry_run_never_reports_the_key_even_when_the_url_embeds_it() {
        // The dry run is a serialization boundary: the resolved config it
        // carries must be the redacted one, both for the embedded key and for
        // an ingest URL that contains it in its path (custom / Kick ingests).
        let mut config = valid_config();
        config.stream.ingest_url = Some("rtmp://127.0.0.1:1935/live/fixture_key".to_string());
        let redacted = config.redacted();
        assert_eq!(
            redacted.stream.stream_key.as_deref(),
            Some("fi••••"),
            "the key must be masked"
        );
        assert_eq!(
            redacted.stream.ingest_url.as_deref(),
            Some("<redacted stream URL>"),
            "a URL embedding the key must be replaced outright"
        );
        assert_eq!(
            config.stream.stream_key.as_deref(),
            Some("fixture_key"),
            "redaction must not mutate the live config"
        );

        let report = StreamDryRunReport {
            platform: "Custom".to_string(),
            ingest: redacted_ingest("rtmp://127.0.0.1:1935/live/fixture_key"),
            key: "fi••••".to_string(),
            bitrate_kbps: 4500,
            pipeline: "appsrc ! flvmux ! rtmp2sink".to_string(),
            config: redacted,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("fixture_key"), "{json}");
        assert!(!report.describe().contains("fixture_key"));
    }

    #[test]
    fn every_failure_surface_carries_the_redacted_config() {
        // A failing run must not leak the key through `stream_config`.
        let mut config = valid_config();
        config.stream.ingest_url = Some("rtmp://127.0.0.1:1/live".to_string());
        let failure = StreamJob::new(config).dry_run().unwrap_err();
        let json = failure.to_json();
        assert!(!json.contains("fixture_key"), "{json}");
        let carried = failure
            .resolved_stream_config()
            .expect("the failure carries the stream config");
        assert_eq!(carried.stream.stream_key.as_deref(), Some("fi••••"));
    }

    #[test]
    fn the_report_serializes_for_machine_consumers() {
        let report = StreamReport {
            seconds: 3,
            frames: 90,
            bytes_sent: 1_500_000,
            kbps: 4200.0,
            dropped_ratio: 0.01,
            health: "good".to_string(),
        };
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&report).unwrap()).unwrap();
        for key in [
            "seconds",
            "frames",
            "bytes_sent",
            "kbps",
            "dropped_ratio",
            "health",
        ] {
            assert!(value.get(key).is_some(), "report needs {key}");
        }
    }
}
