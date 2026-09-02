# Milestone Quality Gates

This document defines the product-quality gate for every milestone after the
recording foundation. It complements automated tests and code review with a
repeatable check of usability, accessibility, platform behavior, diagnostics,
and user trust.

A milestone is not complete merely because its implementation and unit tests are
green. Its advertised workflows must also be understandable, recoverable, and
usable on the platforms and display configurations that the milestone supports.

## Gate levels

Use the smallest gate that matches the change, then run the full gate before a
milestone is closed or a release channel changes.

| Level | When to use | Required evidence |
| --- | --- | --- |
| Feature check | One isolated feature or bug fix | Focused workflow, regression test, error path |
| Milestone check | Milestone implementation complete | Common checks plus milestone-specific matrix |
| Release check | Beta, RC, or stable release | Full platform matrix, accessibility review, diagnostics, open-findings decision |

The M2 review in [`m2-ui-ux-review.md`](m2-ui-ux-review.md) is the first full
milestone-check implementation. Future milestone reports should use the same
structure and link their evidence from release notes.

## Common gate for every milestone

Run these checks for all user-facing work, including CLI, API, and background
features where the equivalent developer workflow applies.

### Core workflow

- The primary task can be completed from a clean profile without undocumented
  setup steps.
- The current state, active target, progress, and completion result are visible.
- Actions have predictable placement, labels, icons, and disabled states.
- Long names, empty states, first-run states, and unavailable hardware have an
  intentional presentation.
- Cancel, retry, undo, and recovery behavior are clear where an operation can
  fail or take more than a moment.

### Accessibility and visual quality

- Keyboard navigation reaches all primary actions and has a logical focus order.
- Focus is visible in dark and light themes and is not conveyed by color alone.
- Text, controls, status colors, and disabled states remain readable at the
  supported display scales, including 125% and 150% on desktop platforms.
- No important labels, controls, status messages, or dialogs overlap or clip at
  the minimum supported window size.
- Localization does not mix languages in one workflow and translated text fits
  its containers.
- Motion is brief, purposeful, non-blocking, and does not hide state changes.

### Reliability and recovery

- The UI thread remains responsive during capture, encoding, streaming, update,
  refresh, and transition operations.
- Failure states are shown in the GUI, not only in a console or log file.
- Logs and crash diagnostics identify the failing operation without exposing
  stream keys, credentials, or unnecessary personal paths.
- Restarting after a failure does not silently discard persistent settings or
  leave a false active state.
- Platform limitations and fallback backends are named in the UI and docs.

### Automated baseline

Run the checks appropriate for the repository state before manual review:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Also run the applicable CI checks: workflow linting, ShellCheck, action-pin
validation, asset drift, signing/packaging tests, and parity checks. Hardware,
GPU, WebView, and display-server checks must be marked `PASS`, `BLOCKED`, or
`N/A` with a reason; they must not be silently omitted.

## Cross-cutting resource-efficiency gate

This gate applies to **every milestone M0–M10**, in addition to the milestone-specific checks below. It is scaled to the feature: a CLI or API feature may use process CPU/RAM and determinism measurements, while capture/rendering features must also measure frame time, GPU, queues, and power where available.

Use [`resource-efficiency-goal.md`](resource-efficiency-goal.md) as the source of thresholds and methodology. Every milestone report must record:

- CPU and GPU impact compared with a documented baseline;
- p95/p99 frame-time impact and 1% lows for interactive video/game workflows;
- memory usage and growth over a representative session;
- behavior when a resource or hardware budget is unavailable;
- queue bounds, cancellation, and UI responsiveness;
- `PASS`, `BLOCKED`, or `N/A` for hardware-only measurements, including a reason.

| Milestone | Minimum resource-efficiency evidence |
| --- | --- |
| M0 | Capture/recording CPU, GPU, RAM, frame-time baseline |
| M1 | Encoder, audio, overlay, replay-buffer overhead |
| M2 | Scene composition, preview, transitions, and source-count scaling |
| M3 | Fan-out queues, reconnect isolation, bitrate and delay overhead |
| M4 | Replay, filters, remux, and virtual-camera resource behavior |
| M5 | Plugin, hotkey, updater, and cross-platform overhead/fallback behavior |
| M6 | Headless resource limits and deterministic CI execution |
| M7 | Host-application overhead, lifecycle, and bounded background work |
| M8 | GPU-direct, zero-copy, renderer, thermal, and battery measurements |
| M9 | Local model CPU/RAM/VRAM budget, responsiveness, and graceful degradation |
| M10 | Layout persistence, registry/plugin overhead, sandbox limits, and failure isolation |

A milestone cannot claim resource efficiency from compilation or unit tests alone. Attach the report or mark the measurement `BLOCKED`/`N/A`; never silently omit it.

## Milestone-specific gates

### M2: Scenes and Composition

Use the complete workflow and accessibility matrix in
[`m2-ui-ux-review.md`](m2-ui-ux-review.md). The review must cover:

- Scene creation, organization, collections/profiles, duplication, and removal.
- Preview/Program Studio Mode, Take, Cut/Fade transitions, and recovery from a
  failed or interrupted transition.
- Source identity, per-scene composition, transforms, crop, visibility, lock,
  z-order, and undo/redo.
- Game/window/monitor selection, live preview refresh, and platform fallbacks.
- Small windows, DPI scaling, dark/light themes, keyboard focus, localization,
  and empty/error states.

Exit rule: no open Blocker or Critical finding; every High finding is fixed or
assigned to a follow-up milestone with an owner and issue.

### M3: Streaming

Review the complete lifecycle on every supported streaming platform or protocol:

- Configure a destination without displaying or persisting a stream key in the
  wrong place; the UI must mask the key and logs/diagnostics must never include it.
- Validate URL, platform, codec, audio, and bitrate settings before connecting;
  reject empty keys and non-TLS URLs for platform presets before starting.
- Verify Twitch, YouTube, Kick, and custom presets show the selected destination,
  quality preset, and effective bitrate without requiring the user to remember
  endpoint details.
- Start, observe, pause or stop, reconnect, and recover from network loss.
- Distinguish Connecting, Good, Warning, Poor, Offline, and authentication
  failure states with actionable text.
- Verify dual output does not duplicate controls or hide which output failed.
- Confirm stream-key masking, safe clipboard behavior, and log redaction.
- Verify adaptive bitrate is opt-in, stays within configured minimum/maximum bounds,
  lowers quality on Poor health, and recovers conservatively on Good health; the
  current policy layer must not be presented as live encoder reconfiguration until
  that integration is complete.
- Verify stream delay is opt-in, bounded, visible in status, and does not block the
  UI; test zero, typical, and maximum accepted delay values.
- Verify Multitrack Video clearly reports the selected track count and refuses
  unsupported values; do not claim per-track transport negotiation until it is
  implemented.
- Test narrow windows and long platform/server names.

Exit evidence: a redacted stream-health recording or screenshots, preset and
validation results, adaptive-bitrate boundary tests, delay-boundary tests,
multitrack configuration tests, reconnect results, and a protocol/platform
compatibility table.

### M4: Advanced Output and Capture

Review output safety and data-loss behavior:

- Save Replay is discoverable, reports the exact output path, and handles an
  empty or too-short buffer without a false success.
- Remux, format conversion, file splitting, and recording recovery explain
  progress, cancellation, overwrite behavior, and partial files.
- Filters expose safe defaults, reset behavior, bypass state, and meaningful
  unsupported-plugin errors.
- Virtual camera lifecycle is visible to other applications and has a clear
  stop/release path.
- Audio ducking, master mix, and VST/plugin failures cannot silently mute the
  wrong source or crash the recording.

Exit evidence: valid output files, failure/recovery cases, and a data-loss review.

### M5: Ecosystem and Platform Parity

Review trust, permissions, installation, and cross-platform consistency:

- Installer, updater, signing failure, rollback, and restart paths are clear.
- Plugin installation identifies publisher, permissions, compatibility, and
  sandbox status before activation.
- OBS compatibility mode is explicitly marked as a compatibility/risk boundary.
- Global hotkeys and remapping show conflicts, scope, reserved keys, and reset.
- Windows, macOS, and Linux expose equivalent core workflows or clearly label
  platform-specific limitations.
- Accessibility, locale coverage, and diagnostics are checked in each package.

Exit evidence: platform feature matrix, installer/update recordings, and plugin
trust/permission review.

### M6: Automation and Determinism

This is a developer-experience gate as well as a UI gate:

- CLI help, examples, configuration errors, and exit codes are actionable.
- A clean invocation produces deterministic output or reports every source of
  nondeterminism.
- Progress and cancellation work in terminals and CI without interactive-only
  assumptions.
- JSON/status output is stable, documented, and separated from human-readable
  diagnostics.
- Logs identify the pipeline, input, configuration, and failing stage without
  leaking secrets.
- Golden-frame, timestamp, and reproducibility failures show useful diffs.

Exit evidence: clean-machine command transcript, reproducibility comparison,
and machine-readable schema validation.

### M7: Embeddable Engine and API

Review the API as a product consumed by another developer:

- Public names, defaults, error types, lifecycle, and thread-safety guarantees
  are coherent and documented.
- Examples compile and cover recording, streaming, dual output, encoders, and
  frame delivery.
- Feature detection and fallback results are available without parsing logs.
- Versioning, deprecation, migration, and compatibility policy are explicit.
- Invalid configuration fails early with typed, actionable errors.
- API docs and examples work without depending on GUI-only state.

Exit evidence: docs build, example build/run matrix, public API review, and a
small downstream-consumer smoke test.

### M8: Modern Architecture

Review perceived performance and hardware fallback behavior:

- Startup, idle, preview, scene switch, and recording transitions remain
  responsive on supported hardware tiers.
- CPU, GPU, memory, battery, and frame-time behavior is measured rather than
  inferred from visual impression; apply the game-first targets in
  [`resource-efficiency-goal.md`](resource-efficiency-goal.md).
- GPU-direct and fallback paths show which backend is active and why.
- Scaling, high-DPI rendering, color handling, and resize behavior remain stable.
- Effects and compute filters provide progress or frame-budget feedback when
  they cannot keep up.
- Capture and streaming leave bounded headroom for the game; p95/p99 frame-time,
  idle CPU, memory stability, and fallback evidence are recorded.
- Unsupported GPUs degrade gracefully without blank previews or silent output.

Exit evidence: representative performance report, backend matrix, and screenshots
or recordings at the minimum and recommended hardware profiles.

### M10: Extensible UI and Plugin Platform

Review customization without sacrificing safety, accessibility, or responsiveness:

- A clean profile opens with a safe default layout; persisted layout state survives
  restart and is migrated from every supported schema version.
- Layout persistence stores only versioned, non-sensitive preferences; secrets,
  tokens, private endpoints, runtime handles, and process state are excluded.
- Built-in and optional views use one stable registry with deterministic ordering,
  collision handling, localization, keyboard focus, and accessible labels.
- Plugin manifests declare API version, publisher/integrity information, requested
  capabilities, and compatibility before activation.
- UI-only plugins receive no network, filesystem, capture, audio, or secrets access
  unless explicitly granted; denied capabilities produce actionable feedback.
- A plugin cannot freeze or terminate the GUI: timeouts, resource limits, cancellation,
  and crash isolation are observable and tested.
- Disabled, incompatible, corrupted, and removed plugins leave the core navigation
  and persisted layout usable.
- Plugin panels remain readable at supported DPI scales, in both themes and locales,
  and do not obscure primary recording/streaming controls.
- The plugin registry and runtime stay within the documented CPU, memory, startup,
  and frame-time budgets; idle plugins do not force continuous repainting.

Exit evidence: layout round-trip and migration tests, registry collision/accessibility
checks, permission-denial tests, malformed-manifest tests, plugin timeout/crash
isolation evidence, and a resource-efficiency report for an enabled/disabled plugin set.

### M9: AI Chat Assistant

Review privacy, control, and failure boundaries:

- The user can tell whether a request is processed locally or sent to a cloud
  endpoint before enabling it.
- Provider, model, channel, persona, memory, and moderation settings are
  discoverable and persist as documented.
- Tokens and chat content are masked in logs, exports, screenshots, and errors.
- The assistant distinguishes suggestions from actions and requires explicit
  confirmation for destructive or externally visible operations.
- Rate limits, provider outages, malformed responses, moderation blocks, and
  context overflow have understandable recovery paths.
- Bot coexistence and duplicate-response suppression are visible and testable.

Exit evidence: privacy review, redacted chat transcript, provider failure cases,
and confirmation/permission checks.

## Platform and viewport matrix

For a release-level gate, use at least these profiles and expand them for the
supported hardware. `N/A` requires a documented platform limitation and issue.

| Profile | Environment | Common checks |
| --- | --- | --- |
| P1 | Windows, 100%, dark theme | Core workflow and milestone-specific checks |
| P2 | Windows, 150%, light theme | Accessibility, layout, localization, recovery |
| P3 | Linux X11, 100%, dark theme | Core workflow and capture/backend checks |
| P4 | Linux Wayland, 125%, light theme | Portal, scaling, permissions, recovery |
| P5 | macOS, Retina/system theme | Core workflow, package, parity, fallback checks |
| P6 | Clean profile / clean machine | First-run, installer, update, defaults, diagnostics |

Do not claim platform parity from compilation alone. A compiled binary is not
proof that capture, permissions, rendering, audio, update, or recovery workflows
are usable on that platform.

## Findings and decision record

Use the following severity definitions:

- **Blocker:** the advertised workflow cannot be completed.
- **Critical:** data loss, unsafe credential handling, unrecoverable freeze/crash,
  or a release platform cannot perform its core workflow.
- **High:** a core task is misleading, inaccessible, or repeatedly fails with
  only a fragile workaround.
- **Medium:** substantial friction or inconsistent feedback without data loss.
- **Low:** non-blocking polish, copy, spacing, or minor visual inconsistency.

Record findings with a stable ID, profile, reproduction, owner, issue, and retest
result. A milestone decision must be one of:

- `PASS`: no Blocker/Critical findings and High findings are closed.
- `CONDITIONAL`: only documented Medium/Low work remains, or High work has an
  explicit owner, issue, and follow-up milestone accepted by the release owner.
- `FAIL`: any release-blocking finding remains without an approved mitigation.

## Report template

Create `docs/<milestone>-quality-report.md` from this template:

```markdown
# <Milestone> Quality Report

- Commit/build:
- Review date (UTC):
- Reviewers:
- Profiles executed:
- Result: PASS / CONDITIONAL / FAIL

## Automated checks

- Format:
- Tests:
- Clippy/lint:
- CI-specific checks:

## Workflow results

| Workflow | Windows | Linux | macOS | Evidence / issue |
| --- | --- | --- | --- | --- |

## Accessibility and usability

| Check | Result | Evidence / issue |
| --- | --- | --- |
| Keyboard/focus |  |  |
| Contrast/themes |  |  |
| Scaling/layout |  |  |
| Localization |  |  |
| Error/recovery |  |  |
| Responsiveness |  |  |
| Privacy/log redaction |  |  |

## Findings

| ID | Severity | Reproduction | Owner / issue | Retest |
| --- | --- | --- | --- | --- |

## Decision

- [ ] Milestone gate passed.
- [ ] Conditional pass with assigned follow-ups.
- [ ] Gate failed; release-blocking work remains.
```

Keep completed reports linked from the milestone release notes. The checklist
should evolve only when a new regression, platform, workflow, or security risk
requires an additional check.
