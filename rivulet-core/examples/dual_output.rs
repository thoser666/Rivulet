use rivulet_core::{RecordingPreset, RivuletEngine, StreamSettings};
use std::thread;
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt::init();

    println!("Starting dual output example (Record + Stream)...");
    let mut engine = RivuletEngine::new();

    // 1. Configure Recording
    engine.set_preset(RecordingPreset::P1080P30);

    // 2. Configure Streaming (StreamSettings has no Default impl by design —
    //    the constructors carry the per-platform ingest URL).
    let settings = StreamSettings::twitch("dummy_key_for_example");
    engine.set_stream_settings(Some(settings));

    // Start both simultaneously. Failures surface asynchronously via the
    // engine's error slot (stable API).
    engine.start_local_recording("dual_output.mp4");
    if let Some(e) = engine.take_error() {
        eprintln!("Failed to start dual output: {}", e);
        std::process::exit(1);
    }

    println!("Dual output started. Recording to dual_output.mp4 and streaming...");
    thread::sleep(Duration::from_secs(5));

    println!("Stopping dual session...");
    engine.stop_recording();
    println!("Session stopped.");
}
