//! Fuzz the updater's GitHub release JSON parser.
//!
//! The release list comes from the network. Malformed JSON must surface as
//! `Err`, not a panic; valid-but-hostile JSON (wrong types, huge tags,
//! negative sizes) must never break the SemVer comparison.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rivulet_updater::parse_latest_release;

fuzz_target!(|data: &[u8]| {
    // 0.5.0-alpha.1 is the current version used by the unit tests.
    let _ = parse_latest_release(data, "0.5.0-alpha.1");
});
