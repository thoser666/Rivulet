use rivulet_core::{RivuletEngine, RecordingPreset, StreamSettings};
use std::time::Duration;
use std::thread;

fn main() {
    tracing_subscriber::fmt::init();

    println!("Starting dual output example (Record + Stream)...");
    let mut engine = RivuletEngine::new();

    // 1. Configure Recording
    engine.set_preset(RecordingPreset::Standard);

    // 2. Configure Streaming
    let settings = StreamSettings {
        ingest_url: "rtmps://live.twitch.tv/app".to_string(),
        stream_key: "dummy_key_for_example".to_string(),
        ..Default::default()
    };
    engine.set_stream_settings(Some(settings));

    // Start both simultaneously
    match engine.start_local_recording("dual_output.mp4") {
        Ok(_) => {
            println!("Dual output started. Recording to dual_output.mp4 and streaming...");
            thread::sleep(Duration::from_secs(5));
            
            println!("Stopping dual session...");
            engine.stop_recording();
            println!("Session stopped.");
        }
        Err(e) => {
            eprintln!("Failed to start dual output: {}", e);
            std::process::exit(1);
        }
    }
}
