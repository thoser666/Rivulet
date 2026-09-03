# UI Audit

This document records the current state of the Rivulet desktop UI and how the
project audits it. It is the living counterpart to the one-shot
[M2 UI/UX review gate](m2-ui-ux-review.md): where that gate produces a snapshot
report per milestone, this file describes the **ongoing** contracts that keep the
UI accessible, deterministic, and coherent between milestones.

## Scope

The audit covers the `rivulet-gui` (egui/eframe) application: navigation between
the primary views (Record, Scenes, Settings, Diagnostics, ...), theme rendering,
interaction states, accessibility semantics, and the privacy of UI reports. It
is deliberately separate from audio/capture correctness, which is covered by the
engine tests and the streaming smoke tests.

## Current state (v0.65 alpha)

### Theme polish

The shared theme module (`rivulet-gui/src/theme.rs`) implements the visual layer:

- **Glassmorphism panels** — `theme::glass_frame()` renders translucent panel
  surfaces for the modern, layered look.
- **Interaction strokes** — `theme::hover_stroke()` (1.5 px accent at ~60 % alpha)
  and `theme::active_stroke()` (2 px opaque accent) highlight hover/focus and
  active/pressed controls without overriding egui's disabled visuals.
- **Preview fade-in** — `theme::preview_fade_alpha()` wraps `ctx.animate_bool` so
  game-preview tint fades in smoothly; the Windows and Linux preview renderers use
  `from_white_alpha` so the alpha applies consistently across platforms.

These are covered by the theme unit tests in `rivulet-gui/src/theme.rs`.

### Automated contracts (CI-gated)

The following tests enforce stable UI contracts and run on every CI push in the
`Build & Test` matrix:

- **UI smoke** (`rivulet-gui/tests/ui_smoke.rs`) — verifies that
  - navigation covers all primary views and lists them through a stable
    `AppView::all()` contract,
  - keyboard handlers do not fire global shortcuts while a text field is focused,
  - diagnostics are surfaced in the GUI,
  - the deterministic screenshot report redacts secrets (stream keys, ingest URLs,
    internal secret-field names),
  - an accessibility contract requires labels, focus, and contrast.
- **UI regression** (`rivulet-gui/tests/ui_regression.rs`) — deterministic
  viewport/interaction snapshot contracts that are independent of the host font
  and GPU, so visual output stays stable across machines.
- **UI accessibility** (`rivulet-gui/tests/ui_accessibility.rs`) — a stable
  accessibility report that asserts semantic contracts for the primary workflows.

### Contrast

`scripts/check-theme-contrast.py` checks the status palette against WCAG AA in
both dark and light themes and runs in the CI `lints` job.

## Audit procedure

For a milestone gate, run the full procedure in
[m2-ui-ux-review.md](m2-ui-ux-review.md) (workflow matrix W01-W15, heuristics
H01-H14, test matrix P1-P5). For the ongoing/continuous audit, run the following
before each feature commit that touches the GUI:

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test -p rivulet-gui --test ui_smoke --verbose`
4. `cargo test -p rivulet-gui --test ui_regression --verbose`
5. `cargo test -p rivulet-gui --test ui_accessibility --verbose`
6. `python3 scripts/check-theme-contrast.py`

Manual spot checks remain necessary for real-target rendering (fonts, DPI
scaling, compositor behavior) that the headless contracts cannot fully replace.

## Findings backlog

| ID | Severity | Area | Finding | Tracker / owner |
| --- | --- | --- | --- | --- |
| ui-001 | Low | Accessibility scanning | No automated accessibility scan of the rendered UI on every PR; currently only the in-process accessibility report and contrast checker run | Enable a GitHub accessibility/automated scan app (see `docs/security.md` / GitHub app enablement); owner: maintainer |
| ui-002 | Low | Scaling | Text-scaling (125 %/150 %) was verified in the M2 gate but is not yet a CI contract | Consider extending `ui_regression` snapshots; owner: maintainer |

Open findings are intentionally low severity; they track ongoing hardening rather
than known release blockers. When a finding is fixed, move it below under
"Closed findings" with the commit/branch that resolved it.

## Closed findings

_None yet._ Record fixed findings here with the resolving commit.
