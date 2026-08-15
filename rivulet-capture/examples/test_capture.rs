use rivulet_capture::{CaptureSource, XCapScreenCapture};
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("Listing available monitors...");
    let monitors = XCapScreenCapture::list_monitors()?;
    for monitor in &monitors {
        println!(
            "  [{}] {} - {}x{} at ({}, {}){}",
            monitor.index,
            monitor.name,
            monitor.width,
            monitor.height,
            monitor.x,
            monitor.y,
            if monitor.is_primary { " (PRIMARY)" } else { "" }
        );
    }

    println!("\nInitializing screen capture for primary monitor...");
    let mut capture = XCapScreenCapture::new(0)?;

    let (width, height) = capture.dimensions();
    println!("Capture resolution: {}x{}", width, height);

    capture.start()?;
    println!("Capturing 90 frames (3 seconds at 30fps)...");

    let mut frame_count = 0;
    let start = std::time::Instant::now();

    for i in 0..90 {
        match capture.capture_frame()? {
            Some(frame) => {
                frame_count += 1;
                if frame_count == 1 {
                    println!("First frame captured!");
                    println!("  Resolution: {}x{}", frame.width, frame.height);
                    println!("  Data size: {} bytes", frame.data.len());
                    println!("  Stride: {} bytes", frame.stride);
                }
                if i % 30 == 0 {
                    println!("Frame {}: {}x{}", frame_count, frame.width, frame.height);
                }
            }
            None => {
                println!("No frame captured");
            }
        }
        std::thread::sleep(Duration::from_millis(33)); // ~30fps
    }

    let elapsed = start.elapsed();
    let fps = frame_count as f64 / elapsed.as_secs_f64();
    println!("\n=== Results ===");
    println!(
        "Captured {} frames in {:.2}s",
        frame_count,
        elapsed.as_secs_f64()
    );
    println!("Average FPS: {:.2}", fps);

    capture.stop()?;
    println!("Capture stopped successfully!");
    Ok(())
}
