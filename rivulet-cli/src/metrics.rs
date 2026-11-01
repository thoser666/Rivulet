//! `rivulet metrics` — phase timings for the M9 resource-metrics harness
//! (issue #363).
//!
//! This subcommand does **not** decide whether the numbers are good; that is
//! `scripts/resource-metrics.py`'s job, together with a committed baseline.
//! It only drives the real engine and the real render path through a defined
//! sequence and reports what each phase cost, as newline-delimited JSON on
//! stdout:
//!
//! ```text
//! {"event":"phase_start","phase":"startup"}
//! {"event":"phase_end","phase":"startup","duration_ms":3.1,...}
//! {"event":"phase_start","phase":"idle"}
//! ...
//! {"event":"report","schema_version":1,...}
//! ```
//!
//! The events exist so an *external* sampler can attach its samples to the
//! right phase: process CPU, RSS and thread count are measured by the harness
//! from outside, because a process that reports its own footprint can always
//! be wrong about it. The timings this module reports are the ones only the
//! process itself can see (engine bootstrap, per-frame render cost).
//!
//! Phases are opt-in:
//!
//! - `startup` always runs: it constructs [`RivuletEngine`], which is where
//!   GStreamer is initialised, and is the only phase a bare `rivulet metrics`
//!   performs.
//! - `idle` (`--idle-seconds N`) holds the engine and does nothing, which is
//!   what the "idle CPU ≤ 2 %" target in
//!   [`docs/resource-efficiency-goal.md`](../../docs/resource-efficiency-goal.md)
//!   is about.
//! - `preview` (`--config <scene.toml> --frames N`) renders N frames through
//!   [`SceneRenderConfig::render_frame`], the exact path `rivulet render`
//!   uses, so per-frame cost is measured from real code rather than a fixture.
//! - `record` (`--record-seconds N`) pushes a synthetic RGBA frame stream at
//!   `--fps` through a real recording pipeline. The capture-on half of the
//!   capture-on/off A/B is this phase; the capture-off half is `--idle-seconds`
//!   with the same duration.
//!
//! A runner without GStreamer cannot measure `startup` honestly. Rather than
//! fabricate a number, the phase is reported as `unavailable` with the panic
//! message as the reason, and the harness records `BLOCKED`/`N/A` with that
//! reason — the gate wording in
//! [`docs/milestone-quality-gates.md`](../../docs/milestone-quality-gates.md)
//! requires exactly that instead of a silently passing run.

use rivulet_core::render::SceneRenderConfig;
use rivulet_core::{RivuletEngine, VideoEncoder};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::exit_code;

/// Schema version of the `report` event. Bumping it is a breaking change for
/// `scripts/resource-metrics.py`, which reads the fields this emits.
pub const SCHEMA_VERSION: u32 = 1;

/// Default frame count for the `preview` phase.
const DEFAULT_FRAMES: u32 = 30;

/// Default synthetic capture geometry: small enough to run on a shared CI
/// runner, large enough that per-frame work is not noise.
const DEFAULT_WIDTH: u32 = 640;
const DEFAULT_HEIGHT: u32 = 360;
const DEFAULT_FPS: u32 = 30;

/// Upper bounds. A metrics run is a measurement, not a stress test: the
/// bounds keep a typo (`--idle-seconds 36000`) from hanging the job for hours.
const MAX_SECONDS: f64 = 3_600.0;
const MAX_DIMENSION: u32 = 7_680;
const MAX_FPS: u32 = 240;
const MAX_FRAMES: u32 = 6_000;

/// The `metrics` section of `rivulet --help`.
pub const USAGE: &str = "\
USAGE:
    rivulet metrics [--idle-seconds <n>] [--config <scene.toml> --frames <n>]
                    [--record-seconds <n>] [--no-engine]

FLAGS (metrics):
        --idle-seconds <n>    hold the engine idle for N seconds (default 0)
        --config <file>       scene composition rendered as the preview workload
        --frames <n>          frames the preview workload renders (default 30)
        --record-seconds <n>  drive a synthetic recording for N seconds (default 0)
        --width <px>          synthetic capture width (default 640)
        --height <px>         synthetic capture height (default 360)
        --fps <n>             synthetic capture frame rate (default 30)
        --no-engine           skip the GStreamer engine bootstrap

Emits newline-delimited JSON on stdout: `phase_start`, `phase_end` and a final
`report` event. `scripts/resource-metrics.py` drives it and samples the process
from outside; the numbers are gated against a committed baseline there.
";

/// Parsed `rivulet metrics` arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricsArgs {
    /// How long the `idle` phase holds the engine without any work.
    pub idle_seconds: f64,
    /// How long the `record` phase pushes synthetic frames.
    pub record_seconds: f64,
    /// Scene composition for the `preview` phase.
    pub config: Option<PathBuf>,
    /// Frames the `preview` phase renders.
    pub frames: u32,
    /// Synthetic capture geometry for the `record` phase.
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Skip [`RivuletEngine`] construction (and therefore GStreamer init).
    pub no_engine: bool,
}

impl Default for MetricsArgs {
    fn default() -> Self {
        Self {
            idle_seconds: 0.0,
            record_seconds: 0.0,
            config: None,
            frames: DEFAULT_FRAMES,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            fps: DEFAULT_FPS,
            no_engine: false,
        }
    }
}

/// Per-frame statistics of the `preview` phase.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FrameStats {
    /// Frames the statistics are computed over (warmup excluded).
    pub frames: u32,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Frames per second implied by the mean of the slowest 1 % of frames.
    /// `None` when there are no samples to derive it from.
    pub one_percent_low_fps: Option<f64>,
}

/// Parse `metrics` flags (everything after the subcommand).
pub fn parse_metrics_args(args: &[String]) -> Result<MetricsArgs, String> {
    let mut parsed = MetricsArgs::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = |index: &mut usize| -> Result<String, String> {
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| format!("missing value for {flag}"))
        };
        match flag {
            "--idle-seconds" => parsed.idle_seconds = parse_seconds(flag, &value(&mut index)?)?,
            "--record-seconds" => parsed.record_seconds = parse_seconds(flag, &value(&mut index)?)?,
            "-c" | "--config" => parsed.config = Some(PathBuf::from(value(&mut index)?)),
            "--frames" => parsed.frames = parse_u32(flag, &value(&mut index)?, MAX_FRAMES)?,
            "--width" => parsed.width = parse_dimension(flag, &value(&mut index)?)?,
            "--height" => parsed.height = parse_dimension(flag, &value(&mut index)?)?,
            "--fps" => parsed.fps = parse_u32(flag, &value(&mut index)?, MAX_FPS)?,
            "--no-engine" => parsed.no_engine = true,
            other => return Err(format!("unknown flag {other:?} (for `rivulet metrics`)")),
        }
        index += 1;
    }
    validate(&parsed)?;
    Ok(parsed)
}

fn parse_seconds(flag: &str, raw: &str) -> Result<f64, String> {
    let seconds: f64 = raw
        .parse()
        .map_err(|_| format!("{flag}: not a number of seconds ({raw})"))?;
    if !seconds.is_finite() || seconds < 0.0 || seconds > MAX_SECONDS {
        return Err(format!(
            "{flag}: must be between 0 and {MAX_SECONDS} seconds (got {raw})"
        ));
    }
    Ok(seconds)
}

fn parse_u32(flag: &str, raw: &str, max: u32) -> Result<u32, String> {
    let value: u32 = raw
        .parse()
        .map_err(|_| format!("{flag}: not a whole number ({raw})"))?;
    if value == 0 || value > max {
        return Err(format!("{flag}: must be between 1 and {max} (got {raw})"));
    }
    Ok(value)
}

fn parse_dimension(flag: &str, raw: &str) -> Result<u32, String> {
    parse_u32(flag, raw, MAX_DIMENSION)
}

/// Reject combinations the engine cannot satisfy. Doing it here keeps the
/// failure a usage error (exit 2, naming the key) instead of a pipeline error
/// several seconds into a measurement run.
fn validate(args: &MetricsArgs) -> Result<(), String> {
    // The engine's RGBA caps path and x264 both require even dimensions.
    // (`is_multiple_of` is newer than this crate's declared MSRV.)
    if args.width % 2 != 0 || args.height % 2 != 0 {
        let (width, height) = (args.width, args.height);
        return Err(format!(
            "--width/--height: must be even for the capture pipeline (got {width}x{height})"
        ));
    }
    Ok(())
}

/// Run `rivulet metrics`: drive the requested phases, emit the event stream,
/// and return the documented exit code.
pub fn run_metrics(args: &MetricsArgs) -> i32 {
    run_metrics_with(args, emit)
}

fn emit(event: Value) {
    // stdout is a `LineWriter`, so the newline flushes. The explicit flush
    // makes the phase boundaries readable by an external sampler even if that
    // ever stops being true.
    println!("{event}");
    let _ = std::io::stdout().flush();
}

/// The testable core of [`run_metrics`]: every event goes through `sink`, so a
/// test can assert the full stream without capturing a process's stdout.
fn run_metrics_with<F: FnMut(Value)>(args: &MetricsArgs, mut sink: F) -> i32 {
    let mut phases: Vec<Value> = Vec::new();
    let mut frame_times_ms: Vec<f64> = Vec::new();
    let mut frame_stats: Option<FrameStats> = None;
    let mut frames_pushed: Option<u64> = None;
    let mut record_bytes: Option<u64> = None;

    // --- startup ------------------------------------------------------
    sink(json!({"event": "phase_start", "phase": "startup"}));
    let started = Instant::now();
    let (engine_status, engine_reason, mut engine) = bootstrap_engine(args.no_engine);
    let startup_ms = elapsed_ms(started);
    let mut startup_detail = Map::new();
    startup_detail.insert("engine".to_string(), json!(engine_status));
    insert_reason(&mut startup_detail, engine_reason);
    phases.push(phase("startup", startup_ms, startup_detail.clone()));
    sink(json!({
        "event": "phase_end",
        "phase": "startup",
        "duration_ms": startup_ms,
        "engine": engine_status,
        "detail": Value::Object(startup_detail),
    }));

    // --- idle ---------------------------------------------------------
    if args.idle_seconds > 0.0 {
        sink(json!({"event": "phase_start", "phase": "idle"}));
        let started = Instant::now();
        std::thread::sleep(Duration::from_secs_f64(args.idle_seconds));
        let duration_ms = elapsed_ms(started);
        phases.push(phase("idle", duration_ms, Map::new()));
        sink(json!({
            "event": "phase_end",
            "phase": "idle",
            "duration_ms": duration_ms,
        }));
    }

    // --- preview ------------------------------------------------------
    if let Some(path) = args.config.as_ref() {
        sink(json!({"event": "phase_start", "phase": "preview"}));
        let started = Instant::now();
        match crate::render::load_scene_config(path) {
            Ok(config) => {
                let (times, stats) = measure_preview(&config, args.frames);
                frame_times_ms = times;
                frame_stats = Some(stats);
                let duration_ms = elapsed_ms(started);
                let mut detail = Map::new();
                detail.insert("config".to_string(), json!(path.display().to_string()));
                detail.insert("stats".to_string(), json!(frame_stats));
                phases.push(phase("preview", duration_ms, detail.clone()));
                sink(json!({
                    "event": "phase_end",
                    "phase": "preview",
                    "duration_ms": duration_ms,
                    "detail": Value::Object(detail),
                }));
            }
            Err(error) => {
                sink(json!({"event": "error", "phase": "preview", "message": error}));
                eprintln!("error: {error}\n\n{USAGE}");
                return exit_code::USAGE;
            }
        }
    }

    // --- record -------------------------------------------------------
    if args.record_seconds > 0.0 {
        sink(json!({"event": "phase_start", "phase": "record"}));
        let started = Instant::now();
        let mut detail = Map::new();
        let status = match engine.as_mut() {
            Some(engine) => match record_phase(engine, args) {
                Ok((pushed, bytes)) => {
                    frames_pushed = Some(pushed);
                    record_bytes = Some(bytes);
                    detail.insert("frames_pushed".to_string(), json!(pushed));
                    detail.insert("output_bytes".to_string(), json!(bytes));
                    if let Some(error) = engine.take_error() {
                        detail.insert("engine_error".to_string(), json!(error));
                    }
                    "ok"
                }
                Err(error) => {
                    insert_reason(&mut detail, Some(error));
                    "failed"
                }
            },
            None => {
                insert_reason(
                    &mut detail,
                    Some("engine not available (startup reported it)".to_string()),
                );
                "skipped"
            }
        };
        detail.insert("status".to_string(), json!(status));
        let duration_ms = elapsed_ms(started);
        phases.push(phase("record", duration_ms, detail.clone()));
        sink(json!({
            "event": "phase_end",
            "phase": "record",
            "duration_ms": duration_ms,
            "detail": Value::Object(detail),
        }));
    }

    sink(json!({
        "event": "report",
        "schema_version": SCHEMA_VERSION,
        "platform": std::env::consts::OS,
        "engine": engine_status,
        "idle_seconds": args.idle_seconds,
        "record_seconds": args.record_seconds,
        "capture": {"width": args.width, "height": args.height, "fps": args.fps},
        "phases": phases,
        "frame_times_ms": frame_times_ms,
        "frame_stats": frame_stats,
        "frames_pushed": frames_pushed,
        "record_bytes": record_bytes,
    }));
    exit_code::OK
}

/// Construct the engine, or explain why it is not available.
///
/// GStreamer initialisation lives behind `Lazy::force` inside
/// `RivuletEngine::default()`, and it *panics* when the runtime is missing.
/// Catching that turns "this runner cannot measure startup" into a recorded
/// reason instead of a crash whose exit code nobody interprets.
fn bootstrap_engine(no_engine: bool) -> (&'static str, Option<String>, Option<RivuletEngine>) {
    if no_engine {
        return ("skipped", Some("--no-engine".to_string()), None);
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(RivuletEngine::new)) {
        Ok(engine) => ("gstreamer", None, Some(engine)),
        Err(payload) => ("unavailable", Some(panic_message(payload)), None),
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "engine initialisation panicked (no message)".to_string()
}

/// Render `frames` frames through the production render path and reduce them
/// to percentiles.
///
/// One warmup frame is rendered and excluded: the first call pays for the
/// allocator and for cold code, and a 30-frame measurement should describe the
/// steady state, not the first frame. The warmup frame is reported through
/// `FrameStats::frames` staying at the measured count, so the number cannot be
/// confused with the total work done.
fn measure_preview(config: &SceneRenderConfig, frames: u32) -> (Vec<f64>, FrameStats) {
    let _warmup = config.render_frame(0);
    let mut times = Vec::with_capacity(frames as usize);
    for index in 0..frames {
        let started = Instant::now();
        let frame = config.render_frame(u64::from(index));
        times.push(elapsed_ms(started));
        // Drop before the next iteration so the previous frame's buffer is not
        // freed inside the next measurement.
        drop(frame);
    }
    let stats = frame_stats(&times);
    (times, stats)
}

/// Percentiles and the 1 % low over per-frame durations.
///
/// An empty slice yields `NaN` percentiles and `None` for the 1 % low rather
/// than a passing-looking zero: no samples must never read as "fast".
pub fn frame_stats(times: &[f64]) -> FrameStats {
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    FrameStats {
        frames: u32::try_from(times.len()).unwrap_or(u32::MAX),
        p50_ms: percentile(&sorted, 0.50),
        p95_ms: percentile(&sorted, 0.95),
        p99_ms: percentile(&sorted, 0.99),
        max_ms: sorted.last().copied().unwrap_or(f64::NAN),
        one_percent_low_fps: one_percent_low_fps(&sorted),
    }
}

/// Nearest-rank percentile over an ascending slice. `NaN` for an empty slice.
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let last = sorted.len() - 1;
    let index = ((last as f64) * fraction).round() as usize;
    sorted[index.min(last)]
}

/// Frames per second implied by the mean of the slowest 1 % of frames.
///
/// This is the number a game feels: p99 says how bad the tail is, the 1 % low
/// says how much of the second is spent there. `None` when nothing was
/// measured.
fn one_percent_low_fps(ascending: &[f64]) -> Option<f64> {
    if ascending.is_empty() {
        return None;
    }
    let count = ((ascending.len() as f64 / 100.0).ceil() as usize).max(1);
    let slowest = &ascending[ascending.len() - count..];
    let mean = slowest.iter().sum::<f64>() / count as f64;
    if !mean.is_finite() || mean <= 0.0 {
        return None;
    }
    Some(1_000.0 / mean)
}

/// Push a synthetic RGBA frame stream through a real recording pipeline for
/// `--record-seconds`, paced to `--fps`, and report what was produced.
///
/// Returns `(frames pushed, output bytes)`.
fn record_phase(engine: &mut RivuletEngine, args: &MetricsArgs) -> Result<(u64, u64), String> {
    let path = std::env::temp_dir().join(format!(
        "rivulet-metrics-{}-{}.mp4",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_file(&path);

    // Software encoding keeps the measurement comparable between a machine
    // with a hardware encoder and one without; the hardware path is its own
    // benchmark (M9 #360/#362).
    engine.set_video_encoder(VideoEncoder::Software);
    engine.set_audio_enabled(false);
    engine.start_local_recording(path.clone());

    let frame = vec![0u8; (args.width * args.height * 4) as usize];
    let period = Duration::from_secs_f64(1.0 / f64::from(args.fps));
    let started = Instant::now();
    let mut next = Instant::now();
    let mut pushed = 0u64;
    while started.elapsed().as_secs_f64() < args.record_seconds {
        engine.process_raw_frame(&frame, args.width, args.height);
        pushed += 1;
        next += period;
        match next.checked_duration_since(Instant::now()) {
            Some(remaining) => std::thread::sleep(remaining),
            // A late frame must not try to catch up in a burst; that would
            // recreate the overdriven producer the M6 harness had to fix.
            None => next = Instant::now() + period,
        }
    }

    engine.stop_recording();
    let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let _ = std::fs::remove_file(&path);
    Ok((pushed, bytes))
}

fn phase(name: &'static str, duration_ms: f64, detail: Map<String, Value>) -> Value {
    let mut object = Map::new();
    object.insert("name".to_string(), json!(name));
    object.insert("duration_ms".to_string(), json!(duration_ms));
    object.insert("detail".to_string(), Value::Object(detail));
    Value::Object(object)
}

fn insert_reason(detail: &mut Map<String, Value>, reason: Option<String>) {
    if let Some(reason) = reason {
        detail.insert("reason".to_string(), json!(reason));
    }
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn collect(args: &MetricsArgs) -> Vec<Value> {
        let mut events = Vec::new();
        let code = run_metrics_with(args, |event| events.push(event));
        assert_eq!(code, exit_code::OK, "a valid run must succeed");
        events
    }

    fn report(events: &[Value]) -> &Value {
        events
            .iter()
            .find(|event| event["event"] == "report")
            .expect("every run ends with a report event")
    }

    #[test]
    fn defaults_measure_the_engine_and_nothing_else() {
        let parsed = parse_metrics_args(&[]).expect("no flags is valid");
        assert_eq!(parsed.idle_seconds, 0.0);
        assert_eq!(parsed.record_seconds, 0.0);
        assert_eq!(parsed.config, None);
        assert_eq!(parsed.frames, DEFAULT_FRAMES);
        assert!(!parsed.no_engine);
    }

    #[test]
    fn flags_round_trip() {
        let parsed = parse_metrics_args(&args(&[
            "--idle-seconds",
            "60",
            "--record-seconds",
            "30",
            "--config",
            "card.toml",
            "--frames",
            "12",
            "--width",
            "320",
            "--height",
            "240",
            "--fps",
            "60",
            "--no-engine",
        ]))
        .expect("all flags together are valid");
        assert_eq!(parsed.idle_seconds, 60.0);
        assert_eq!(parsed.record_seconds, 30.0);
        assert_eq!(parsed.config, Some(PathBuf::from("card.toml")));
        assert_eq!(parsed.frames, 12);
        assert_eq!((parsed.width, parsed.height, parsed.fps), (320, 240, 60));
        assert!(parsed.no_engine);
    }

    #[test]
    fn odd_geometry_is_a_usage_error_naming_the_flag() {
        // The engine's RGBA caps and x264 both reject odd dimensions; failing
        // here keeps it a usage error instead of a pipeline error.
        let error = parse_metrics_args(&args(&["--width", "321"])).unwrap_err();
        assert!(error.contains("--width/--height"), "got {error}");
        assert!(error.contains("321x360"), "got {error}");
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        for (flag, value) in [
            ("--frames", "0"),
            ("--frames", "6001"),
            ("--fps", "0"),
            ("--fps", "241"),
            ("--width", "9000"),
            ("--idle-seconds", "-1"),
            ("--idle-seconds", "4000"),
            ("--record-seconds", "not-a-number"),
        ] {
            assert!(
                parse_metrics_args(&args(&[flag, value])).is_err(),
                "{flag} {value} must be rejected"
            );
        }
    }

    #[test]
    fn unknown_flags_name_the_subcommand() {
        let error = parse_metrics_args(&args(&["--idle"])).unwrap_err();
        assert!(error.contains("rivulet metrics"), "got {error}");
    }

    #[test]
    fn a_bare_run_reports_startup_and_nothing_measured() {
        // `--no-engine` keeps this test independent of GStreamer: the point
        // here is the event stream, not the environment.
        let parsed = parse_metrics_args(&args(&["--no-engine"])).unwrap();
        let events = collect(&parsed);
        let report = report(&events);

        assert_eq!(report["schema_version"], SCHEMA_VERSION);
        assert_eq!(report["engine"], "skipped");
        let phases = report["phases"].as_array().expect("phases array");
        assert_eq!(phases.len(), 1, "only startup ran");
        assert_eq!(phases[0]["name"], "startup");
        assert!(phases[0]["duration_ms"].as_f64().unwrap() >= 0.0);
        assert_eq!(report["frame_stats"], Value::Null);
        assert_eq!(report["frames_pushed"], Value::Null);
    }

    #[test]
    fn the_event_stream_marks_every_phase_boundary() {
        let parsed =
            parse_metrics_args(&args(&["--no-engine", "--idle-seconds", "0.05"])).expect("valid");
        let events = collect(&parsed);
        let names: Vec<(String, String)> = events
            .iter()
            .filter(|event| event["event"] != "report")
            .map(|event| {
                (
                    event["event"].as_str().unwrap_or_default().to_string(),
                    event["phase"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        assert_eq!(
            names,
            vec![
                ("phase_start".to_string(), "startup".to_string()),
                ("phase_end".to_string(), "startup".to_string()),
                ("phase_start".to_string(), "idle".to_string()),
                ("phase_end".to_string(), "idle".to_string()),
            ],
            "start/end must bracket each phase so an external sampler can attach its samples"
        );
        // The reported idle duration must reflect the request, not the
        // process lifetime.
        let idle = events
            .iter()
            .find(|event| event["event"] == "phase_end" && event["phase"] == "idle")
            .expect("idle ended");
        let duration = idle["duration_ms"].as_f64().expect("duration");
        assert!(
            (40.0..1_000.0).contains(&duration),
            "0.05 s of idling reported as {duration} ms"
        );
    }

    #[test]
    fn idle_alone_pushes_no_frames() {
        let parsed = parse_metrics_args(&args(&["--no-engine", "--idle-seconds", "0.02"])).unwrap();
        let events = collect(&parsed);
        assert_eq!(report(&events)["frames_pushed"], Value::Null);
    }

    #[test]
    fn an_unreadable_preview_config_is_a_usage_error() {
        let parsed = parse_metrics_args(&args(&[
            "--no-engine",
            "--config",
            "definitely-not-here/some-scene.toml",
            "--frames",
            "2",
        ]))
        .expect("the flag itself is valid");
        let mut events = Vec::new();
        let code = run_metrics_with(&parsed, |event| events.push(event));
        assert_eq!(code, exit_code::USAGE, "a bad config is exit 2");
        let error = events
            .iter()
            .find(|event| event["event"] == "error")
            .expect("the failure is reported as an event");
        assert!(
            error["message"].as_str().unwrap().contains("cannot read"),
            "the error must name what failed: {error}"
        );
        assert!(
            !events.iter().any(|event| event["event"] == "report"),
            "a failed run must not emit a report that looks successful"
        );
    }

    #[test]
    fn the_preview_phase_measures_frames_through_the_render_path() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let config = dir.path().join("card.toml");
        std::fs::write(
            &config,
            r#"
name = "MetricsCard"
fps = 30

[canvas]
width = 32
height = 32

[[layers]]
name = "Backdrop"
kind = "Color"
animated = false
"#,
        )
        .expect("write config");

        let parsed = parse_metrics_args(&args(&[
            "--no-engine",
            "--config",
            config.to_str().unwrap(),
            "--frames",
            "5",
        ]))
        .expect("valid");
        let events = collect(&parsed);
        let report = report(&events);

        let times = report["frame_times_ms"].as_array().expect("frame times");
        assert_eq!(times.len(), 5, "one sample per measured frame");
        assert!(times.iter().all(|time| time.as_f64().unwrap() >= 0.0));
        let stats = &report["frame_stats"];
        assert_eq!(stats["frames"], 5, "the warmup frame is not counted");
        let (p50, p99, max) = (
            stats["p50_ms"].as_f64().unwrap(),
            stats["p99_ms"].as_f64().unwrap(),
            stats["max_ms"].as_f64().unwrap(),
        );
        assert!(p50 <= p99, "p50 {p50} above p99 {p99}");
        assert!(p99 <= max, "p99 {p99} above max {max}");
        assert!(
            stats["one_percent_low_fps"].as_f64().unwrap() > 0.0,
            "a measured run must report a 1 % low"
        );

        let preview = report["phases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|phase| phase["name"] == "preview")
            .expect("preview phase recorded");
        assert_eq!(
            preview["detail"]["config"].as_str().unwrap(),
            config.to_str().unwrap()
        );
    }

    #[test]
    fn percentiles_are_nearest_rank_over_the_sorted_samples() {
        let sorted = [1.0, 2.0, 3.0, 4.0, 10.0];
        assert_eq!(percentile(&sorted, 0.50), 3.0);
        assert_eq!(percentile(&sorted, 0.99), 10.0);
        assert_eq!(percentile(&sorted, 1.0), 10.0);
        assert!(percentile(&[], 0.5).is_nan());
    }

    #[test]
    fn stats_sort_their_input_and_do_not_mutate_the_caller() {
        let times = [4.0, 1.0, 3.0, 2.0];
        let stats = frame_stats(&times);
        assert_eq!(times, [4.0, 1.0, 3.0, 2.0], "input stays untouched");
        assert_eq!(stats.frames, 4);
        assert_eq!(stats.p50_ms, 2.0);
        assert_eq!(stats.max_ms, 4.0);
    }

    #[test]
    fn an_empty_measurement_is_not_a_fast_one() {
        let stats = frame_stats(&[]);
        assert_eq!(stats.frames, 0);
        assert!(stats.p50_ms.is_nan());
        assert!(stats.p99_ms.is_nan());
        assert!(stats.max_ms.is_nan());
        assert_eq!(
            stats.one_percent_low_fps, None,
            "no samples must not read as 60 fps"
        );
    }

    #[test]
    fn the_one_percent_low_uses_the_slowest_hundredth() {
        // 100 frames at 10 ms plus one 200 ms frame: ceil(101/100) = 2, so the
        // two slowest are 200 ms and 10 ms -> 105 ms mean -> ~9.5 fps.
        let mut times = vec![10.0; 100];
        times.push(200.0);
        times.sort_by(f64::total_cmp);
        let low = one_percent_low_fps(&times).expect("measured");
        assert!((low - 1_000.0 / 105.0).abs() < 1e-9, "got {low}");

        // A single slow frame in ten is not "the slowest 1 %".
        let uniform = [16.0; 50];
        let low = one_percent_low_fps(&uniform).expect("measured");
        assert!((low - 62.5).abs() < 1e-9, "got {low}");
    }

    #[test]
    fn a_zero_duration_frame_cannot_produce_a_frame_rate() {
        assert_eq!(one_percent_low_fps(&[0.0, 0.0]), None);
    }

    #[test]
    fn panic_payloads_are_turned_into_a_readable_reason() {
        let message = panic_message(Box::new("GStreamer initialization failed.".to_string()));
        assert_eq!(message, "GStreamer initialization failed.");
        let message = panic_message(Box::new(42u8));
        assert!(message.contains("no message"), "got {message}");
    }
}
