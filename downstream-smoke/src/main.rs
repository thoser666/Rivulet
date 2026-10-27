//! Downstream-consumer smoke crate (#297 / M8 exit evidence).
//!
//! This crate is deliberately **not** a workspace member and never builds
//! against the working tree. `scripts/run-downstream-smoke.sh` runs
//! `cargo package` on `rivulet-core`, unpacks the `.crate` cargo would upload
//! and builds *this* program against the unpacked artifact. Everything
//! reachable from here is therefore the published file set — no internal
//! paths, no test helpers, no `dev-dependencies`, no extra crates.
//!
//! The event stream `RecordingJob::start` hands out is a `tokio` broadcast
//! receiver. This crate never names that type: it only calls methods on it, so
//! the consumer needs no `tokio` dependency of its own. A consumer that wants
//! to move the receiver to a UI thread would add one — the smoke crate keeps
//! the dependency set minimal on purpose.
//!
//! What the program does, in order:
//!
//! 1. the typed configuration gate accepts a valid configuration and refuses a
//!    broken one with a field-named error, before GStreamer is involved,
//! 2. it records a **video-only** session end to end with frames this crate
//!    produced itself, and checks the returned `SessionReport` and the file on
//!    disk against what it pushed,
//! 3. it attempts the same session **with audio**. A host without the audio
//!    plugins gets the typed `MissingElements` refusal, which is part of the
//!    public contract and is reported as such — unless
//!    `RIVULET_DOWNSTREAM_REQUIRE_AUDIO=1` is set (CI sets it), in which case
//!    a missing audio leg is a failure, so the positive path cannot silently
//!    degrade into a refusal,
//! 4. any deviation exits non-zero. CI keys on the exit status, never on the
//!    printed lines.

use rivulet_core::{
    detect_available_encoders_with_info, AudioSettings, JobError, JobState, RecordingConfig,
    RecordingJob, RecordingJobConfig, SessionEvent, VideoCodec,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Small even geometry: fast to encode, large enough for every codec.
const W: u32 = 320;
const H: u32 = 240;
const FPS: u32 = 30;
/// Audio format this consumer pushes; `AudioSettings` documents the ranges.
const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// 20 ms of interleaved stereo at `RATE`.
const CHUNK_FRAMES: usize = 960;
/// How many video frames and audio chunks each session gets.
const FRAMES: u32 = 30;
const CHUNKS: usize = 4;

fn main() {
    if let Err(problem) = run() {
        eprintln!("downstream smoke FAILED: {problem}");
        std::process::exit(1);
    }
    println!("downstream smoke OK");
}

fn run() -> Result<(), String> {
    check_configuration_gate()?;

    // The video-only session is the one every host can run: it needs no audio
    // plugin, so it is the baseline proof that the packaged crate drives a
    // pipeline from a foreign crate.
    let video_only = temp_path("rivulet_downstream_smoke_video.mp4");
    match run_session(&video_only, false)? {
        SessionOutcome::Ran(report) => println!(
            "video-only session: {} frames, {} bytes on disk",
            report.frames_written, report.bytes_written
        ),
        SessionOutcome::AudioPluginsMissing(elements) => {
            return Err(format!(
                "a video-only session must not need audio plugins, but {elements} were missing"
            ))
        }
    }

    // The audio leg. A host without libav refuses typed; that refusal is a
    // documented part of the contract, so it is reported rather than hidden.
    let with_audio = temp_path("rivulet_downstream_smoke_audio.mp4");
    let audio_proven = match run_session(&with_audio, true)? {
        SessionOutcome::Ran(report) => {
            println!(
                "audio+video session: {} frames, {} audio samples, {} bytes on disk",
                report.frames_written, report.audio_samples, report.bytes_written
            );
            true
        }
        SessionOutcome::AudioPluginsMissing(elements) => {
            println!("audio plugins missing on this host ({elements}); typed refusal verified");
            false
        }
    };

    // Structured feature detection, reachable from a consumer without
    // gstreamer-rs of its own: `RecordingJob::start` initialises GStreamer, so
    // by this point the factories are usable.
    let h264 = detect_available_encoders_with_info(VideoCodec::H264);
    if h264.is_empty() {
        return Err("feature detection returned no H.264 candidate".into());
    }
    if !h264
        .iter()
        .all(|info| info.is_available() || !info.why_not.is_empty())
    {
        return Err("every candidate must be available or carry a machine-readable why_not".into());
    }
    println!("feature detection: {} H.264 candidates", h264.len());

    if require_audio() && !audio_proven {
        return Err(
            "RIVULET_DOWNSTREAM_REQUIRE_AUDIO is set, but the audio session could not run: \
             the host is missing the plugins the public contract says it must have"
                .into(),
        );
    }
    Ok(())
}

/// The gate a foreign crate hits first: typed, field-named, and reachable
/// without touching GStreamer.
fn check_configuration_gate() -> Result<(), String> {
    let config = RecordingJobConfig::file(recording_config(&temp_path("gate.mp4")))
        .with_audio(AudioSettings {
            sample_rate: RATE,
            channels: CHANNELS,
        });
    config
        .validate()
        .map_err(|err| format!("a valid configuration must validate, got: {err}"))?;

    let mut broken = config.clone();
    broken.audio = Some(AudioSettings {
        sample_rate: 0,
        channels: CHANNELS,
    });
    match broken.validate() {
        Err(err) => println!("invalid configuration refused typed: {err}"),
        Ok(()) => return Err("a zero sample rate must be refused".into()),
    }
    Ok(())
}

/// What a session run proved.
enum SessionOutcome {
    /// The session started, took the pushed frames (and samples) and closed
    /// with a report that matched them.
    Ran(SessionEvidence),
    /// The host has no audio plugins, so the audio session refused typed.
    AudioPluginsMissing(String),
}

/// The numbers a completed session reported, after they were checked against
/// what the consumer pushed and what landed on disk.
struct SessionEvidence {
    frames_written: u64,
    audio_samples: u64,
    bytes_written: u64,
}

fn run_session(path: &Path, with_audio: bool) -> Result<SessionOutcome, String> {
    let _ = std::fs::remove_file(path);
    let video = RecordingJobConfig::file(recording_config(path));
    let config = if with_audio {
        video.with_audio(AudioSettings {
            sample_rate: RATE,
            channels: CHANNELS,
        })
    } else {
        video
    };

    let (mut job, mut events) = match RecordingJob::start(config) {
        Ok(session) => session,
        Err(JobError::MissingElements { elements }) if with_audio => {
            return Ok(SessionOutcome::AudioPluginsMissing(elements.join(", ")))
        }
        Err(err) => return Err(format!("session refused to start: {err}")),
    };
    if job.state() != JobState::Recording {
        return Err(format!(
            "a started session must report Recording, got {}",
            job.state()
        ));
    }

    let started = poll_until(
        || events.try_recv().ok(),
        Duration::from_secs(2),
        |event| matches!(event, SessionEvent::Started { .. }),
    );
    if started.is_none() {
        return Err("no Started event within the budget".into());
    }

    // Frames and samples this crate generated itself, so a constant buffer
    // cannot stand in for real input.
    let mut live = job.subscribe();
    for index in 0..FRAMES {
        job.push_frame(&gradient_frame(index))
            .map_err(|err| format!("frame {index} rejected: {err}"))?;
    }
    let pushed_samples = if with_audio {
        let samples = (CHUNKS * CHUNK_FRAMES * CHANNELS as usize) as u64;
        for chunk in 0..CHUNKS {
            job.push_audio(&sine_chunk(chunk))
                .map_err(|err| format!("audio chunk {chunk} rejected: {err}"))?;
        }
        samples
    } else {
        0
    };

    // The status stream carries the session's own counters.
    let progressed = poll_until(
        || live.try_recv().ok(),
        Duration::from_secs(5),
        |event| {
            matches!(
                event,
                SessionEvent::Progress {
                    frames_written,
                    ..
                } if *frames_written > 0
            )
        },
    );
    if progressed.is_none() {
        return Err("no Progress event with frames within the budget".into());
    }

    // Pause and resume are typed state transitions.
    job.pause().map_err(|err| format!("pause refused: {err}"))?;
    if job.state() != JobState::Paused {
        return Err(format!("pause must report Paused, got {}", job.state()));
    }
    job.resume().map_err(|err| format!("resume refused: {err}"))?;

    let report = job.stop().map_err(|err| format!("stop failed: {err}"))?;
    if report.frames_written != FRAMES as u64 {
        return Err(format!(
            "pushed {FRAMES} frames, report says {}",
            report.frames_written
        ));
    }
    if report.audio_samples != pushed_samples {
        return Err(format!(
            "pushed {pushed_samples} audio samples, report says {}",
            report.audio_samples
        ));
    }
    if report.bytes_written == 0 {
        return Err("the report recorded no bytes written".into());
    }
    if report.path.as_deref() != Some(path) {
        return Err(format!("report path mismatch: {:?}", report.path));
    }
    let on_disk = std::fs::metadata(path)
        .map_err(|err| format!("output file missing at {}: {err}", path.display()))?
        .len();
    if on_disk == 0 {
        return Err("output file is empty".into());
    }
    let _ = std::fs::remove_file(path);

    Ok(SessionOutcome::Ran(SessionEvidence {
        frames_written: report.frames_written,
        audio_samples: report.audio_samples,
        bytes_written: on_disk,
    }))
}

fn recording_config(path: &Path) -> RecordingConfig {
    RecordingConfig {
        width: W,
        height: H,
        fps: FPS,
        path: Some(path.display().to_string()),
        ..Default::default()
    }
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

/// Whether a missing audio leg must fail the smoke run. CI sets this so the
/// typed refusal cannot quietly replace the positive evidence.
fn require_audio() -> bool {
    matches!(
        std::env::var("RIVULET_DOWNSTREAM_REQUIRE_AUDIO").as_deref(),
        Ok("1") | Ok("true")
    )
}

/// Poll the session's event stream until `accept` matches or `budget` runs out.
///
/// `next` is the caller's own non-blocking poll (`receiver.try_recv().ok()`).
/// Taking it as a closure rather than the receiver itself keeps the `tokio`
/// broadcast type out of this crate's source and out of its dependencies (see
/// the module docs).
fn poll_until<F>(
    mut next: impl FnMut() -> Option<SessionEvent>,
    budget: Duration,
    mut accept: F,
) -> Option<SessionEvent>
where
    F: FnMut(&SessionEvent) -> bool,
{
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        match next() {
            Some(event) if accept(&event) => return Some(event),
            Some(_) => continue,
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    None
}

/// An RGBA frame with a per-frame colour ramp, so the bytes pushed differ from
/// frame to frame and a constant buffer cannot pass for real input.
fn gradient_frame(index: u32) -> Vec<u8> {
    let mut frame = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            frame.push((x + index * 8) as u8);
            frame.push((y + index * 8) as u8);
            frame.push((index * 8) as u8);
            frame.push(255);
        }
    }
    frame
}

/// 20 ms of a 440 Hz sine, interleaved at `CHANNELS`, as interleaved `f32`.
fn sine_chunk(chunk: usize) -> Vec<f32> {
    let mut samples = Vec::with_capacity(CHUNK_FRAMES * CHANNELS as usize);
    for frame in 0..CHUNK_FRAMES {
        let t = (chunk * CHUNK_FRAMES + frame) as f32 / RATE as f32;
        let value = (t * 440.0 * std::f32::consts::TAU).sin() * 0.25;
        for _ in 0..CHANNELS {
            samples.push(value);
        }
    }
    samples
}
