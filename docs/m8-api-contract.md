# M8 API Contract

M8 objective: protect the Rivulet engine public API against silent drift.

Scope:
- `rivulet-core` public API surface: `RivuletEngine`, `OutputSettings`, `RateControl`, `RateControlMode`, `VideoEncoder`, `VideoCodec`, `StreamSettings`, `RecordingPreset`, `PresenceActivity`, `PresenceStatus`, and the stable test-helpers contract used by the regression suite.
- Contract proof: `rivulet-core/tests/api_stability.rs` (wired as an explicit `[[test]]` target in `rivulet-core/Cargo.toml`).
- Documentation: this file, plus the regression-test policy already covers the new safeguard wrappers.

Verification:
- `cargo test -p rivulet-core --test api_stability` — passes (2 tests: lifecycle defaults and encapsulation guard).
- The contract tests compile and pass on the CI toolchain pinned in `rivulet-core/Cargo.toml`.

Contract guards covered by the suite:
- **Lifecycle defaults** — `RivuletEngine::new()` stays callable; `OutputSettings::default()` keeps 1920×1080; `RateControl::default()` stays CBR/q23; codec/encoder labels (`H.264 (AVC)`, `Software`, `Software (H.264 (AVC))`) stay stable so UI strings and serialization do not break silently.
- **Intentional encapsulation** — stability-sensitive fields (`OutputSettings.width`, `RateControl.mode`, …) stay private. Widening visibility is a contract regression that downstream consumers would only notice later.

Status: verified — the suite passes locally and is enforced by the CI test job.
