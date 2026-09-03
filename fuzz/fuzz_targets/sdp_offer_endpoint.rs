//! Fuzz the deterministic SDP generator.
//!
//! The WHIP endpoint string is remote-influenced configuration (entered in
//! Settings / presets). `SdpOffer::h264_opus` derives the session id seed
//! from it and must always produce a well-formed SDP text, whatever bytes
//! the endpoint contains.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rivulet_core::sdp::SdpOffer;

fuzz_target!(|data: &str| {
    let offer = SdpOffer::h264_opus(data);
    let sdp = offer
        .to_sdp()
        .expect("h264_opus offer must always serialize");

    // Structural invariants of the generated offer.
    assert!(sdp.starts_with("v=0\r\n"), "offer must start with v=0");
    assert!(sdp.contains("m=video"), "offer must contain the video m-line");
    assert!(sdp.contains("m=audio"), "offer must contain the audio m-line");
    assert!(
        !sdp.contains('\u{0}'),
        "endpoint bytes must not leak NULs into the SDP"
    );
});
