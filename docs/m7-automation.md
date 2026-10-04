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

- [x] New `rivulet-cli` crate; `record` subcommand wrapping the engine (binary and
  library paths share one implementation: `rivulet_cli::RecordJob`).
- [x] Config file (TOML) + CLI flags: source selection (test sources/loopback),
  audio routing, encoder, output path, duration limit — the MVP ships the
  headless-capable subset: test source (pattern, size, fps), optional silent
  audio track, container, output path, duration; encoder selection follows the
  engine's `best_encoder()` default and becomes a flag with W3's render work.
- [x] Stable JSON status events on stdout (documented schema), human-readable
  diagnostics on stderr — strictly separated streams.
- [x] Documented exit codes; graceful SIGINT/SIGTERM stop that finalizes the
  container.
- [x] Library path: the same recording achievable in-process without spawning the
  binary (`rivulet_cli::record(config)` and `RecordJob` with custom event
  sinks).

**Acceptance criteria**

- [x] Headless recording of N seconds from a test source produces a valid
  container on CI Linux with no capture hardware
  (`headless_recording_produces_a_valid_container` in `rivulet-cli`).
- [x] `--json` emits documented, stable status events; diagnostics never mix
  into the JSON stream (schema pinned by the ci_pinning guard; note: JSON
  events are the default and only stdout format — there is no separate flag
  to disable them, so `--json` is accepted as a no-op alias in future if a
  non-JSON mode is ever added).
- [x] Invalid config exits non-zero with an actionable error naming the
  offending key (exit code 2; covered by config tests and the binary).
- [x] SIGINT/SIGTERM stops cleanly (finalized file, exit code 0) — the
  handler flips an atomic flag and `RecordJob::run` finalizes between
  frames.
- [x] CLI help, examples, and exit codes are documented in this spec (§ CLI
  surface reference; pinned by the ci_pinning guard).

### W2a — Deterministic pipeline (clock + reproducible-run contract) — issue [#187](https://github.com/thoser666/Rivulet/issues/187)

**DoD**

- [x] Engine accepts an injectable clock source (system clock default; virtual /
  manual clock for tests and rendering).
- [x] Virtual clock drives PTS/GstClock so a run is time-scriptable (advance,
  hold, step).
- [x] Reproducible-run contract: identical inputs + virtual clock → identical
  container timestamps.
- [x] Nondeterminism inventory: wall-clock metadata, encoder rate-control state,
  hardware encoders — documented in this spec (§ Nondeterminism inventory).
- [x] Machine-readable run report lists which nondeterminism sources were active.

**Acceptance criteria**

- [x] Two runs with identical inputs under the virtual clock produce identical
  PTS/DTS sequences (integration test).
- [x] A run using wall-clock features reports them in the run summary.
- [x] The spec documents the reproducibility contract and its explicit limits.
- [x] Clean-machine transcript (gate exit evidence) reproducible from the docs.

**Implementation**

`rivulet_core::clock` provides the injectable time source: `SystemClock`
(wall-clock pacing, today's `do-timestamp` behavior) and `VirtualClock`
(`advance_ns` / `step_frames` / `hold`), injected with
`RivuletEngine::set_clock`. Under the virtual clock the video buffer is stamped
explicitly at `session base + clock.now_ns()` and the appsrc's `do-timestamp` is
switched off, so the PTS cadence is exactly the scripted one and independent of
real-time speed. Audio PTS stay derived from sample counts at the fixed engine
rate, so the audio timeline is clock-independent by construction.

The run report is `rivulet_core::inspect::NondeterminismReport`, built from the
engine's *actual* state after a run (`clock_mode()`, `video_encoder()`, the
source kind) rather than from the requested config, so it describes what
happened. `rivulet record` emits it in the `stopped` status event:

```json
{
  "event": "stopped",
  "frames": 300,
  "seconds": 10,
  "file_size_bytes": 4194304,
  "clock": "system",
  "pts_source": "do-timestamp",
  "encoder": "NVIDIA NVENC",
  "reproducible": false,
  "byte_reproducible": false,
  "nondeterminism": {
    "sources": [
      {
        "source": "engine_clock",
        "detail": "PTS/session duration from the system clock",
        "active": true,
        "affects_timestamps": true,
        "timestamp_risk": false,
        "affects_bytes": false
      }
    ]
  }
}
```

Every source is reported on every run, with `active` telling a consumer whether
this run used it — so a documented limit is distinguishable from a limit that
actually applied. Two separate flags keep the claims honest:

- `reproducible` — the container **timestamp sequence** repeats. False whenever
  wall-clock PTS drive the run. `timestamp_risk` marks a source as only a *risk*
  to the timeline (element-internal threads), which the reproducibility test
  establishes empirically rather than by static analysis.
- `byte_reproducible` — the encoded **bytes** repeat. A stronger claim, and
  generally false: it additionally depends on encoder rate control, on whether
  a hardware encoder was used, and on wall-clock container metadata (creation
  time), which is written from the system clock even under the virtual clock.

Live capture sources (`Webcam`, `ScreenCapture`, `GameCapture`, `Audio` — see
`SourceKind::is_live`) are reported as active and cannot repeat regardless of the
clock; file-backed and synthetic sources can.

### W2b — Deterministic tests as first-class citizens — issue [#188](https://github.com/thoser666/Rivulet/issues/188)

**DoD**

- [x] Golden-frame helper: render frame N → compare against a reference; failure
  output shows frame index and a useful diff, not a raw buffer dump.
- [x] Exact PTS/DTS verification helper for pipeline contract tests.
- [x] At least two golden-frame tests and one PTS/DTS contract test in the repo
  using the helpers.
- [x] Helpers documented in this spec with usage examples.

**Acceptance criteria**

- [x] Golden-frame test failure output names the frame index and shows a
  pixel-level diff summary.
- [x] PTS/DTS helper detects a tampered timestamp in a test.
- [x] All M7 pipeline contract tests can express themselves in terms of these
  helpers.

**Implementation**

`rivulet_core::test_helpers` provides both helpers, dependency-free and
operating on raw RGBA (the engine's appsrc format):

- `GoldenFrame` — a frame plus the geometry needed to interpret it.
  `assert_matches(&reference, frame_index)` is the test entry point; on
  mismatch it panics with the frame index, how many pixels differ, the maximum
  and mean channel delta, and the first differing coordinates. `diff` returns
  the same information as data (`FrameDiff`, `Serialize`) for a CI step that
  wants JSON instead of a panic message, and `to_png_bytes` emits a viewable
  artifact. A length mismatch is rejected at construction rather than compared,
  because comparing unrelated regions produces a confidently wrong diff.
- `Timestamps` — a PTS/DTS sequence in nanoseconds with three contracts:
  `assert_equals` (exact sequence), `assert_constant_interval` (fixed cadence),
  `assert_monotonic` (strictly increasing), plus `assert_dts_not_after_pts`.
  Each `check_*` variant returns a `TimestampViolation` carrying the offending
  *index*, the expected value and the actual value, so a failure reads
  "index 3: expected 100000000 ns, got 101000000 ns" instead of two dumped
  sequences. `from_buffers` reads a `(pts, dts)` list straight off an appsink,
  with DTS falling back to PTS the way the engine's own probes do.
- `SceneState` — the scene-collection counterpart, so "this operation is
  deterministic" is assertable as a *named* difference (which item, which
  property) rather than one opaque whole-collection `assert_eq!`. Snapshot with
  `SceneState::new(mgr.current_collection())`, compare with `assert_matches`,
  and use `check_matches` when the difference should be data (for a test that
  asserts the shape of a failure). Ordering is normalized, so two runs that
  agree on content but not on insertion order do not report a spurious
  difference — precisely the distinction "is this operation deterministic?"
  needs. W6's copy/paste determinism and undo-restore tests express their
  assertions through it.

Usage — scene-state determinism:

```rust
use rivulet_core::test_helpers::SceneState;

let before = SceneState::new(mgr.current_collection());
mgr.duplicate_scene_item(sid, scene);
let after = SceneState::new(mgr.current_collection());

assert!(mgr.undo_paste());
SceneState::new(mgr.current_collection()).assert_matches(&before);
```

A failure names the drift instead of dumping two collections:

```
scene state differs: source 9f2c... property z_order differs: expected 0, got 9
  (actual 3 sources / 2 bindings, expected 3 sources / 2 bindings)
```

Synthetic frames come from `rivulet_core::source::TestVideoSource`, the same
generator the headless recording path pushes; it moved from `rivulet-cli` to
core for this workstream and is re-exported there, so the CLI's public surface
is unchanged. `TestVideoSource::frame_at(n, w, h, fps)` renders frame *n* from a
scratch source, so a reference frame does not depend on how many frames a test
has already consumed.

Usage — golden frame:

```rust
use rivulet_core::source::TestVideoSource;

let frame = TestVideoSource::frame_at(5, 64, 48, 30);
let reference = TestVideoSource::frame_at(5, 64, 48, 30);
frame.assert_matches(&reference, 5);
```

A failure reads:

```
golden frame mismatch at frame 5: 4/3072 (0.13%) pixels differ, max channel
delta 64, mean 16.00, first difference at (10, 10): expected [40, 40, 10, 255],
got [104, 40, 10, 255]
```

Usage — PTS/DTS contract:

```rust
use rivulet_core::test_helpers::Timestamps;

let interval = rivulet_core::clock::frame_interval_ns((30, 1));
let expected: Vec<u64> = (0..5).map(|frame| frame * interval).collect();

Timestamps::from_pts(expected.clone()).assert_equals(&expected);
Timestamps::from_pts(expected.clone()).assert_constant_interval(interval);

// A tampered or dropped timestamp is localized rather than merely detected:
assert!(Timestamps::from_pts(vec![0, interval, interval + 1_000, 3 * interval, 4 * interval])
    .check_equals(&expected)
    .is_err());
```

Usage — waiting on asynchronous state (`wait_until`):

```rust
use rivulet_core::test_helpers::wait_until;

let msg = wait_until(Duration::from_secs(5), || {
    chat.messages().and_then(|rx| rx.try_recv().ok())
})
.expect("message delivered");
```

`wait_until` is the suite-wide idiom established by the timing audit (see the
CHANGELOG entry "Suite-weites Timing-Audit"): a test that waits for an
asynchronous worker, pipeline or channel delivery polls the condition up to a
caller-supplied deadline (25 ms interval) instead of sleeping a fixed amount.
A positive assert after a fixed sleep is flake-prone — under parallel test
load a slow runner can sample the state before the worker produced it —
while polling turns the same assert into "wait until it is true, fail after
the deadline": slow runners wait longer, fast ones return immediately, and a
genuinely broken condition still fails instead of hanging. Prefer it over any
new `thread::sleep`-then-assert pattern; per-module hand-rolled loops are
being migrated to this helper. Two exceptions are deliberate: negative
asserts ("must not happen") stay as they are — a fixed delay only makes the
condition more true — and loops with a fail-fast escape (for example the chat
worker tests panic as soon as the worker reports `Disconnected`, because
retrying can never succeed) keep that logic instead of waiting out the
deadline.

Note the helper's contract when writing assertions about the deadline itself:
the condition is sampled at least once before the deadline is honored — under
load a single 25 ms sleep can overrun its interval by a large factor, so a
condition that turns true during such an overrun may still be observed. Tests
must not pin "must-not-see" expectations to sub-second
deadline/sleep timing; pin such behavior with call counting instead (the
helper's unit tests show how).

The helpers are exercised end-to-end in `rivulet-core/tests/m7_golden_frames.rs`,
which includes a test that captures timestamps from a *real* GStreamer
pipeline through an appsink, so the helper is proven on pipeline data rather
than only on hand-written vectors.

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

- [x] Rendering frame N twice under the virtual clock yields identical PNG
  output (test).
- [x] Batch run over ≥2 configs produces a machine-readable summary (per-job
  status, output paths).
- [x] A CI job demonstrates end-to-end video generation from code on a clean
  runner.

#### Surface

`rivulet-core/src/render.rs` owns the composition side, `rivulet-cli/src/render.rs`
the process side:

| Item | Purpose |
| --- | --- |
| `SceneRenderConfig` | One render job: canvas, fps, layers, collection/profile. |
| `CanvasConfig`, `RenderLayerConfig` | Geometry plus per-layer kind, transform, visibility, z-order, `animated`. |
| `SceneRenderConfig::snapshot` / `render_frame` | Composites frame N into a `SceneSnapshot`, then a `GoldenFrame`. |
| `SceneRenderConfig::write_frame_png` | Deterministic PNG for frame N (`GoldenFrame::to_png_bytes`). |
| `SceneRenderConfig::dominant_source_kind` | The kind that decides the run's reproducibility claim. |
| `render_video` | Pushes `frames` frames through `RivuletEngine` on a `VirtualClock` with the software encoder. |
| `RenderVideoReport` | Frames, duration, fps, dimensions, output size, and the nondeterminism inventory. |
| `frame_pts_ns` | PTS of frame N, matching the W2a timestamp contract. |
| `RenderArgs` / `parse_render_args` | The `rivulet render` flags. |
| `BatchRenderSummary` / `BatchRenderJob` / `JobStatus` | The machine-readable batch report. |
| `render_batch` / `load_scene_config` | One job per config, failures isolated. |

Validation is deliberately strict, because these files are CI inputs: canvas
non-zero and even, `fps` non-zero, at least one layer, unique non-empty layer
names, at most `MAX_LAYERS` (64) layers, and opacity within `0.0..=1.0`. A bad
key is named in the error, and unknown TOML keys are rejected rather than
ignored.

#### Determinism

Frame content comes from `SceneSnapshot::render_rgba()` over synthetic
`TestVideoSource` frames addressed by frame index, so frame N does not depend on
how many frames were rendered before it. An `animated` layer gets a stable
per-layer motion offset; a hidden layer contributes nothing. PNG encoding is the
dependency-free encoder from `GoldenFrame`, which keeps the bytes reproducible
and lets `rivulet render` write a file without pulling in an image crate.

`render_video` never uses the wall clock: it sets a `VirtualClock` *before*
`start_local_recording` (the engine rejects a clock swap once a session is
running) and steps it one frame before each push, so frame 0 is stamped at the
session base rather than one interval in. The report is emitted from
`config.dominant_source_kind()`, so a config naming a live kind (webcam, game
capture) is not advertised as byte-reproducible.

#### CLI

```console
# A still, deterministic and byte-identical on every run.
rivulet render --config scene.toml --frame 7 --png out/frame7.png

# A video through the real encoder.
rivulet render --config scene.toml --video out/scene.mp4 --frames 60

# Batch: one job per config, isolated failures, machine-readable summary.
rivulet render --config-dir scenes/ --out-dir renders/ --json
```

`render` has its own flag vocabulary and parser rather than sharing `record`'s,
because rendering configures a composition instead of a recording. Exit codes
follow the existing convention: `0` success, `1` runtime failure, `2` invalid
usage or config.

The summary records `schema_version`, `out_dir`, `jobs`, `succeeded` and
`failed`; each job carries `config`, `status`, `video`, `png`, `frames`,
`duration_secs` and `error`. Job output paths are relative to `out_dir`, so a
summary from one runner is diffable against another, and a failing job's `error`
already names its own config.

#### CI integration example

`scripts/ci-render-smoke.sh` runs the shipped binary against the two fixture
configs in `scripts/ci-render-smoke/`, and the `Render Smoke` job
(`render_smoke`) runs it on a clean `ubuntu-latest` runner with the GStreamer
runtime and x264 installed. It checks that the same frame rendered twice is
byte-identical, that consecutive frames differ (so the first check cannot pass
by rendering one constant image), that the output carries the PNG signature,
that a batch over two configs reports two succeeded jobs with relative output
paths, that the two scenes produce different stills, and that both videos carry
an MP4 header.

### W4 — Reproducible distribution inputs — issue [#190](https://github.com/thoser666/Rivulet/issues/190)

**Status: shipped.** Two changes and one honest limit.

The manifest generation was already inline in both publishing paths
(`find | sort | xargs sha256sum`, once in `ci.yml`, once in `release.yml`).
Two copies of a shell pipeline that nobody can test is one copy too many, so
it moved to [`scripts/release-manifest.py`](../scripts/release-manifest.py),
which is the single implementation for every channel.

What the script adds over the pipeline it replaced:

- **Determinism** — entries sorted by path, `sha256sum`-compatible
  `<digest><two spaces><name>` lines, LF endings. Two runs over one tree
  produce byte-identical output.
- **Verification**, which did not exist at all. `verify` checks *both*
  directions: every listed file must exist and hash to the recorded digest, and
  every file in the tree must be listed. The second direction is the one worth
  having — a manifest that quietly omits an artifact publishes it without
  integrity coverage, and nothing downstream would notice.
- **A self-test** (`--self-test`) that runs in the *lints* job on every push,
  so a regression in the release-critical path is caught by CI rather than by
  the next release.

**DoD**

- [x] Audit each distribution channel (EXE zip, MSI, Scoop, Chocolatey, WinGet,
  AUR, Flatpak) for build reproducibility gaps; document them — see the
  determinism table in
  [`release-platforms.md`](release-platforms.md#per-channel-determinism).
- [x] Generate SHA-256 manifests per release artifact set as part of the
  release job — both channels call the one script.
- [x] Post-publish verification job: re-download published artifacts, verify
  hashes and (where signing exists) signatures — the new `verify_release`
  job in `release.yml` and `ci.yml`.
- [x] Document which channels can be bit-reproducible and which are
  content-deterministic only (installer GUIDs, timestamps).

**Acceptance criteria**

- [x] A release produces a `SHA256SUMS` manifest covering every published
  **build artifact** — `generate --dir release-assets` runs before the
  release is created, and the post-publish job re-checks it against what was
  actually uploaded. The five documentation assets the release also attaches by
  path are deliberately *not* in the manifest and are excluded from the
  coverage check; `docs/release-platforms.md` spells out why, and the guard
  pins that exclusion list against the workflows.
- [x] Post-publish verification passes on the latest release (or documents
  per-channel deviations honestly) — **deviation, stated plainly**: the
  signature check only runs when the release actually carries a detached
  signature. Signing is secret-gated, so unsigned alpha/tag builds report that
  instead of failing. Nothing pretends to verify a signature that does not
  exist.
- [x] The spec lists per-channel determinism status with reasons — see the
  table linked above; the short version is that **no published installer is
  currently bit-reproducible**, and claiming otherwise would be the easy lie.

**The one thing that is not automated yet:** a rebuild-and-compare job. Proving
*bit* reproducibility means building the same commit twice and diffing, which
needs a second full runner per platform and is not wired up. The manifest
guarantees integrity and coverage, not bit-identity, and the table says so.

### W5 — Pipeline inspector/diagnostics — issue [#191](https://github.com/thoser666/Rivulet/issues/191)

The introspection half of the CLI story, analogous to `gst-inspect` /
`gst-launch` but engine-aware.

**DoD**

- [x] `rivulet inspect`: print the pipeline string the engine would build for a
  given config (both mux legs, audio branches) without running it.
- [x] Element/capability listing: available encoders, capture backends, audio
  filters with feature-detection results as JSON.
- [x] Session diagnostics: on failure, the run report names pipeline, input,
  configuration and failing stage; redaction rules apply (masked keys).
- [x] Dry-run mode for the record command reusing the same code path.

**Acceptance criteria**

- [x] `inspect` output for a config matches the pipeline string the engine
  actually builds (shared code path, test-pinned).
- [x] Feature detection lists encoders/backends as stable JSON; no secrets in
  any diagnostic surface (existing redaction tests extended).
- [x] A failing run names the failing stage in the machine-readable report.

**Implementation notes**

- `RivuletEngine::pipeline_description()` delegates to the same private
  `build_pipeline_str()` the first-frame path uses, so the inspected string
  cannot drift from the built one. It returns the description already passed
  through `redact_pipeline_for_log()`, which is why an ingest URL with an
  embedded stream key shows as `<redacted stream URL>` even in the inspect
  output.
- Capability detection lives in `rivulet_core::FeatureReport` (module
  `rivulet-core/src/inspect.rs`) rather than in the CLI, because availability is
  a property of the local GStreamer installation and core is the only crate
  that links it. Availability means "element factory registered"; a registered
  hardware encoder can still fail to instantiate without the matching
  GPU/driver, which the engine's encoder fallback covers.
- `rivulet_cli::engine_for()` is the single config-to-engine mapping shared by
  `record`, `record --dry-run` and `inspect`. W1 validated `--container` but
  never applied it to the engine, so every container produced MP4; the shared
  mapping applies `set_recording_container`, which both fixes that gap and is
  what lets `inspect` report the muxer a real run would use.
- A failing run is machine-readable, not just prose (AC: "a failing run names
  the failing stage in the machine-readable report"). `rivulet_cli::RunFailure`
  carries the `Stage`, an actionable message and — when known — the redacted
  pipeline and the resolved config; the binary renders it as a `failed` object
  on stdout plus a human line on stderr. The stages are `usage`, `config`,
  `output`, `engine` and `finalize`, and the stage selects the exit code, so the
  two can never disagree. The engine's existing
  `pipeline_parse_failure_message()` (GStreamer domain + code) is the `engine`
  stage's message; the `finalize` stage carries an engine error reported at stop
  time, falling back to the empty-output complaint.


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

- [x] Paste of the same clipboard content twice produces identical scene state
  (deterministic id generation, test-pinned).
- [x] Cross-scene paste does not move the item out of the source scene
  (duplicate semantics, not move).
- [x] Undo restores the pre-paste scene state exactly.
- [x] The operation is covered by the deterministic-test helpers from W2b.
## CLI surface reference

```
rivulet record --config recording.toml [--output FILE] [--duration SECS]
               [--width PX] [--height PX] [--fps N] [--audio]
               [--container mp4|mkv|mov|mpegts] [--dry-run]   (shipped, W1)
rivulet inspect --config recording.toml [--json]            (shipped, W5)
rivulet render --config scene.toml --frame N --png out.png    (shipped, W3)
rivulet render --config-dir scenes/ --out-dir renders/        (shipped, W3 batch)
```

Flags override the TOML config (`--output` wins over `output.path`, etc.).
Without `--config`, flags alone define the run; `output.path` is always
required (inline via `--output`). The library path
(`rivulet_cli::RecordJob`) accepts the same validated `RecordConfig`, so the
identical recording is achievable in-process without spawning the binary
(W1 DoD).

`rivulet inspect` (W5) never records and never creates the output file:

- Plain mode prints the pipeline string, the resolved output/container/audio
  settings, and the detected capabilities.
- `--json` prints one JSON object — the machine-readable report.
- `rivulet record --dry-run` prints the same pipeline for the record path and
  exits 0 without pushing a frame.
- `--duration`, `--width`, `--height`, `--fps` and `--dry-run` are rejected
  under `inspect` (exit 2) rather than silently ignored.

Inspect JSON schema (shipped in W5) — one object with `pipeline`, the echoed
`config`, and `features`:

| Field | Contents |
| --- | --- |
| `pipeline` | the pipeline string the engine would build, redacted |
| `config` | the fully resolved `RecordConfig` (self-describing report) |
| `features.gstreamer_version` | the GStreamer version probed against |
| `features.encoders[]` | `backend`, `backend_label`, `hardware`, `codecs[]` (`codec`, `codec_label`, `element`, `available`) |
| `features.containers[]` | `container`, `label`, `muxer`, `available`, `crash_safe` |
| `features.capture_backends[]` | `name`, `label`, `element`, `available` |
| `features.audio_filters[]` | `name`, `label`, `element`, `available` |

The order of every list is fixed (`nvenc, quicksync, amf, software`; `h264,
h265, vp9`; `mp4, mkv, mov, mpegts`) so two reports diff cleanly, and the shape
is pinned by the `cli_inspect_surface_is_pinned` ci_pinning test.


Exit codes (shipped in W1):

| Code | Meaning |
| --- | --- |
| 0 | success — including a graceful SIGINT/SIGTERM stop that finalized the container (per the W1 acceptance criterion) |
| 1 | generic runtime failure (engine error, IO error, empty output) |
| 2 | invalid usage or configuration (error names the offending key, e.g. `output.container: unknown container …`) |

These codes are fixed by W1; which one a failure gets is derived from its
`stage` (the table below), so the exit status and the machine-readable report can
never disagree.

JSON status event schema (shipped in W1): one JSON object per line on stdout
with a stable `event` discriminator —

| Event | Fields |
| --- | --- |
| `started` | `width`, `height`, `fps`, `audio` |
| `progress` | `seconds`, `frames`, `fps`, `file_size_bytes` |
| `stopped` | `frames`, `seconds`, `file_size_bytes`, `clock`, `pts_source`, `encoder`, `reproducible`, `byte_reproducible`, `nondeterminism` (shipped in W2a) |
| `failed` | `stage`, `exit_code`, `message`, `pipeline` (optional), `config` (optional) (shipped in W5) |

Failures are part of the same JSON stream: a failing run appends a final
`failed` object to stdout and writes the human diagnostic to stderr. The streams
stay separated — prose never lands on stdout — but a JSON consumer still learns
*where* the run broke without scraping text, and never mistakes a broken run for
an empty successful one (AC: diagnostics never mix into the JSON stream).

Failure report schema (shipped in W5) — the last stdout object of a failed run:

```json
{"event": "failed", "stage": "engine", "exit_code": 1, "message": "engine error: bus error", "pipeline": "appsrc … ! mp4mux ! filesink", "config": {"output": {"path": "out.mp4"}, "video": {"source": "test"}, "audio": {"enabled": false}}}
```

| Field | Contents |
| --- | --- |
| `event` | always `"failed"` |
| `stage` | `usage`, `config`, `output`, `engine` or `finalize` |
| `exit_code` | the code the process ended with; derived from `stage`, never chosen separately |
| `message` | actionable, secret-free description — names the offending config key where there is one |
| `pipeline` | the redacted pipeline, when one was built before the failure |
| `config` | the fully resolved `RecordConfig`, when one was parsed |

`pipeline` and `config` are omitted rather than serialized as `null` when the
failure happened before they existed (a usage error has no config), so a
consumer can tell "not reached yet" from "empty".

| Stage | Exit | Meaning |
| --- | --- | --- |
| `usage` | 2 | invalid command-line arguments |
| `config` | 2 | invalid config file or values |
| `output` | 1 | the output location could not be prepared |
| `engine` | 1 | the engine failed while pushing frames or audio |
| `finalize` | 1 | the engine failed while stopping, or produced no output |

Redaction applies unchanged: the `pipeline` a failure reports is the same
redacted string `inspect` prints, so a stream key never reaches the report. The
success and failure schemas are pinned by the `cli_mvp_schema_and_docs_are_pinned`
and `w5_machine_readable_failure_is_pinned` ci_pinning tests.

## Nondeterminism inventory

The reproducible-run contract (W2a) holds only within these limits, which the
run report must state explicitly:

| Source | Behavior | Reporting |
| --- | --- | --- |
| Engine clock (PTS, session duration) | deterministic under the virtual clock | `active` when the system clock drives the run; `affects_timestamps` |
| Encoder rate-control state (x264 lookahead/VVB) | deterministic for identical input frames; first-frame effects depend on settings | always `active`; `affects_bytes` |
| Hardware encoders (NVENC/QSV/VAAPI) | not bit-deterministic across runs/driver versions | `active` when a hardware backend encoded; `affects_bytes` |
| Wall-clock-derived metadata (creation_time, capture timestamps) | nondeterministic by nature, even under the virtual clock | always `active`; `affects_bytes` |
| Capture-backed sources (screen/camera/mic) | live input cannot repeat | `active` for live `SourceKind`s; `affects_bytes` |
| GStreamer element threading | fixed scheduling under the virtual clock; element-internal threads (queue leaks) may reorder | `timestamp_risk: true` — validated by the reproducibility test |

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
