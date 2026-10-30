# M8 API Contract

M8 objective: protect the Rivulet engine public API against silent drift.

Scope:
- `rivulet-core` public API surface: `RivuletEngine`, `OutputSettings`, `RateControl`, `RateControlMode`, `VideoEncoder`, `VideoCodec`, `StreamSettings`, `RecordingPreset`, `PresenceActivity`, `PresenceStatus`, and the stable test-helpers contract used by the regression suite.
- Contract proof: `rivulet-core/tests/api_stability.rs` (wired as an explicit `[[test]]` target in `rivulet-core/Cargo.toml`).
- Documentation: this file, plus the regression-test policy already covers the new safeguard wrappers.

Verification:
- `cargo test -p rivulet-core --test api_stability` — passes (5 tests: lifecycle defaults, encapsulation guard, the removal guard, the deprecation-note guard and the policy-link guard; the last three are described under [Deprecation and migration policy](#deprecation-and-migration-policy-344)).
- The contract tests compile and pass on the CI toolchain pinned in `rivulet-core/Cargo.toml`.

Contract guards covered by the suite:
- **Lifecycle defaults** — `RivuletEngine::new()` stays callable; `OutputSettings::default()` keeps 1920×1080; `RateControl::default()` stays CBR/q23; codec/encoder labels (`H.264 (AVC)`, `Software`, `Software (H.264 (AVC))`) stay stable so UI strings and serialization do not break silently.
- **Intentional encapsulation** — stability-sensitive fields (`OutputSettings.width`, `RateControl.mode`, …) stay private. Widening visibility is a contract regression that downstream consumers would only notice later.
- **Removal guard** — every crate-root name in `rivulet-core/tests/api_surface_baseline.txt` (the surface of the last released line) that is gone from `docs/api-overview.md` needs a row in the *Applied migrations* table of [`api-migration.md`](api-migration.md) with a replacement and a migration note.
- **Deprecation-note guard** — every `#[deprecated]` attribute in `rivulet-core/src` carries `since` and a non-empty `note` naming the replacement.
- **Policy-link guard** — [`api-migration.md`](api-migration.md) exists, names the supported window, and is linked from this document and from `docs/api-overview.md`.

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
- Contract proof: `rivulet-core/tests/recording_job.rs` (29 tests, wired as
  an explicit `[[test]]` target in `rivulet-core/Cargo.toml`) pins typed
  validation per parameter, the pipeline fragments, the event stream
  (Started/Progress/Paused/Resumed/Stopped), pause/resume push gating, the
  stop counters and the Drop-finalize (moov atom present) — plus the
  end-to-end acceptance test
  `audio_and_video_reach_the_file_and_a_local_rtmp_server_in_one_session`,
  which runs one session's audio+video into file *and* a loopback RTMP
  fixture (a minimal in-test server: handshake, connect/createStream/publish
  replies, counted audio/video messages — no hardware, no network) — plus
  `rivulet-core/examples/session_embed.rs`, which CI runs with the other
  examples.

Status: verified — the suite passes locally (29/29, the E2E leg included)
and the example runs with exit 0; enforced by the CI test and example-run
jobs.

## Deprecation and migration policy (#344)

The written policy lives in [`api-migration.md`](api-migration.md); this section
is the contract's summary of it and the reason the summary cannot go stale.

- **Window.** A public name is deprecated for **at least one minor release**
  before it is removed, and never inside the same `0.x` minor line. Pre-1.0
  Cargo treats a minor bump as breaking; the policy says so plainly instead of
  hiding behind `0.x`, and still guarantees a migration window.
- **Deprecation.** `#[deprecated(since = "…", note = "…")]` — the note names the
  replacement, the item keeps working, a `## [Unreleased]` changelog entry names
  both, and the item appears in the *Scheduled removals* table.
- **Removal.** Requires a row in *Applied migrations* with a replacement and a
  migration note **before** the removal merges, plus a changelog entry linking
  the deprecation, plus a refreshed `api_surface_baseline.txt`.
- **What counts as a removal.** Signature changes, dropped enum variants and
  struct fields, widening a public struct with a constructible field, and a
  changed answer with an unchanged type. #345 chose a new function over a
  changed signature for exactly this reason.
- **Not covered.** Modules whose own docs call them experimental, test helpers,
  and documented platform-dependent behaviour.

Guards: the `api_stability` suite (baseline removal, deprecation notes, policy
links), [`api-overview.md`](api-overview.md) generation (`--check` in CI), and
`python scripts/generate-api-overview.py --write-baseline`, which both refreshes
the baseline and refuses to drop a name the policy does not document.

Status: verified - `cargo test -p rivulet-core --test api_stability` passes
(5 tests) and `python scripts/generate-api-overview.py --self-test` covers the
baseline refusal.

## Deprecation: `detect_encoder_availability` (#345)

The first written deprecation of the stable surface, and the shape future ones
follow:

- **What is deprecated.** `rivulet_core::detect_encoder_availability(encoder)`.
  It answered `RecordingConfigDetect::Available` for every input, because
  availability is a property of an *(encoder, codec)* pair and the signature had
  no codec.
- **What replaces it.** `detect_encoder_availability_for_codec(encoder, codec)`
  for the typed `RecordingConfigDetect`, and
  `detect_available_encoders_with_info(codec)` for the full candidate list with
  a machine-readable `why_not` per backend.
- **How a consumer migrates.** The `#[deprecated(note = ...)]` on the item names
  the replacement, and `docs/feature-detection.md` carries the before/after
  example. Nothing else about the type changes: both functions return
  `RecordingConfigDetect`.
- **What still works.** The item stays callable and re-exported from the crate
  root for the whole `0.65` line (H.264 only, delegating to the codec-aware
  helper), so an existing caller keeps compiling and now gets a true answer for
  the codec it implicitly assumed.
- **What is pinned.** `rivulet-core/tests/recording_config.rs` asserts the typed
  helper agrees with the probe for every (encoder, codec) pair, and a
  source-level guard fails if the deprecated helper ever returns a constant
  again or loses the note naming its replacement.

Status: verified - `cargo test -p rivulet-core --test recording_config` passes
(34 tests) and `cargo clippy --workspace --all-targets -- -D warnings` reports
no deprecated-use warning inside the crate.
