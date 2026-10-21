//! #294 session embedding: record from a foreign crate **without starting
//! the Rivulet app**.
//!
//! Demonstrates the full embed contract in one run:
//! typed configuration gate → `RecordingJob::start` → the event stream
//! (`Started`, live `Progress` ticks via `subscribe`) → pause/resume →
//! `stop()` consuming the handle and returning the session report.
//!
//! Audio is one builder call away (`RecordingJobConfig::with_default_audio`);
//! this example stays video-only so it runs on every CI host.

use rivulet_core::{RecordingConfig, RecordingJob, RecordingJobConfig, SessionEvent};
use std::time::{Duration, Instant};

/// Small even geometry: fast to encode, large enough for every codec here.
const W: u32 = 320;
const H: u32 = 240;
const FPS: u32 = 30;

fn main() {
    tracing_subscriber::fmt::init();

    // 1. Typed configuration — invalid values are refused here, before any
    //    GStreamer work happens.
    let path = std::env::temp_dir().join("rivulet_session_embed.mp4");
    let recording = RecordingConfig {
        width: W,
        height: H,
        fps: FPS,
        path: Some(path.display().to_string()),
        ..Default::default()
    };
    let config = RecordingJobConfig::file(recording);
    let frame = vec![0u8; (W * H * 4) as usize];

    // 2. Start the session. On a host without the required plugins this
    //    answers with a typed `JobError` naming the missing elements — a
    //    foreign crate never has to parse logs to learn why.
    let (mut job, mut events) = match RecordingJob::start(config) {
        Ok(session) => session,
        Err(err) => {
            eprintln!("session refused: {err}");
            return;
        }
    };
    println!("session started, state: {}", job.state());

    // 3. The handle's own receiver already carries `Started`...
    match poll_event(&mut events, Duration::from_secs(2)) {
        Some(SessionEvent::Started { path }) => println!("recording to {path:?}"),
        other => println!("first event: {other:?}"),
    }

    // ...while a later subscriber (a UI thread, say) sees live events only.
    let mut live = job.subscribe();

    // 4. Push RGBA frames the embedder produced itself.
    for _ in 0..30 {
        job.push_frame(&frame).expect("320x240 RGBA frame");
    }

    // 5. The status stream: `Progress` ticks carry the session counters.
    match poll_event(&mut live, Duration::from_secs(3)) {
        Some(SessionEvent::Progress {
            frames_written,
            bytes_written,
            ..
        }) => println!("live progress: {frames_written} frames, {bytes_written} bytes"),
        other => println!("status stream: {other:?}"),
    }

    // 6. Pause and resume are typed state transitions.
    job.pause().expect("pause from Recording");
    job.resume().expect("resume from Paused");

    // 7. `stop()` consumes the handle and finalizes the file.
    let report = job.stop().expect("clean stop");
    println!(
        "session stopped: {} frames -> {}",
        report.frames_written,
        report
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(stream only)".to_string())
    );

    let _ = std::fs::remove_file(&path);
}

/// Poll a broadcast receiver from a plain `main` (no Tokio runtime).
fn poll_event(
    rx: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
    budget: Duration,
) -> Option<SessionEvent> {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        match rx.try_recv() {
            Ok(event) => return Some(event),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => return None,
        }
    }
    None
}
