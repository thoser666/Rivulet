use rivulet_streaming::VideoEncoder;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use xcap::Monitor;

fn main() -> anyhow::Result<()> {
    println!("🎬 Rivulet Screen Recorder\n");

    // Liste alle Monitors auf
    println!("Available monitors:");
    let monitors = Monitor::all()?;

    for (i, monitor) in monitors.iter().enumerate() {
        println!(
            "  [{}] {} - {}x{} @ ({}, {})",
            i,
            monitor.name().unwrap_or_default(),
            monitor.width().unwrap_or(0),
            monitor.height().unwrap_or(0),
            monitor.x().unwrap_or(0),
            monitor.y().unwrap_or(0)
        );
    }

    // Select the primary monitor
    let monitor = monitors
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| Monitor::all().ok()?.into_iter().next())
        .ok_or_else(|| anyhow::anyhow!("No monitor found"))?;

    let width = monitor.width().unwrap_or(0);
    let height = monitor.height().unwrap_or(0);

    println!(
        "\nRecording monitor: {}",
        monitor.name().unwrap_or_default()
    );
    println!("   Resolution: {}x{}", width, height);
    println!(
        "   Position: ({}, {})",
        monitor.x().unwrap_or(0),
        monitor.y().unwrap_or(0)
    );

    // Create the encoder
    let output_path = PathBuf::from("screen_recording.mp4");
    let fps = 30;
    let duration_secs = 5;

    let mut encoder = VideoEncoder::new(
        &output_path,
        width,
        height,
        fps,
        8_000_000, // 8 Mbps
    )?;

    println!(
        "\n⏺️  Recording for {} seconds at {} FPS...",
        duration_secs, fps
    );
    println!("   (Move your mouse around to see it in the recording!)\n");

    let start = Instant::now();
    let frame_duration = Duration::from_millis(1000 / fps as u64);
    let mut frame_count = 0;
    let mut last_print = Instant::now();

    while start.elapsed().as_secs() < duration_secs {
        let frame_start = Instant::now();

        // Capture frame
        let image = monitor
            .capture_image()
            .map_err(|e| anyhow::anyhow!("Capture failed: {}", e))?;

        // xcap gibt uns ein image::RgbaImage
        let rgba_data = image.as_raw();

        // Konvertiere RGBA -> BGRA (FFmpeg erwartet BGRA)
        let mut bgra_data = Vec::with_capacity(rgba_data.len());
        for pixel in rgba_data.chunks_exact(4) {
            bgra_data.push(pixel[2]); // B
            bgra_data.push(pixel[1]); // G
            bgra_data.push(pixel[0]); // R
            bgra_data.push(pixel[3]); // A
        }

        // Encode frame
        encoder.encode_frame(
            &bgra_data,
            width,
            height,
            width * 4, // stride
        )?;

        frame_count += 1;

        // Progress output (jede Sekunde)
        if last_print.elapsed().as_secs() >= 1 {
            let elapsed = start.elapsed().as_secs_f32();
            let actual_fps = frame_count as f32 / elapsed;
            println!(
                "   {} frames ({:.1}s) - {:.1} FPS",
                frame_count, elapsed, actual_fps
            );
            last_print = Instant::now();
        }

        // Frame-Rate begrenzen
        let frame_time = frame_start.elapsed();
        if frame_time < frame_duration {
            std::thread::sleep(frame_duration - frame_time);
        } else if frame_time > frame_duration * 2 {
            println!(
                "   ⚠️  Warning: Encoding too slow! Frame took {:?}",
                frame_time
            );
        }
    }

    println!("\n⏹️  Stopping recording...");
    encoder.finish()?;

    let actual_duration = start.elapsed().as_secs_f32();
    let avg_fps = frame_count as f32 / actual_duration;

    println!("\n✅ Recording complete!");
    println!("   Output: {:?}", output_path);
    println!("   Frames: {}", frame_count);
    println!("   Duration: {:.2}s", actual_duration);
    println!("   Average FPS: {:.1}", avg_fps);
    println!("\n💡 Play with: ffplay screen_recording.mp4");

    Ok(())
}
