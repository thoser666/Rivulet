# M2 UI/UX Review Report

- Commit/build: `bf780fa` (CI run `33039433165`)
- Review date (UTC): 2026-08-27
- Reviewers: Rivulet maintainers (automated evidence review; manual platform execution noted below)
- Profiles executed: P1–P5 evidence matrix; automated cross-platform builds and quality checks
- Overall result: `CONDITIONAL PASS`

## Summary

The functional M2 checklist and automated cross-platform baseline pass. The UI/UX gate has
no open Blocker or Critical findings. One High-risk platform limitation is explicitly
assigned to M5: native macOS capture parity is not yet implemented. It is not silently
accepted as parity; the limitation remains visible in the roadmap and is a release blocker
for the beta gate.

- Blocker/Critical findings: **0**
- High findings: **0 open**; platform limitation assigned to M5 / existing parity tracking
- Deferred Medium/Low findings: native second-display projector routing, native source-pixel
  rendering, and native browser WebView adapter wiring
- Known platform limitations: macOS capture is unavailable in the current GUI path;
  Linux PipeWire requires a compatible portal/display session; projector routing currently
  uses an in-window fallback rather than a native second-display window.

## Automated checks

| Check | Result | Evidence |
| --- | --- | --- |
| Format | PASS | `cargo fmt --all -- --check` on the reviewed commit |
| Tests | PASS | Workspace/core/GUI regression suites in CI |
| Clippy/lint | PASS | CI Lints job with `-D warnings` |
| CI-specific checks | PASS | Actionlint, pinning, WCAG, asset drift, G5, signing E2E |
| Windows build | PASS | CI `Build & Test on windows-latest` |
| Linux build | PASS | CI `Build & Test on ubuntu-latest` |
| macOS build | PASS | CI `Build & Test on macos-latest` |

## Workflow results

The checklist in [`m2-ui-ux-review.md`](m2-ui-ux-review.md) is the normative matrix. The
following records the available reproducible evidence for the reviewed commit.

| Profile | Passed | Failed | Blocked / N/A | Evidence |
| --- | --- | --- | --- | --- |
| P1 Windows 100% dark | W01–W15, H01–H14 automated/UI-contract checks | None | Native projector routing: N/A | Windows CI build; GUI tests; theme/WCAG checks |
| P2 Windows 150% light | W01–W03, W05–W11, W13; H01–H14 contract checks | None | Native projector routing: N/A | Light/dark palette tests; responsive UI code paths |
| P3 Linux X11 100% dark | W01–W15, H01–H14 automated/UI-contract checks | None | Portal-specific behavior: environment-dependent | Linux CI build; PipeWire/xcap tests; capture diagnostics tests |
| P4 Linux Wayland 125% light | W01–W04, W08–W14 contract checks | None | Portal permissions/display session: environment-dependent | Linux CI build; portal fallback tests; WCAG checks |
| P5 macOS Retina/system | Build, navigation, settings, scene UI contract checks | None | Capture workflows: platform limitation | macOS CI build; platform-gated GUI compilation |

No real credentials, stream keys, or personal paths were used in evidence. Hardware- and
display-server-dependent checks are marked N/A/blocked with the reason above rather than
claimed from compilation alone.

## Heuristic and accessibility results

| Check group | Result | Evidence / follow-up |
| --- | --- | --- |
| Information hierarchy and action consistency | PASS | AppView sidebar, view headings, primary controls, and shared accent helpers |
| Source identity and feedback | PASS | Independent source/window state, live preview status, refresh behavior, localized errors |
| Error recovery and diagnostics | PASS | GUI error channels, daily logs, pre-Rust diagnostics, no-frame timeout |
| Keyboard access and focus visibility | PASS | Hotkey tests; egui focus handling; theme palette/focus stroke implementation |
| Contrast and themes | PASS | WCAG script/tests for both dark and light palettes |
| Scaling and responsive layout | PASS | Responsive preview sizing; 125%/150% layout contract reviewed |
| Localization and privacy | PASS | EN/DE i18n coverage; no secrets in diagnostics contract |
| Responsiveness and motion | PASS | Non-blocking transitions, throttled previews, bounded refresh loops |

## Findings

| ID | Severity | Profile / check | Description | Reproduction | Issue / owner | Retest |
| --- | --- | --- | --- | --- | --- | --- |
| F-M2-001 | Medium | P1/P2 projector | Native fullscreen routing to a second display is not available; the UI uses an in-window projector fallback. | Enable Multi-view and open Projector in Scenes. | M5 platform parity / renderer owner; existing issue #75 | Retest at native projector implementation |
| F-M2-002 | Medium | P3/P4 source rendering | Snapshot/composition uses deterministic layout tiles until native source-pixel renderers are connected. | Export a scene snapshot with native sources. | M9 renderer owner; documented follow-up | Retest when native renderer lands |
| F-M2-003 | Medium | P5 browser source | Browser configuration is portable, but native WebView adapter wiring remains unavailable. | Configure a browser source and attempt native rendering. | M5 ecosystem owner; S5b follow-up | Retest when platform adapters land |

There are no Blocker, Critical, or open High findings. The macOS capture limitation is an
advertised platform gap and remains assigned to the M5 parity milestone; it is not counted as
an open gate finding because the current M2 scope does not advertise native macOS capture.

## Decision

- [ ] M2 UI/UX gate passed without conditions.
- [x] Conditional pass; all remaining findings are Medium and assigned to follow-up work.
- [ ] Failed; release-blocking work remains.

M2 is complete for its functional scope. The milestone must not be used to claim beta
platform parity: native projector routing, native source rendering, browser adapters, and
macOS capture parity remain explicit post-M2 work. The report is linked from the M2 roadmap
gate in `README.md`; release notes should reference this report.
