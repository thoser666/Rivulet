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

- Seven examples in `rivulet-core/examples/`: `simple_record`, `simple_stream`,
  `dual_output`, `encoder_config`, `engine_lifecycle`, `audio_routing`,
  `session_embed` — no `egui`/`eframe` anywhere in their call chain.
- CI **runs** every one of them: the `Run rivulet-core examples (M8 #293)`
  step in the Build & Test job (ubuntu, windows, macOS) fails on a non-zero
  exit.
- The `Examples` table in the crate docs (`rivulet-core/src/lib.rs`) and the
  README M8 section both link the overview.
- Guard: `ci_pinning::m8_examples_are_run_by_ci_and_linked_from_crate_docs_and_readme`
  reads the file list from `rivulet-core/examples/` and pins step, crate docs
  and README against it, so a new example cannot skip the run matrix.

Status: verified — all seven run locally with exit 0 and the guard is enforced
by the CI pinning job.

Status: verified — the suite passes locally and is enforced by the CI test job.

## In-process session contract (#294)

The embedding promise of #294 is part of the contract: a foreign crate must
be able to run a whole recording session without starting the Rivulet app,
and every refusal must be typed before GStreamer is involved.

- Entry point: `RecordingJob` (`rivulet-core/src/recording_job.rs`) —
  `start()` validates the config, preflights the required elements and
  builds the pipeline; the embedder then `push_frame()`s / `push_audio()`es,
  subscribes to `SessionEvent`s, `pause()`s / `resume()`s and `stop()`s —
  or just drops the handle, which finalizes the output like `stop()`.
- Typed contract: every refusal is a `JobError` — `Config` (field-named
  via `RecordingConfigError`), `MissingElements` (element names, from a
  `make`-based preflight that also catches stale registries), `WrongState`
  (operation + current `JobState`), `PushFailed` (expected vs. got frame
  size), plus `Pipeline`/`Gstreamer` for environment failures.
- Inspectability: `RecordingJobConfig::pipeline_description()` exposes the
  exact `parse_launch` string; it embeds the stream key, so redact with
  `redact_pipeline_for_log` before logging.
- Contract proof: `rivulet-core/tests/recording_job.rs` (27 tests, wired as
  an explicit `[[test]]` target in `rivulet-core/Cargo.toml`) pins typed
  validation per parameter, the pipeline fragments, the event stream
  (Started/Progress/Paused/Resumed/Stopped), pause/resume push gating, the
  stop counters and the Drop-finalize (moov atom present) — plus
  `rivulet-core/examples/session_embed.rs`, which CI runs with the other
  examples.

Status: verified — the suite passes locally (27/27) and the example runs
with exit 0; enforced by the CI test and example-run jobs.
