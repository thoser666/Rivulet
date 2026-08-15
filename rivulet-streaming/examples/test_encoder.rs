use rivulet_streaming::VideoEncoder;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    println!("Testing FFmpeg encoder...");

    // Create a simple test frame (red screen)
    let width = 1280;
    let height = 720;
    let stride = width * 4;

    let mut frame_data = vec![0u8; (stride * height) as usize];

    // Encoder erstellen
    let mut encoder = VideoEncoder::new(
        &PathBuf::from("test_output.mp4"),
        width,
        height,
        30,
        5_000_000,
    )?;

    println!("Encoding 90 frames (3 seconds)...");

    // Encode 90 frames (3 seconds at 30fps)
    for i in 0..90 {
        // Wechsle Farben: Rot -> Grün -> Blau
        let color = match (i / 30) % 3 {
            0 => (255, 0, 0), // Rot
            1 => (0, 255, 0), // Grün
            _ => (0, 0, 255), // Blau
        };

        for pixel in frame_data.chunks_mut(4) {
            pixel[0] = color.2; // B
            pixel[1] = color.1; // G
            pixel[2] = color.0; // R
            pixel[3] = 255; // A
        }

        encoder.encode_frame(&frame_data, width, height, stride)?;

        if (i + 1) % 30 == 0 {
            println!("Encoded {} frames", i + 1);
        }
    }

    println!("Finishing encoding...");
    encoder.finish()?;

    println!("✅ Done! Check test_output.mp4");
    Ok(())
}
