//! Fuzz the SHA256SUMS manifest parser.
//!
//! The manifest is downloaded from the release page before every update
//! install. Arbitrary text must parse to `Some` entries or none — never a
//! panic — and every parsed digest/name pair must be non-empty.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rivulet_updater::parse_checksums;

fuzz_target!(|data: &str| {
    for (name, digest) in parse_checksums(data) {
        assert!(!name.is_empty(), "parser emitted an empty asset name");
        assert!(!digest.is_empty(), "parser emitted an empty digest");
        assert!(
            !name.contains('\u{0}') && !digest.contains('\u{0}'),
            "NUL byte leaked into a parsed checksum entry"
        );
    }
});
