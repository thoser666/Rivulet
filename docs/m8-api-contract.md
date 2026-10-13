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

## Runnable examples (#293)

The examples crate is part of the contract: an example that compiles but does
not run is not delivered.

- Six examples in `rivulet-core/examples/`: `simple_record`, `simple_stream`,
  `dual_output`, `encoder_config`, `engine_lifecycle`, `audio_routing` — no
  `egui`/`eframe` anywhere in their call chain.
- CI **runs** every one of them: the `Run rivulet-core examples (M8 #293)`
  step in the Build & Test job (ubuntu, windows, macOS) fails on a non-zero
  exit.
- The `Examples` table in the crate docs (`rivulet-core/src/lib.rs`) and the
  README M8 section both link the overview.
- Guard: `ci_pinning::m8_examples_are_run_by_ci_and_linked_from_crate_docs_and_readme`
  reads the file list from `rivulet-core/examples/` and pins step, crate docs
  and README against it, so a new example cannot skip the run matrix.

Status: verified — all six run locally with exit 0 and the guard is enforced
by the CI pinning job.

Status: verified — the suite passes locally and is enforced by the CI test job.
