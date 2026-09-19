# M7 — Automation & Determinism ("Render-First")

**Status:** Planned — 7 workstream issues on the
[M7 milestone](https://github.com/thoser666/Rivulet/milestone/7), none started.
Issue #186 (headless CLI) is the recommended first implementation step.

*Differentiation pillar #1: OBS is interactive-first, Rivulet is deterministic.
This milestone is the reason a developer or team **cannot** use OBS but **can**
use Rivulet.*

**Tracked in:** Milestone M7 — Automation & Determinism

## Problem

OBS is built for a human at a desk: interactive, GUI-first, its `libobs` core
is not a clean library. Everything reproducible — generating video from code,
rendering overlays in CI, batch-producing content, verifying a pipeline bit by
bit — is painful or impossible. Rivulet's engine already builds GStreamer
pipelines as inspectable strings, its event/alert/chat contracts are pure and
deterministically testable, and its test suite runs the real pipeline parser
without capture hardware. What is missing is making that determinism a
*product*: a headless entry point, a controllable clock, golden-frame
verification, and inspector tooling.

## Goal

1. **Headless CLI** — capture and rendering without a GUI
   (`rivulet record ...`), usable as a binary and as a library.
2. **Deterministic pipeline** — a controllable engine clock; the same inputs
   produce reproducible output, and every remaining source of nondeterminism
   is reported, not hidden.
3. **CI-friendly rendering** — generate video from code natively in Rust
   (Remotion approach): batch creation, tests, per-frame screenshots.

Supporting workstreams: reproducible distribution inputs, pipeline
inspector/diagnostics, scene-item copy/paste API, deterministic tests as
first-class citizens.

## Non-goals

- Language bindings (JS/Python) — M8 territory.
- `rivulet-core` API stabilization/semver work — M8.
- WebGPU/zero-copy rendering — M9.
- Streaming over the CLI: the first CLI milestone ships `record` (plus inspect
  and render); a streaming subcommand is a follow-up once the CLI surface is
  proven.

## Platform scope

| Platform | Headless CLI | Notes |
| --- | --- | --- |
| Linux (CI) | **Primary target** — every workstream is validated on a clean CI runner without capture hardware | test/loopback sources only |
| Windows | supported via the same engine paths | capture-backed sources need real hardware |
| macOS | supported via the same engine paths | capture-backed sources need real hardware |

## Workstreams

### W1 — Headless CLI (`rivulet record`) — issue [#186](https://github.com/thoser666/Rivulet/issues/186)

The smallest complete slice of "automation": a `rivulet-cli` crate with a
`record` subcommand that wraps the engine — as a binary *and* as a library.

**DoD**

- New `rivulet-cli` crate; `record` subcommand wrapping the engine (binary and
  library paths share one implementation).
- Config file (TOML) + CLI flags: source selection (test sources/loopback),
  audio routing, encoder, output path, duration limit.
- Stable JSON status events on stdout (documented schema), human-readable
  diagnostics on stderr — strictly separated streams.
- Documented exit codes; graceful SIGINT/SIGTERM stop that finalizes the
  container.
- Library path: the same recording achievable in-process without spawning the
  binary.

**Acceptance criteria**

- [ ] Headless recording of N seconds from a test source produces a valid
  container on CI Linux with no capture hardware.
- [ ] `--json` emits documented, stable status events; diagnostics never mix
  into the JSON stream.
- [ ] Invalid config exits non-zero with an actionable error naming the
  offending key.
- [ ] SIGINT/SIGTERM stops cleanly (finalized file, exit code 0).
- [ ] CLI help, examples, and exit codes are documented in this spec (§ CLI
  surface reference).

### W2a — Deterministic pipeline (clock + reproducible-run contract) — issue [#187](https://github.com/thoser666/Rivulet/issues/187)

**DoD**

- Engine accepts an injectable clock source (system clock default; virtual /
  manual clock for tests and rendering).
- Virtual clock drives PTS/GstClock so a run is time-scriptable (advance,
  hold, step).
- Reproducible-run contract: identical inputs + virtual clock → identical
  container timestamps.
- Nondeterminism inventory: wall-clock metadata, encoder rate-control state,
  hardware encoders — documented in this spec (§ Nondeterminism inventory).
- Machine-readable run report lists which nondeterminism sources were active.

**Acceptance criteria**

- [ ] Two runs with identical inputs under the virtual clock produce identical
  PTS/DTS sequences (integration test).
- [ ] A run using wall-clock features reports them in the run summary.
- [ ] The spec documents the reproducibility contract and its explicit limits.
- [ ] Clean-machine transcript (gate exit evidence) reproducible from the docs.

### W2b — Deterministic tests as first-class citizens — issue [#188](https://github.com/thoser666/Rivulet/issues/188)

**DoD**

- Golden-frame helper: render frame N → compare against a reference; failure
  output shows frame index and a useful diff, not a raw buffer dump.
- Exact PTS/DTS verification helper for pipeline contract tests.
- At least two golden-frame tests and one PTS/DTS contract test in the repo
  using the helpers.
- Helpers documented in this spec with usage examples.

**Acceptance criteria**

- [ ] Golden-frame test failure output names the frame index and shows a
  pixel-level diff summary.
- [ ] PTS/DTS helper detects a tampered timestamp in a test.
- [ ] All M7 pipeline contract tests can express themselves in terms of these
  helpers.

### W3 — CI-friendly rendering (video from code) — issue [#189](https://github.com/thoser666/Rivulet/issues/189)

**DoD**

- Scriptable rendering of a configured scene/composition to video (batch
  mode).
- Per-frame screenshot mode: render frame N to PNG deterministically.
- Batch driver: a directory of configs renders to a directory of outputs, one
  job per config, with a summary report.
- CI integration example: a workflow job or documented local command that
  renders a sample video and validates the output.

**Acceptance criteria**

- [ ] Rendering frame N twice under the virtual clock yields identical PNG
  output (test).
- [ ] Batch run over ≥2 configs produces a machine-readable summary (per-job
  status, output paths).
- [ ] A CI job demonstrates end-to-end video generation from code on a clean
  runner.

### W4 — Reproducible distribution inputs — issue [#190](https://github.com/thoser666/Rivulet/issues/190)

**DoD**

- Audit each distribution channel (EXE zip, MSI, Scoop, Chocolatey, WinGet,
  AUR, Flatpak) for build reproducibility gaps; document them.
- Generate SHA-256 manifests per release artifact set as part of the release
  job.
- Post-publish verification job: re-download published artifacts, verify
  hashes and (where signing exists) signatures.
- Document which channels can be bit-reproducible and which are
  content-deterministic only (installer GUIDs, timestamps).

**Acceptance criteria**

- [ ] A release produces a `SHA256SUMS` manifest covering every published
  artifact.
- [ ] Post-publish verification passes on the latest release (or documents
  per-channel deviations honestly).
- [ ] The spec lists per-channel determinism status with reasons.

### W5 — Pipeline inspector/diagnostics — issue [#191](https://github.com/thoser666/Rivulet/issues/191)

The introspection half of the CLI story, analogous to `gst-inspect` /
`gst-launch` but engine-aware.

**DoD**

- `rivulet inspect`: print the pipeline string the engine would build for a
  given config (both mux legs, audio branches) without running it.
- Element/capability listing: available encoders, capture backends, audio
  filters with feature-detection results as JSON.
- Session diagnostics: on failure, the run report names pipeline, input,
  configuration and failing stage; redaction rules apply (masked keys).
- Dry-run mode for the record command reusing the same code path.

**Acceptance criteria**

- [ ] `inspect` output for a config matches the pipeline string the engine
  actually builds (shared code path, test-pinned).
- [ ] Feature detection lists encoders/backends as stable JSON; no secrets in
  any diagnostic surface (existing redaction tests extended).
- [ ] A failing run names the failing stage in the machine-readable report.

### W6 — Scene-item copy/paste API — issue [#192](https://github.com/thoser666/Rivulet/issues/192)

**DoD**

- Core: copy/paste/duplicate of scene items within and across scenes
  (transforms, filters, source references preserved; ids regenerated
  deterministically).
- Undo/redo integration (M2 undo stack).
- Scriptable surface: the operation is callable without the GUI (engine API or
  headless command), so tests and scripts can drive it.
- GUI affordance follows the existing scene-item interaction patterns.

**Acceptance criteria**

- [ ] Paste of the same clipboard content twice produces identical scene state
  (deterministic id generation, test-pinned).
- [ ] Cross-scene paste does not move the item out of the source scene
  (duplicate semantics, not move).
- [ ] Undo restores the pre-paste scene state exactly.
- [ ] The operation is covered by the deterministic-test helpers from W2b.

## CLI surface reference (grows with implementation)

```
rivulet record --config recording.toml --output out.mp4 [--json] [--duration SECS]
rivulet inspect --config recording.toml [--json]
rivulet render --config scene.toml --frame N --png out.png    (planned, W3)
rivulet render --config-dir scenes/ --out-dir renders/        (planned, W3 batch)
```

Exit codes (planned, finalized in W1):

| Code | Meaning |
| --- | --- |
| 0 | success (including graceful stop) |
| 1 | generic runtime failure |
| 2 | invalid configuration (error names the offending key) |
| 3 | signal stop (SIGINT/SIGTERM) after finalizing the container |

JSON status event schema (planned, finalized in W1): one JSON object per line
on stdout with a stable `type` discriminator (`started`, `progress`,
`stopped`, `error`); diagnostics never mix into the JSON stream.

## Nondeterminism inventory

The reproducible-run contract (W2a) holds only within these limits, which the
run report must state explicitly:

| Source | Behavior | Reporting |
| --- | --- | --- |
| Engine clock (PTS, session duration) | deterministic under the virtual clock | run report |
| Encoder rate-control state (x264 lookahead/VBV) | deterministic for identical input frames; first-frame effects depend on settings | run report notes encoder config |
| Hardware encoders (NVENC/QSV/VAAPI) | not bit-deterministic across runs/driver versions | run report flags hw encoder use |
| Wall-clock-derived metadata (creation_time, capture timestamps) | nondeterministic by nature | run report lists fields |
| Capture-backed sources (screen/camera/mic) | live input cannot repeat | out of reproducibility contract; test/loopback sources only |
| GStreamer element threading | fixed scheduling under the virtual clock; element-internal threads (queue leaks) may reorder | validated by the reproducibility test |

## Quality gate

Gate review happens at milestone exit per
[docs/milestone-quality-gates.md](milestone-quality-gates.md) § M7 — the
developer-experience gate: actionable CLI help/errors/exit codes, deterministic
output or reported nondeterminism, CI-usable progress/cancellation, stable
machine-readable JSON separated from human diagnostics, logs that identify
pipeline/input/config/failing stage without leaking secrets, and useful diffs
on golden-frame/timestamp/reproducibility failures. Exit evidence: clean-machine
command transcript, reproducibility comparison, machine-readable schema
validation.

## References

- Roadmap: README § M7; long-term goal "Feature Parity" (determinism pillar)
- Quality gate: [docs/milestone-quality-gates.md](milestone-quality-gates.md) § M7
- Existing determinism precedents: `alerts_eventsub.rs` / `alerts_ingest.rs`
  (pure, deterministic contracts); engine pipeline-string contract tests
- Release platforms context: [docs/release-platforms.md](release-platforms.md)
