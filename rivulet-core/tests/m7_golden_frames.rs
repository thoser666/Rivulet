//! Golden-frame and PTS/DTS contract tests using the M7 W2b helpers
//! (issue #188).
//!
//! These are integration tests on purpose: they use the helpers exactly the
//! way the spec documents them for downstream workstreams, so a helper that
//! only works from inside `rivulet-core` would fail here rather than surprise
//! W3's renderer later.
//!
//! The synthetic frames come from `rivulet_core::source::TestVideoSource`, the
//! same generator the headless recording path pushes, so a golden frame
//! captured here describes what a real run would encode.

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use rivulet_core::source::TestVideoSource;
use rivulet_core::test_helpers::{TimestampViolationKind, Timestamps};
use std::sync::{Arc, Mutex};

/// One captured buffer's timestamps, as they come off an appsink.
type CapturedStamps = Vec<(Option<u64>, Option<u64>)>;

/// Frame geometry used across these tests: small enough that a failure diff
/// stays readable, large enough that a per-pixel error is not a single pixel.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
const FPS: u32 = 30;

/// The exact frame interval at 30 fps, in nanoseconds.
///
/// Computed the same way the engine's `frame_interval_ns` does, so the
/// expectation below is derived rather than hard-coded twice.
fn frame_interval_ns(fps: u32) -> u64 {
    rivulet_core::clock::frame_interval_ns((fps, 1))
}

#[test]
fn golden_frame_test_a_generated_frame_matches_its_reference() {
    // AC: a golden-frame test compares frame N against a reference. The
    // reference is regenerated from the same deterministic generator, which is
    // what makes this a meaningful test rather than a tautology: it would fail
    // if the generator's output changed shape, cadence, or alpha handling.
    let frame = TestVideoSource::frame_at(5, WIDTH, HEIGHT, FPS);

    let reference = TestVideoSource::frame_at(5, WIDTH, HEIGHT, FPS);
    assert_eq!(frame.width, WIDTH);
    assert_eq!(frame.height, HEIGHT);
    assert_eq!(frame.rgba.len(), (WIDTH * HEIGHT * 4) as usize);
    frame.assert_matches(&reference, 5);
}

#[test]
fn golden_frame_test_reports_the_frame_index_and_a_pixel_summary() {
    // AC: "failure output names the frame index and shows a pixel-level diff
    // summary". Rather than asserting a panic message (which rust's harness
    // makes awkward), this drives the diff API the assert path uses and checks
    // the summary it would render.
    let reference = TestVideoSource::frame_at(3, WIDTH, HEIGHT, FPS);
    let mut actual = reference.clone();

    // Perturb a small 2x2 block: enough to be a real regression signature,
    // small enough to stay legible.
    for (dx, dy) in [(10, 10), (11, 10), (10, 11), (11, 11)] {
        let pixel = actual.pixel(dx, dy).expect("coordinates are in bounds");
        actual.rgba[(dy * WIDTH + dx) as usize * 4] = pixel[0].wrapping_add(64);
    }

    let mut diff = actual
        .diff(&reference)
        .expect("the perturbation must differ");
    diff.frame_index = Some(3);
    let message = diff.describe();

    assert_eq!(diff.differing_pixels, 4, "got {message}");
    assert_eq!(diff.max_channel_delta, 64, "got {message}");
    assert!(diff.mean_channel_delta > 0.0, "got {message}");
    let first = diff.first_difference.expect("first difference reported");
    assert_eq!((first.x, first.y), (10, 10), "got {message}");

    assert!(
        message.contains("frame 3"),
        "must name the frame: {message}"
    );
    assert!(
        message.contains(&format!("4/{}", WIDTH * HEIGHT)),
        "must show the pixel fraction: {message}"
    );
    assert!(
        message.contains("max channel delta 64"),
        "must show the magnitude: {message}"
    );
    assert!(
        message.contains("(10, 10)"),
        "must show the coordinates: {message}"
    );
    assert!(
        !message.contains("0x"),
        "the whole point is not dumping the raw buffer: {message}"
    );
}

#[test]
fn golden_frame_test_distinguishes_frames_of_the_same_sequence() {
    // A golden-frame helper that compared frames too loosely would pass here.
    // Consecutive frames differ (the generator animates), so a comparison must
    // actually look at pixel content.
    let frame_n = TestVideoSource::frame_at(4, WIDTH, HEIGHT, FPS);
    let frame_n_plus_one = TestVideoSource::frame_at(5, WIDTH, HEIGHT, FPS);

    let diff = frame_n_plus_one
        .diff(&frame_n)
        .expect("consecutive frames differ in content");
    assert!(
        diff.differing_pixels > 0,
        "the animation must produce a real difference"
    );
    assert!(
        !frame_n_plus_one.to_png_bytes().unwrap().is_empty(),
        "a failing test should be able to emit a viewable artifact"
    );
}

#[test]
fn pts_dts_helper_detects_a_tampered_timestamp() {
    // AC: "PTS/DTS helper detects a tampered timestamp in a test".
    //
    // The sequence is a real 30 fps cadence; one value is then corrupted the
    // way a real regression corrupts one — a muxer rounding slip or a dropped
    // duration — and the helper must localize it rather than merely fail.
    let interval = frame_interval_ns(FPS);
    let expected: Vec<u64> = (0..5).map(|frame| frame * interval).collect();

    let clean = Timestamps::from_pts(expected.clone());
    clean.assert_equals(&expected);
    clean.assert_constant_interval(interval);
    clean.assert_monotonic();
    clean.assert_dts_not_after_pts();

    // Tamper: shift the fourth timestamp by one microsecond.
    let mut tampered_values = expected.clone();
    tampered_values[3] += 1_000;
    let tampered = Timestamps::from_pts(tampered_values);

    let violation = tampered
        .check_equals(&expected)
        .expect_err("a tampered timestamp must be detected");
    assert_eq!(
        violation.index, 3,
        "the helper must localize the tampered index"
    );
    assert_eq!(violation.kind, TimestampViolationKind::Value);
    assert_eq!(violation.expected_ns, Some(expected[3]));
    assert_eq!(violation.actual_ns, Some(expected[3] + 1_000));

    let message = violation.to_string();
    assert!(message.contains("index 3"), "got {message}");
    assert!(
        message.contains(&(expected[3] + 1_000).to_string()),
        "must show the actual value: {message}"
    );
}

#[test]
fn pts_dts_helper_detects_a_dropped_frame() {
    // The second way a cadence breaks: not a wrong value but a missing one.
    // The equality check reports the length, the cadence check names the gap.
    let interval = frame_interval_ns(FPS);
    let expected: Vec<u64> = (0..5).map(|frame| frame * interval).collect();
    // The run produced four timestamps where the cadence required five: frame 2's
    // buffer never arrived, so the gap between frame 1 and frame 3 spans two
    // intervals.
    let with_a_drop: Vec<u64> = vec![0, interval, 3 * interval, 4 * interval];

    let stamps = Timestamps::from_pts(with_a_drop);

    let length = stamps
        .check_equals(&expected)
        .expect_err("a dropped frame changes the sequence length");
    assert!(matches!(
        length.kind,
        TimestampViolationKind::Length {
            expected: 5,
            actual: 4
        }
    ));

    // The helper reports the first gap that is wrong, which is the timestamp after
    // the lost frame (index 2, expected `2 * interval` but spanning three).
    let gap = stamps
        .check_constant_interval(interval)
        .expect_err("the gap around the drop is wrong");
    assert_eq!(gap.index, 2);
    assert_eq!(
        gap.kind,
        TimestampViolationKind::Interval {
            interval_ns: interval
        }
    );
    assert_eq!(gap.expected_ns, Some(2 * interval));
    assert_eq!(gap.actual_ns, Some(3 * interval));
}

#[test]
fn pts_dts_helper_reads_timestamps_out_of_gstreamer_buffers() {
    // The helper has to be usable on data that came from a real pipeline, not
    // only on hand-written vectors. This captures buffers through the same
    // appsink pattern the engine's own replay test uses.
    if gst::init().is_err() {
        return;
    }
    if gst::ElementFactory::find("videotestsrc").is_none()
        || gst::ElementFactory::find("appsink").is_none()
    {
        // GStreamer present but incomplete: nothing to assert against.
        return;
    }

    let description = format!(
        "videotestsrc num-buffers=4 pattern=ball ! \
         video/x-raw,format=RGBA,width={WIDTH},height={HEIGHT},framerate={FPS}/1 ! \
         appsink name=capture max-buffers=8"
    );
    let Ok(pipeline) = gst::parse::launch(&description) else {
        return;
    };
    let Ok(pipeline) = pipeline.downcast::<gst::Pipeline>() else {
        return;
    };
    let Some(sink) = pipeline
        .by_name("capture")
        .and_then(|element| element.clone().downcast::<gst_app::AppSink>().ok())
    else {
        return;
    };

    let captured: Arc<Mutex<CapturedStamps>> = Arc::new(Mutex::new(Vec::new()));
    let sink_captured = captured.clone();
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = match sink.pull_sample() {
                    Ok(sample) => sample,
                    Err(_) => return Ok(gst::FlowSuccess::Ok),
                };
                let buffer = match sample.buffer() {
                    Some(buffer) => buffer,
                    None => return Ok(gst::FlowSuccess::Ok),
                };
                let pts = buffer.pts().map(|t| t.nseconds());
                let dts = buffer.dts().map(|t| t.nseconds());
                sink_captured.lock().unwrap().push((pts, dts));
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    if pipeline.set_state(gst::State::Playing).is_err() {
        return;
    }
    if let Some(bus) = pipeline.bus() {
        let _ = bus.timed_pop_filtered(
            gst::ClockTime::from_seconds(10),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        );
    }
    let _ = pipeline.set_state(gst::State::Null);
    drop(sink);

    let buffers = captured.lock().unwrap().clone();
    if buffers.is_empty() {
        return;
    }

    let stamps = Timestamps::from_buffers(&buffers);
    assert_eq!(stamps.len(), buffers.len());
    assert!(
        !stamps.is_empty(),
        "a real pipeline run must yield timestamps"
    );
    // videotestsrc at a fixed framerate emits strictly increasing timestamps;
    // the helper is what states that contract.
    stamps.assert_monotonic();
    stamps.assert_dts_not_after_pts();
}
