use rivulet_core::{RivuletEngine, StreamSettings};
use std::time::Duration;
use std::thread;

fn main() {
    tracing_subscriber::fmt::init();

    println!("Starting simple streaming example...");
    let mut engine = RivuletEngine::new();

    // Configure stream settings (using a dummy URL for the example)
    let settings = StreamSettings {
        ingest_url: "rtmps://live.twitch.tv/app".to_string(),
        stream_key: "dummy_key_for_example".to_string(),
        ..Default::default()
    };
    engine.set_stream_settings(Some(settings));

    // Start streaming
    match engine.start_streaming() {
        Ok(_) => {
            println!("Streaming started. Streaming for 5 seconds...");
            thread::sleep(Duration::from_secs(5));
            
            println!("Stopping stream...");
            engine.stop_recording(); // stop_recording stops any active session
            println!("Stream stopped.");
        }
        Err(e) => {
            eprintln!("Failed to start streaming: {}. (This is expected if the stream key is invalid)", e);
        }
    }
}
