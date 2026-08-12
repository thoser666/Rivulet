//! End-to-end example: capture the screen together with mixed system and
//! microphone audio and stream it over RTMPS to Twitch, Kick or YouTube.
//!
//! Usage: `cargo run --example stream_rtmps`
//!
//! Configuration via environment variables:
//! - `RIVULET_STREAM_PLATFORM` = `twitch` | `kick` | `youtube` | `custom`
//!   (default: `twitch`)
//! - `RIVULET_STREAM_URL`     = custom ingest base URL (required for `custom`,
//!   optional for `kick`)
//! - `RIVULET_STREAM_KEY`     = the stream key from the platform dashboard
//!   (required)
//! - `RIVULET_STREAM_SECS`    = stream duration in seconds (default: 10)
//! - `RIVULET_STREAM_RECORD_PATH` = optional local MP4 path; when set the
//!   engine records and streams simultaneously (dual output)
//!
//! Kick uses a streamer-specific AWS IVS ingest endpoint, e.g.
//! `rtmps://<id>.global-contribute.live-video.net/app` - set it via
//! `RIVULET_STREAM_URL`.

use anyhow::Context;
use rivulet_audio::{AudioCapture, AudioConfig};
use rivulet_core::{AudioFrame, RivuletEngine, StreamSettings};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use xcap::Monitor;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let stream_secs: u64 = std::env::var("RIVULET_STREAM_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);

    let stream_key = std::env::var("RIVULET_STREAM_KEY")
        .context("RIVULET_STREAM_KEY must be set to the platform stream key")?;
    let platform = std::env::var("RIVULET_STREAM_PLATFORM")
        .unwrap_or_else(|_| "twitch".to_string())
        .to_lowercase();

    let settings = match platform.as_str() {
        "twitch" => StreamSettings::twitch(&stream_key),
        "youtube" => StreamSettings::youtube(&stream_key),
        "kick" => {
            let url = std::env::var("RIVULET_STREAM_URL").context(
                "RIVULET_STREAM_URL must be set to the Kick ingest endpoint, \
                 e.g. rtmps://<id>.global-contribute.live-video.net/app",
            )?;
            StreamSettings::kick(&url, &stream_key)
        }
        "custom" => {
            let url = std::env::var("RIVULET_STREAM_URL")
                .context("RIVULET_STREAM_URL must be set for a custom ingest")?;
            StreamSettings::custom(&url, &stream_key)
        }
        other => {
            anyhow::bail!("Unbekannte Plattform: {other}. Erlaubt: twitch, kick, youtube, custom")
        }
    };

    println!(
        "Streaming to {} via {} (RTMPS: {})",
        settings.platform.label(),
        settings.location(),
        settings.is_rtmps()
    );

    let monitor = Monitor::all()
        .context("Failed to list monitors")?
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| Monitor::all().ok()?.into_iter().next())
        .context("No monitor available")?;

    let width = monitor.width().unwrap_or(0);
    let height = monitor.height().unwrap_or(0);
    let fps = 30;
    println!(
        "Capturing monitor: {} ({}x{})",
        monitor.name().unwrap_or_default(),
        width,
        height
    );

    // 1. Build the streaming engine with audio enabled.
    let mut engine = RivuletEngine::default();
    engine.set_audio_enabled(true);
    engine.set_stream_settings(Some(settings));

    // Optional local recording while streaming (dual output).
    let mut record_path = None;
    if let Ok(path) = std::env::var("RIVULET_STREAM_RECORD_PATH") {
        let path = std::path::PathBuf::from(path);
        engine.start_local_recording(path.clone());
        record_path = Some(path);
        println!(
            "Dual output: recording locally to {}",
            record_path.as_ref().unwrap().display()
        );
    } else {
        engine.start_streaming();
    }

    // 2. Build the audio capture (system audio + microphone, mixed).
    let audio_config = AudioConfig {
        capture_system: true,
        capture_mic: true,
        system_volume: 0.8,
        mic_volume: 1.0,
        ..Default::default()
    };
    let mut audio = AudioCapture::new(audio_config).context("Failed to create audio capture")?;

    // Bridge from the capture thread to the streaming loop.
    let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>();
    audio.start(Box::new(move |frame| {
        let _ = audio_tx.send(frame);
    }))?;
    println!("Audio capture started (system + mic mixed).");

    let start = Instant::now();
    let frame_duration = Duration::from_millis(1000 / fps as u64);
    let mut frame_count = 0u64;
    let mut audio_frames = 0u64;
    let mut last_health_print = Instant::now();

    while start.elapsed().as_secs() < stream_secs {
        let frame_start = Instant::now();

        // Screen capture -> engine (also starts the pipeline on first frame).
        let image = monitor
            .capture_image()
            .context("Failed to capture screen")?;
        engine.process_raw_frame(image.as_raw(), image.width(), image.height());

        // Audio capture -> engine.
        while let Ok(frame) = audio_rx.try_recv() {
            let _ = engine.push_audio_frame(&frame);
            audio_frames += 1;
        }

        // Print live stream health every two seconds.
        if last_health_print.elapsed() >= Duration::from_secs(2) {
            let stats = engine.stream_stats();
            println!(
                "[health] {:?} | {:.0} kbps | {:.1} fps | {} sent / {} dropped ({:.1}%)",
                stats.status,
                stats.kbps,
                stats.fps,
                stats.frames_sent,
                stats.frames_dropped,
                stats.dropped_ratio * 100.0
            );
            last_health_print = Instant::now();
        }

        frame_count += 1;

        let frame_time = frame_start.elapsed();
        if frame_time < frame_duration {
            std::thread::sleep(frame_duration - frame_time);
        }
    }

    println!("Stopping stream...");
    engine.stop_recording();
    audio.stop()?;

    let final_stats = engine.stream_stats();
    println!("Stream complete.");
    println!("  Video frames: {}", frame_count);
    println!("  Audio frames: {}", audio_frames);
    println!(
        "  Final health: {:?} ({} sent / {} dropped)",
        final_stats.status, final_stats.frames_sent, final_stats.frames_dropped
    );
    if let Some(path) = record_path {
        println!("  Local recording: {}", path.display());
    }

    Ok(())
}
