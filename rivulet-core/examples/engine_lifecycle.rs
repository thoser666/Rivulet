use rivulet_core::RivuletEngine;
use std::thread;
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt::init();

    println!("Starting engine lifecycle example...");
    let mut engine = RivuletEngine::new();

    // 1. Start
    //
    // `start_local_recording` reports failures asynchronously through the
    // engine's error slot (stable API) rather than a return value.
    println!("Attempting to start recording...");
    engine.start_local_recording("lifecycle_test.mp4");
    if let Some(e) = engine.take_error() {
        eprintln!(
            "Start failed: {}. This may happen if GStreamer is not configured.",
            e
        );
        return;
    }
    println!("Engine is now RECORDING.");

    // 2. Monitor
    thread::sleep(Duration::from_secs(2));
    if let Some(err) = engine.take_error() {
        println!("Engine reported an error during session: {}", err);
    } else {
        println!("Engine is operating normally.");
    }

    // 3. Stop
    println!("Stopping engine...");
    engine.stop_recording();
    println!("Engine stopped successfully.");
}
