use rivulet_core::{RecordingPreset, RivuletEngine};
use std::thread;
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt::init();

    println!("Starting simple recording example...");
    let mut engine = RivuletEngine::new();

    // Configure a basic preset
    engine.set_preset(RecordingPreset::P1080P30);

    // Start local recording
    engine.start_local_recording("test_recording.mp4");

    println!("Recording started successfully. Recording for 5 seconds...");
    thread::sleep(Duration::from_secs(5));

    println!("Stopping recording...");
    engine.stop_recording();
    println!("Recording stopped. File saved as test_recording.mp4");
}
