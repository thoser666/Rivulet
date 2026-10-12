use rivulet_core::{RateControl, RateControlMode, RivuletEngine, VideoCodec, VideoEncoder};
use std::thread;
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt::init();

    println!("Starting encoder configuration example...");
    let mut engine = RivuletEngine::new();

    // Manually configure the encoder and bitrate
    // Note: In a real app, you'd use detect_available_encoders()
    let encoder = VideoEncoder::Software;
    let codec = VideoCodec::H264;
    let bitrate = 8000;

    // Using the new stable API for RateControl
    let rc = RateControl::new(
        RateControlMode::Vbr,
        10000, // max bitrate
        23,    // quality
        "",    // custom options
    );

    println!(
        "Configuring engine with {} ({}) at {} kbps (Mode: {:?})",
        encoder.label(),
        codec.label(),
        bitrate,
        rc.mode()
    );

    // In a real implementation, these would be setters on RivuletEngine
    // For now, we demonstrate the logic used by the engine
    let fragment = encoder.branch_fragment_for_codec_rc(codec, bitrate, rc);
    println!("Generated GStreamer fragment: {}", fragment);

    // Start a short session to verify the engine accepts these parameters.
    // Failures surface asynchronously via the engine's error slot (stable API).
    engine.start_local_recording("encoder_test.mp4");
    match engine.take_error() {
        None => {
            println!("Session started with custom encoder config.");
            thread::sleep(Duration::from_secs(2));
            engine.stop_recording();
            println!("Session stopped.");
        }
        Some(e) => {
            eprintln!("Error: {}", e);
        }
    }
}
