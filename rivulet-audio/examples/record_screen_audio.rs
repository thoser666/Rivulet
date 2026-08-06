//! End-to-end example: capture the screen together with system audio and
//! microphone input (mixed) and write an MP4 file containing both video and
//! audio tracks.
//!
//! Usage: `cargo run --example record_screen_audio`
//!
//! Set `RIVULET_RECORD_SECS` to control the recording length (default: 10s).

use anyhow::Context;
use rivulet_audio::{AudioCapture, AudioConfig};
use rivulet_core::{AudioFrame, RivuletEngine};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use xcap::Monitor;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let record_secs: u64 = std::env::var("RIVULET_RECORD_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);

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
        "Recording monitor: {} ({}x{})",
        monitor.name().unwrap_or_default(),
        width,
        height
    );

    let output_path = std::env::temp_dir().join(format!(
        "rivulet_screen_audio_{}.mp4",
        chrono_like_timestamp()
    ));
    println!("Output: {}", output_path.display());

    // 1. Build the recording engine with audio enabled.
    let mut engine = RivuletEngine::default();
    engine.set_audio_enabled(true);

    // 2. Build the audio capture (system audio + microphone, mixed).
    let audio_config = AudioConfig {
        capture_system: true,
        capture_mic: true,
        system_volume: 0.8,
        mic_volume: 1.0,
        ..Default::default()
    };
    let mut audio = AudioCapture::new(audio_config).context("Failed to create audio capture")?;

    // Bridge from the capture thread to the recording loop.
    let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>();
    audio.start(Box::new(move |frame| {
        let _ = audio_tx.send(frame);
    }))?;
    println!("Audio capture started (system + mic mixed).");

    // 3. Start recording.
    engine.start_local_recording(output_path.clone());

    let start = Instant::now();
    let frame_duration = Duration::from_millis(1000 / fps as u64);
    let mut frame_count = 0u64;
    let mut audio_frames = 0u64;

    while start.elapsed().as_secs() < record_secs {
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

        frame_count += 1;

        let frame_time = frame_start.elapsed();
        if frame_time < frame_duration {
            std::thread::sleep(frame_duration - frame_time);
        }
    }

    println!("Stopping recording...");
    engine.stop_recording();
    audio.stop()?;

    println!("Recording complete.");
    println!("  Video frames: {}", frame_count);
    println!("  Audio frames: {}", audio_frames);
    println!("  Output file: {}", output_path.display());

    Ok(())
}

fn chrono_like_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple UTC-ish timestamp without pulling in chrono.
    let days = now / 86400;
    let secs_of_day = now % 86400;
    let h = secs_of_day / 3600;
    let m = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    format!("{}-{}-{}-{}", days, h, m, s)
}
