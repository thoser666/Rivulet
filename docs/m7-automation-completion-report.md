# M7 Automation & Determinism Completion Report

- Commit/PRs: see the per-workstream table below; the workstreams landed in
  `f661e70` (#194), `861cd78` (#238), `b3cf4df` (#265), `1698a75` (#266),
  `577dbfb` (#268), `36834cd` (#285), `a64e70d` (#286), `a27cceb` (#261),
  `f3f6e8a` (#270), `1ea8671` (#226)
- Review date (UTC): 2026-10-05
- Reviewers: Rivulet maintainers (automated evidence review; the
  rebuild-and-compare and live-ingest gaps are noted below)
- Returned scope: headless CLI (`record`/`render`/`inspect`), deterministic
  pipeline with an injectable clock, CI-friendly rendering, reproducible
  distribution inputs, deterministic test helpers, scene-item copy/paste
- Overall result: `CONDITIONAL PASS`

## Summary

All seven M7 workstreams are implemented, tested, and merged; their issues are
closed (#186–#192). The M7 quality-gate section in
[`docs/milestone-quality-gates.md`](milestone-quality-gates.md) records a
per-workstream status note for each, the spec
([`docs/m7-automation.md`](m7-automation.md)) has every acceptance criterion
checked, and the automated cross-platform baseline passes on a clean runner —
including the `Render Smoke` job, which runs the *shipped* `rivulet` binary
end-to-end and asserts byte-identity, the PNG signature, the batch summary, and
a real MP4 container.

Two items do not have automated evidence and are therefore assigned to explicit
follow-ups rather than silently accepted: release artifacts are **auditable and
download-verified** but not yet **proven bit-reproducible by rebuild**, and
streaming over the CLI was a declared non-goal that is now unblocked.

- Roadmap checkboxes: **7 / 7 checked** in the `README.md` M7 section; the
  milestone is complete at the roadmap level.
- Blocker/Critical findings: **0**
- Open High findings: **0**; the two remaining items are Medium
  evidence/follow-up items, explicitly assigned below.

## Functional scope delivered (roadmap)

- [x] Headless CLI (issue #186) — a `rivulet-cli` crate with `rivulet record`
  as a binary *and* a library (`rivulet_cli::record` / `RecordJob` with
  swappable sinks): TOML config plus flag overrides, strict stream separation
  (JSON status events on stdout, diagnostics on stderr), documented exit codes
  (0 success incl. graceful SIGINT/SIGTERM finalize, 1 runtime, 2 config with a
  named key), and test-side verification of a valid MP4
  (`ftyp`/`moov`/`mdat`/`avc1`).
- [x] Deterministic pipeline (issue #187) — injectable `SystemClock` /
  `VirtualClock` (`advance`/`step`/`hold`), identical container timestamps under
  the virtual clock, and every remaining source of nondeterminism *reported*
  rather than hidden: the `stopped` event carries the active sources, the
  encoder, and separate `reproducible` / `byte_reproducible` flags.
- [x] Deterministic tests as first-class citizens (issue #188) —
  `rivulet_core::test_helpers` provides `GoldenFrame` (frame index, pixel
  count/max/mean delta, first differing coordinates, `Serialize` for
  machine-readable CI output, PNG artifact), `Timestamps`
  (`assert_equals`/`assert_constant_interval`/`assert_monotonic`/
  `assert_dts_not_after_pts`, each naming the offending index and both values),
  `SceneState`, and synthetic frames via `TestVideoSource`.
- [x] CI-friendly rendering (issue #189) — `rivulet render --config scene.toml
  --frame 7 --png out.png` renders frame N deterministically, `--video` pushes
  frames through the engine on the virtual clock, and `--config-dir scenes/
  --out-dir renders/ --json` drives a batch (one job per config, isolated
  failures, machine-readable summary with per-job status and relative output
  paths). The `Render Smoke` CI job runs the shipped binary on a clean runner.
- [x] Reproducible distribution inputs (issue #190) — one tested
  `scripts/release-manifest.py` replaces the duplicated inline
  `find | xargs sha256sum` pipeline in both release workflows, and a
  `verify_release` job re-downloads the published release with
  `gh release download` and verifies every artifact against `SHA256SUMS`, so
  what CI hashed is proven to be what a user fetches. Per-channel
  bit-reproducibility is documented honestly in
  [`docs/release-platforms.md`](release-platforms.md).
- [x] Pipeline inspector/diagnostics (issue #191) — `rivulet inspect --config
  recording.toml [--json]` prints the pipeline the engine *would* build (same
  code path as a real run, secrets redacted) plus a capability report of local
  encoders/containers/capture backends/audio filters; `rivulet record --dry-run`
  reuses that path; a failing run appends a machine-readable
  `{"event": "failed", "stage": …}` object naming the failing stage
  (`usage`/`config`/`output`/`engine`/`finalize`), and the exit code is derived
  from that stage so report and exit status cannot disagree.
- [x] Scene-item copy/paste API (issue #192) —
  `SourceManager::copy_scene_item` takes source plus binding verbatim into a
  `SceneItemClipboard` (transforms, crop, lock, visibility, order),
  `paste_scene_item` duplicates into any scene with a `" copy"` suffix and can
  do so deterministically (`with_deterministic_ids`, UUIDv5 from source ID +
  target scene, so scripted pastes are idempotent). Each paste lands on a
  paste-scoped undo/redo stack in the M2 pattern, which the GUI undo dispatch
  prefers. GUI: copy/paste buttons plus Ctrl+C/Ctrl+V in the Scenes view only
  (text-edit meaning elsewhere untouched), three new i18n keys (EN/DE).

## Per-workstream delivery

| Workstream | Issue | Key PRs | Evidence |
| --- | --- | --- | --- |
| W1 headless CLI | #186 | #194 (`f661e70`) | `cli_mvp_schema_and_docs_are_pinned` ci_pinning guard; CLI reference in the spec |
| W2a deterministic pipeline | #187 | #238 (`861cd78`), #265 (`b3cf4df`) | `m7_reproducible_run_report_surface_is_pinned` ci_pinning guard; reproducible-run contract in the spec |
| W2b deterministic test helpers | #188 | #266 (`1698a75`) | `m7_deterministic_test_helpers_are_pinned` ci_pinning guard; helper unit tests in `rivulet-core/src/test_helpers.rs` |
| W3 CI rendering | #189 | #268 (`577dbfb`) | `m7_batch_render_surface_is_pinned` ci_pinning guard; `Render Smoke` job in CI |
| W4 reproducible distribution | #190 | #285 (`36834cd`), #286 (`a64e70d`) | `release_manifest_generation_and_post_publish_verification_are_pinned` ci_pinning guard; `Verify published release` job in CI and release |
| W5 inspector | #191 | #261 (`a27cceb`), #270 (`f3f6e8a`) | `cli_inspect_surface_is_pinned` + `w5_machine_readable_failure_is_pinned` ci_pinning guards |
| W6 scene-item copy/paste | #192 | #226 (`1ea8671`) | `scene_item_copy_paste_surface_is_pinned` ci_pinning guard; core + GUI behaviour tests |

## Automated checks

| Check | Result | Evidence |
| --- | --- | --- |
| Format | PASS | `cargo fmt --all -- --check` |
| Tests | PASS | `rivulet-core` lib suite (1137), `ci_pinning` (116), CI build/test matrix (Windows/Linux/macOS) |
| Clippy/lint | PASS | CI Lints job with `-D warnings` |
| CI-specific checks | PASS | actionlint, action-pin table, Beta-Gate, Scorecard |
| End-to-end render | PASS | `Render Smoke` runs the shipped binary on a clean runner and asserts byte-identity, PNG signature, batch summary, real MP4 container |
| Post-publish verification | PASS | `Verify published release` re-downloads the live release and verifies it against `SHA256SUMS` |
| Roadmap/gate sync | PASS | `scripts/check-roadmap-sync.py` keeps the README milestone table and gate section titles in sync |
| Pre-push hook | PASS | fmt + workspace clippy `-D warnings` + `ci_pinning` + action-pin/parity/release-notes/contrast checks |

## Quality-gate criteria (M7)

| Criterion | Test / evidence |
| --- | --- |
| CLI help, examples, config errors, exit codes are actionable | `cli_mvp_schema_and_docs_are_pinned` (exit-code table, `usage`/`config` key names, CLI reference in the spec) |
| Clean invocation is deterministic **or reports every nondeterminism source** | `m7_reproducible_run_report_surface_is_pinned` — `reproducible` and `byte_reproducible` are separate flags, so a non-reproducible run says so instead of claiming parity |
| Progress/cancellation work headless | `--duration`/`--max-bytes`, graceful SIGINT/SIGTERM finalize, exit code 0 on finalize; `Render Smoke` drives the binary non-interactively |
| JSON/status output stable, documented, separated from diagnostics | strict stdout/stderr split pinned for `record`, `render`, and `inspect` |
| Logs identify pipeline/input/config/stage without leaking secrets | inspector redacts secrets on the same code path a real run takes; `w5_machine_readable_failure_is_pinned` pins the failing-stage object |
| Golden-frame, timestamp, reproducibility failures show useful diffs | `m7_deterministic_test_helpers_are_pinned` — first differing pixel coordinates, offending timestamp index, per-job batch status |
| Exit evidence: clean-machine command transcript | `Render Smoke` on a clean CI runner, plus the post-publish `verify_release` job |
| Exit evidence: reproducibility comparison | `rivulet render --frame` byte-identity inside `Render Smoke`; artifact-level comparison tracked as F-M7-001 |
| Exit evidence: machine-readable schema validation | `ci_pinning` schema guards (115→116 tests) plus `scripts/release-manifest.py --self-test` run on every push |

## Findings and explicit follow-ups

The following are **not** silently accepted as determinism; each is assigned to
a follow-up and remains visible in the roadmap/documentation.

| ID | Severity | Area | Description | Tracking issue | Retest condition |
| --- | --- | --- | --- | --- | --- |
| F-M7-001 | Medium | Release reproducibility | #190 proves that what CI hashed is what a user downloads (manifest generation + post-publish re-download verification). It does **not** prove that two independent builds of the same commit are byte-identical: there is no rebuild-and-compare job. `docs/release-platforms.md` documents which channels are content-deterministic (normalized timestamps, harvested MSI GUIDs) and which are not bit-identical, but that claim is static, not measured. | [#288](https://github.com/thoser666/Rivulet/issues/288) | A CI job rebuilds a channel on a second runner and fails on manifest divergence; `docs/release-platforms.md` carries measured results per channel |
| F-M7-002 | Medium | CLI surface | Streaming over the CLI was a declared M7 non-goal ("a streaming subcommand is a follow-up once the CLI surface is proven"). The CLI surface is now proven (`record`, `render`, `inspect` merged and pinned), so `rivulet stream` is unblocked but unshipped. | [#289](https://github.com/thoser666/Rivulet/issues/289) | `rivulet stream` ships as binary *and* library with the same status-event/exit-code/dry-run contract as `record` |

Signature verification is deliberately *not* listed as a finding: it is not an
ungoverned gap. The post-publish job checks detached signatures wherever they
exist and reports the unsigned-channel deviation explicitly, and the missing
signing identities are tracked in
[#50](https://github.com/thoser666/Rivulet/issues/50). There are no Blocker,
Critical, or open High findings.

## Decision

- [ ] M7 gate passed without conditions.
- [x] Conditional pass; the remaining findings are Medium/evidence items and
  explicitly assigned to follow-ups.
- [ ] Failed; release-blocking M7 work remains.

M7 is complete for its functional scope. The milestone must not be used to claim
bit-reproducible release artifacts (F-M7-001) or headless streaming via the CLI
(F-M7-002); both remain explicit follow-ups, tracked as
[#288](https://github.com/thoser666/Rivulet/issues/288) (F-M7-001) and
[#289](https://github.com/thoser666/Rivulet/issues/289) (F-M7-002).
The report is linked from the M7 roadmap section in `README.md`, the M7 spec
(`docs/m7-automation.md`), and the M7 quality gate in
`docs/milestone-quality-gates.md`; release notes should reference it.