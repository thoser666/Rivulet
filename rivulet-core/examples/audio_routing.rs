use rivulet_core::{RivuletEngine, RecordingPreset};
use std::time::Duration;
use std::thread;

fn main() {
    tracing_subscriber::fmt::init();

    println!("Starting audio routing example...");
    let mut engine = RivuletEngine::new();

    // In a real scenario, we would configure audio sources here.
    // Since the current API focuses on the engine as a whole, 
    // we demonstrate that the engine can be initialized and 
    // started with default audio settings.
    
    engine.set_preset(RecordingPreset::Standard);

    match engine.start_local_recording("audio_test.mp4") {
        Ok(_) => {
            println!("Recording started. Audio is being routed via default sources.");
            thread::sleep(Duration::from_secs(3));
            engine.stop_recording();
            println!("Recording stopped.");
        }
        Err(e) => {
            eprintln!("Failed to start: {}", e);
        }
    }
}
