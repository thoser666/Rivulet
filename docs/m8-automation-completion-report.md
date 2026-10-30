# M8 Embeddable Engine & API Completion Report

- Commit/PRs: see the per-workstream table below; workstreams landed in
  `39c9464` (#295 inspection: Feature-Erkennung), `e022edf` (#296 capture
  backends Trait), prior PRs #292/#293 (API stability + examples), and #297
  (exit evidence: external consumer smoke test, doc build gate, generated API
  overview, this report).
- Review date (UTC): 2026-10-27
- Reviewers: Rivulet maintainers (evidence review; the downstream-consumer
  smoke test now runs in CI against the unpacked `cargo package` artifact — see
  Exit evidence)
- Returned scope: stable `rivulet-core` API, public API documentation,
  examples, and exit evidence (`cargo doc`, example run matrix, downstream
  consumer smoke test, API overview from code).
- Overall result: **Delivered** — six workstreams closed, nine gate criteria
  recorded individually (seven satisfied, two partial), four follow-ups named
  and tracked in their own issues instead of being absorbed into the verdict.

## Summary

All six M8 workstreams are delivered and pinned through `ci_pinning`:

- Workstream 1: stabilized public API (`#292`) + examples (`#293`) + in-process
  capture session entry point (`#294`), plus the `#295` inspection API with
  cache/detector tests.
- Workstream 2: capture-backend trait (`#296`) — documented, `Send+Sync`, typed
  errors, stability matrix.
- Exit evidence (`#297`): complete. The external consumer crate builds and runs
  against the packaged artifact in CI, `cargo doc` is gated at zero warnings,
  the example run matrix executes all seven examples, and the public-API
  overview is generated from `lib.rs` and checked on every push.

The first conditional pass recorded two evidence gaps, and both are closed
here rather than carried forward:

- **F-M8-001** (was Medium) — the smoke test used to be a workspace member, so
  it could not prove that the *released* crate works. **Closed:** the test is
  now the external crate `downstream-smoke/`, built by
  `scripts/run-downstream-smoke.sh` against the unpacked `cargo package`
  artifact, and executed in the Build & Test job on Linux with
  `RIVULET_DOWNSTREAM_REQUIRE_AUDIO=1`.
- **F-M8-002** (was Low) — `cargo doc` warnings were only checked by the
  pre-push hook. **Closed:** the Lints job now runs
  `RUSTDOCFLAGS=-D warnings cargo doc -p rivulet-core --no-deps`, and the 22
  broken intra-doc links it found on the way are fixed (no `-A`/`--cap-lints`
  suppression was added).

Two follow-ups remain and are **not** silently accepted as completion. Each
was found while producing the exit evidence, and each names the criterion it
sits under, its tracking issue and its retest condition. Two further findings
(F-M8-005, F-M8-006) have since been closed by their own pull requests and are
still listed below with that verdict:

- **F-M8-003** (Medium, API documentation): `#![warn(missing_docs)]` is still not
  an active crate attribute; turning it on for real reports 696 undocumented
  public items (measured 2026-10-13). Tracked as
  [#342](https://github.com/thoser666/Rivulet/issues/342).
- **F-M8-004** (Medium, capture backends): `PipeWireBackend` and
  `MetalWgcBackend` answer `BackendUnavailable` by construction; only
  `XCapBackend` captures. Tracked as
  [#343](https://github.com/thoser666/Rivulet/issues/343).
- **F-M8-005** (Low, API policy): the written deprecation and migration policy
  now lives in [`api-migration.md`](api-migration.md) and is enforced by the
  `api_stability` removal, deprecation-note and policy-link guards. Closed by
  [#344](https://github.com/thoser666/Rivulet/issues/344).
- **F-M8-006** (Low, feature detection): the per-encoder helper answered
  `Available` for every input and disagreed with
  `detect_available_encoders_with_info()`. **Closed by**
  [#345](https://github.com/thoser666/Rivulet/issues/345): the helper is
  deprecated, delegates to `detect_encoder_availability_for_codec()`, and both
  surfaces are pinned against each other in `tests/recording_config.rs`.

- Roadmap checkboxes: **6 / 6 checked** in the `README.md` M8 section; the
  pinned `ci_pinning` guards hold status, gate, report and issue references
  together.
- Blocker/Critical findings: **0**; open High findings: **0**. Two of the four
  follow-ups are closed (F-M8-005 by
  [#344](https://github.com/thoser666/Rivulet/issues/344), F-M8-006 by
  [#345](https://github.com/thoser666/Rivulet/issues/345)); the remaining two,
  F-M8-003 and F-M8-004, are Medium and in the "documentation or implementation
  of an already-delivered contract" class — neither is a broken promise to a
  consumer.

## Exit evidence (#297)

The point of this workstream is that the evidence comes from outside the crate's
own prose.

| Evidence | Artifact | What it proves |
| --- | --- | --- |
| External consumer build + run | [`downstream-smoke/`](../downstream-smoke/) + [`scripts/run-downstream-smoke.sh`](../scripts/run-downstream-smoke.sh) | A crate that is not a workspace member, has no internal paths and depends on `rivulet-core` alone can drive a whole session (typed gate, video-only run, audio run, event stream, pause/resume, report vs. file on disk, structured detection) using only public API. |
| Docs build | `RUSTDOCFLAGS=-D warnings cargo doc -p rivulet-core --no-deps` in the Lints job | The publishable documentation builds without warnings; 22 broken intra-doc links were fixed to get there. |
| Example run matrix | `Run rivulet-core examples (M8 #293)` step, all three platforms | Every example *runs*, not just compiles. |
| Public API overview | [`docs/api-overview.md`](api-overview.md), generated by [`scripts/generate-api-overview.py`](../scripts/generate-api-overview.py) | The module list, the crate-root re-exports, the item counts and the experimental markers are read from `lib.rs` and the module sources; `--check` fails when the document drifts. |
| Completion report | this file | Every criterion is recorded individually, and nothing claims a "passed" verdict while follow-ups are open. |

### The word "published"

`rivulet-core` is **not** published to a registry: the project's release flow
publishes application binaries and packages, and no `cargo publish` step exists
in any workflow. "Builds against the published version" is therefore implemented
as *builds against the artifact `cargo publish` would upload* — `cargo package
--no-verify` output, unpacked, with cargo's normalized manifest and without the
workspace, its `dev-dependencies` or its test configuration. That is the
strongest available stand-in, and the report says so instead of implying a
registry release that does not exist.

### Audio on a bare host

The audio leg needs `avenc_aac` (from `gst-libav`). The smoke crate accepts the
typed `JobError::MissingElements` refusal that a host without it produces — the
same contract `rivulet-core`'s own suite pins instead of skipping — and says so
in its output. CI sets `RIVULET_DOWNSTREAM_REQUIRE_AUDIO=1`, which turns a
missing audio leg into a failure, so the positive audio evidence cannot be
replaced by the refusal branch unnoticed. Locally, on a checkout whose
`gst-libav` does not load, the run stays green and prints the refusal.

## Functional scope delivered (roadmap)

- [x] Stabilized public API for `rivulet-core` (semver 1.0, crate-style types;
  `#![warn(missing_docs)]` was named in #292 but stayed a doc-comment example in
  `lib.rs`, not an active lint — see F-M8-003) — [#292](https://github.com/thoser666/Rivulet/issues/292)
- [x] Comprehensive API docs + examples (recording, streaming, dual output,
  encoder selection, engine lifecycle, audio routing, frame delivery —
  `session_embed` pushes real frames through the in-process session API) —
  [#293](https://github.com/thoser666/Rivulet/issues/293), CI runs all seven
- [x] In-process capture API: embed a recording feature in any Rust app
  (audio/video capture, encoding, file/stream) —
  [#294](https://github.com/thoser666/Rivulet/issues/294) — typed config gate +
  GUI-free `RecordingJob` session entry point delivered, pinned by 29 tests in
  `tests/recording_job.rs`, demonstrated end-to-end with audio+video to file
  *and* a loopback RTMP fixture, shown via the `session_embed` example
- [x] Feature detection and runtime diagnostics as an API
  (`detect_available_encoders_with_info`, encoder fallback, capture-backend
  detector) — [#295](https://github.com/thoser666/Rivulet/issues/295) —
  structured `EncoderInfo`/`EncoderFallback`/`CaptureBackendInfo`/`FeatureInfo`,
  `FEATURE_DETECTION_CACHE`, non-localised `why_not`, docs at
  `docs/feature-detection.md`; the older per-encoder helper is deprecated and
  delegates to the codec-aware helper (F-M8-006, closed by
  [#345](https://github.com/thoser666/Rivulet/issues/345))
- [x] Abstraction of capture backends (xcap, PipeWire, Metal/WGC) behind stable
  traits — [#296](https://github.com/thoser666/Rivulet/issues/296) —
  `CaptureBackend` trait (`Send+Sync`, lifecycle, typed `CaptureError`),
  stability matrix (`docs/capture-backends.md`), enumeration guard test; the
  PipeWire and Metal/WGC implementations are still stubs (F-M8-004)
- [x] Exit evidence — [#297](https://github.com/thoser666/Rivulet/issues/297) —
  external consumer smoke test in CI against the packaged artifact, warning-free
  `cargo doc`, example run matrix, generated API overview, this report

## Per-workstream delivery

| Workstream | Issue | Key PRs | Evidence |
| --- | --- | --- | --- |
| W1 stabilized public API | #292 | prior PRs | `api_stability` ci_pinning guard; `library` exports complete |
| W2 API docs + examples | #293 | prior PRs | CI `Run rivulet-core examples`; 7 examples runnable |
| W3 in-process capture entry point | #294 | prior PRs | 29 tests in `recording_job.rs`; `session_embed` example end-to-end |
| W4 inspection API | #295 | `39c9464` | `FEATURE_DETECTION_CACHE`, encoder tests, `docs/feature-detection.md` |
| W5 capture-backend trait | #296 | `e022edf` | `all_registered_backends_implement_trait`, `docs/capture-backends.md` |
| W6 exit evidence | #297 | this PR | `downstream-smoke/` + `scripts/run-downstream-smoke.sh`, `docs/api-overview.md`, doc gate, this report |

## Automated checks

| Check | Result | Evidence |
| --- | --- | --- |
| Format | PASS | `cargo fmt --all -- --check` |
| Tests | PASS | `rivulet-core` lib suite, `ci_pinning`, `recording_job`, CI build/test matrix |
| Clippy/lint | PASS | CI Lints job with `-D warnings` |
| `cargo doc` warnings | PASS | `RUSTDOCFLAGS=-D warnings cargo doc -p rivulet-core --no-deps` in the Lints job (was 22 broken links, now 0) |
| Generated API overview | PASS | `scripts/generate-api-overview.py --check` in the Lints job; `--self-test` covers drift |
| External consumer | PASS | `Downstream consumer smoke test (M8 #297)` step, Build & Test job, Linux |
| CI-specific checks | PASS | actionlint, action-pin table, Beta-Gate, Scorecard, end-to-end render |
| Example run matrix | PASS | `Run rivulet-core examples` executes every example on each platform |
| Roadmap/gate sync | PASS | README M8 table aligns with the gate section |

## Quality-gate criteria (M8)

Every criterion is listed on its own line with its own verdict; none of them is
folded into a single verdict.

| # | Criterion | Status | Evidence / gap |
| --- | --- | --- | --- |
| 1 | Public names, defaults, error types, lifecycle and thread-safety guarantees coherent and documented | **partial** | `api_stability` guard, typed `RecordingConfigError`/`JobError`; documentation is not enforced because the `missing_docs` lint is inactive (F-M8-003) |
| 2 | Examples compile and cover recording, streaming, dual output, encoders, frame delivery | **satisfied** | all seven examples run on three platforms in CI; `session_embed` covers frame delivery |
| 3 | Feature detection and fallback available without parsing logs | **satisfied** | `detect_available_encoders_with_info`, `encoder_fallback_for_codec` with `why_chosen`/`why_not`; `detect_encoder_availability` is deprecated and delegates to the codec-aware helper (was F-M8-006) |
| 4 | Versioning, deprecation, migration and compatibility policy explicit | **satisfied** | [`api-migration.md`](api-migration.md) names the window, the attribute, the migration table and what counts as a removal; `api_stability` fails on an undocumented removal or a note-less `#[deprecated]` (was F-M8-005) |
| 5 | Invalid configuration fails early with typed, actionable errors | **satisfied** | `RecordingConfig::validate()` per-parameter, `tests/recording_config.rs`; `RecordingJob::start` validates before GStreamer |
| 6 | API docs and examples work without depending on GUI-only state | **satisfied** | examples link no `egui`/`eframe`; `RecordingJob` is GUI-free |
| 7 | Capture backend API coherent and documented | **partial** | `CaptureBackend` trait, stability matrix, 9 regression tests; PipeWire/Metal-WGC implementations are stubs (F-M8-004) |
| 8 | Downstream consumer smoke test against the published crate | **satisfied** | external crate built against the `cargo package` artifact, run in CI (was F-M8-001) |
| 9 | Docs build without warnings | **satisfied** | `-D warnings` doc gate in CI, zero warnings (was F-M8-002) |

There is no Blocker, Critical or open High finding. The milestone is delivered:
seven criteria are satisfied outright, and the two partial ones are each an
unfinished *edge* of a delivered contract — a lint that is not switched on and
two stub implementations behind a stable trait. Each of those two has its own
tracking issue and a retest condition, and the README, the gate and this report
name them together rather than one document claiming a completion the others
hedge.

## Findings and explicit follow-ups

The two *evidence* findings from the first conditional pass are closed with the
artifacts that close them, and the four *scope* findings stay open with their
own issues. Nothing is dropped between the two tables.

| ID | Severity | Status | Area | Description | Tracking issue | Retest condition |
| --- | --- | --- | --- | --- | --- | --- |
| F-M8-001 | Medium | **closed by #297** | Exit evidence | The downstream smoke test used to be a workspace member, so it could not prove the released crate works. Replaced by the external crate `downstream-smoke/`, built by `scripts/run-downstream-smoke.sh` against the unpacked `cargo package` artifact and run in the Build & Test job. | [#297](https://github.com/thoser666/Rivulet/issues/297) | The `Downstream consumer smoke test (M8 #297)` step passes with `RIVULET_DOWNSTREAM_REQUIRE_AUDIO=1` |
| F-M8-002 | Low | **closed by #297** | Exit evidence | `cargo doc` warnings were only checked by the pre-push hook. The Lints job now runs the doc build with `RUSTDOCFLAGS=-D warnings`, and the 22 broken links it found are fixed. | [#297](https://github.com/thoser666/Rivulet/issues/297) | `Build API docs without warnings (M8 #297)` fails on the first new rustdoc warning |
| F-M8-003 | Medium | open | API documentation | `#![warn(missing_docs)]` is not active; 696 undocumented public items (2026-10-13). "Stabilized" and "documented" are not the same claim yet. | [#342](https://github.com/thoser666/Rivulet/issues/342) | `docs/api-overview.md` reports `#![warn(missing_docs)]: active` and the build is warning-free with the lint on |
| F-M8-004 | Medium | open | Capture backends | `PipeWireBackend` and `MetalWgcBackend` answer `BackendUnavailable` by construction; only `XCapBackend` captures real frames. | [#343](https://github.com/thoser666/Rivulet/issues/343) | Each backend captures frames on its platform and no longer answers `BackendUnavailable` there |
| F-M8-005 | Low | **closed by #344** | API policy | No written deprecation or migration policy existed next to `docs/m8-api-contract.md`. [`api-migration.md`](api-migration.md) now names the window (at least one minor release), the `#[deprecated]` mechanics, the migration table, and what counts as a removal. | [#344](https://github.com/thoser666/Rivulet/issues/344) | `cargo test -p rivulet-core --test api_stability` fails on a removal that has no migration row and on a note-less `#[deprecated]`; the baseline-update command refuses an undocumented drop |
| F-M8-006 | Low | **closed by #345** | Feature detection | `detect_encoder_availability(encoder)` ignored its argument and answered `Available`, disagreeing with `detect_available_encoders_with_info()`. The helper is deprecated, delegates to `detect_encoder_availability_for_codec()`, and the agreement is pinned in `tests/recording_config.rs`. | [#345](https://github.com/thoser666/Rivulet/issues/345) | Both APIs agree for every encoder/codec pair, pinned by a test |

## Decision

- [ ] M8 gate passed without conditions.
- [x] M8 delivered: six workstreams, nine criteria recorded individually
  (seven satisfied, two partial), four follow-ups named in the table above and
  tracked in their own issues. The two evidence gaps of the first conditional
  pass (F-M8-001, F-M8-002) are closed by this workstream, not deferred.
- [ ] Failed; release-blocking M8 work remains.

M8 is complete for its six delivered workstreams. `#297` closes the milestone
and is the last issue in it. The report is linked from the M8 roadmap section in
`README.md`, the M8 gate in `docs/milestone-quality-gates.md`, and
`docs/api-overview.md`, and it is pinned by
`ci_pinning::m8_closeout_statements_match_across_readme_gate_and_report` and
`ci_pinning::m8_exit_evidence_is_wired_into_ci_and_docs`.
