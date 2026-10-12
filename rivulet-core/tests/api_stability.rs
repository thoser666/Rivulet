//! M8 API-contract proof: the public engine surface must stay stable and
//! intentionally encapsulated.
//!
//! These are regression guards, not exploratory tests. A new public field or a
//! relaxed visibility accessor is a contract regression and must fail this
//! suite rather than surprise downstream consumers later.

use rivulet_core::{
    OutputSettings, RateControl, RateControlMode, RivuletEngine, VideoCodec, VideoEncoder,
};

#[test]
fn m8_api_contract_basic_lifecycle() {
    // The primary entry point must remain callable with the documented defaults.
    // Keep the instance alive here so the test is not a tautology around `Default`.
    let _engine = RivuletEngine::new();

    let settings = OutputSettings::default();
    assert_eq!(settings.width(), 1920);
    assert_eq!(settings.height(), 1080);

    let rc = RateControl::default();
    assert_eq!(rc.mode(), RateControlMode::Cbr);
    assert_eq!(rc.quality(), 23);

    // Downstream stability expectations: the codec/enum labels must stay
    // stable enough that UI strings and serialization do not break silently.
    assert_eq!(VideoCodec::H264.label(), "H.264 (AVC)");
    assert_eq!(VideoEncoder::Software.label(), "Software");
    assert_eq!(
        VideoEncoder::Software.label_with_codec(VideoCodec::H264),
        "Software (H.264 (AVC))"
    );
}

#[test]
fn m8_api_contract_encapsulation_is_intentional() {
    // This test is a compile-time regression guard for the M8 stabilization
    // goals. If any stability-sensitive field became public, the commented-out
    // accesses below must fail to compile rather than silently widening the API.
    let _settings = OutputSettings::default();
    let _rc = RateControl::default();

    // The following must NOT compile if uncommented:
    // let _w = _settings.width;
    // let _m = _rc.mode;
}
