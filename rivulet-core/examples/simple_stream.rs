use rivulet_core::{RivuletEngine, StreamSettings};
use std::thread;
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt::init();

    println!("Starting simple streaming example...");
    let mut engine = RivuletEngine::new();

    // Configure stream settings (using a dummy URL for the example).
    // StreamSettings has no Default impl by design — the constructors carry
    // the per-platform ingest URL so a custom URL can't drift from the
    // platform's schema.
    let settings = StreamSettings::twitch("dummy_key_for_example");
    engine.set_stream_settings(Some(settings));

    // Start streaming. Failures surface asynchronously via take_error().
    engine.start_streaming();
    if let Some(e) = engine.take_error() {
        eprintln!(
            "Failed to start streaming: {}. (This is expected if the stream key is invalid)",
            e
        );
        return;
    }

    println!("Streaming started. Streaming for 5 seconds...");
    thread::sleep(Duration::from_secs(5));

    println!("Stopping stream...");
    engine.stop_recording(); // stop_recording stops any active session
    println!("Stream stopped.");
}
