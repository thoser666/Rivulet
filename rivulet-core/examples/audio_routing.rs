use rivulet_core::{RecordingPreset, RivuletEngine};
use std::thread;
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt::init();

    println!("Starting audio routing example...");
    let mut engine = RivuletEngine::new();

    // In a real scenario, we would configure audio sources here.
    // Since the current API focuses on the engine as a whole,
    // we demonstrate that the engine can be initialized and
    // started with default audio settings.

    engine.set_preset(RecordingPreset::P1080P30);

    // Failures surface asynchronously via the engine's error slot (stable API).
    engine.start_local_recording("audio_test.mp4");
    if let Some(e) = engine.take_error() {
        eprintln!("Failed to start: {}", e);
        return;
    }

    println!("Recording started. Audio is being routed via default sources.");
    thread::sleep(Duration::from_secs(3));
    engine.stop_recording();
    println!("Recording stopped.");
}
