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

- **Automated accessibility** — two contracts guard the AT story:
  - the confirmation dialog warehouse (`draw_confirmation_modal`) renders every
    destructive action (composition source delete, audio source remove, chat
    account remove) behind a modal whose Confirm button uses the error palette,
    while `Esc`/backdrop click/Cancel discard it — covered in `subject()` /
    `ConfirmDecision` style tests in `rivulet-gui/src/app.rs`,
  - the focus-order contract (`tests/ui_accessibility.rs`) asserts that the
    navigation sidebar renders every view as a focusable, focus-ring drawn
    `selectable_label` in document order.
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
  accessibility report that asserts semantic contracts for the primary workflows,
  including the AccessKit bridge contract (`screen_reader_bridge_enabled`): the
  native AT bridge compiles in on non-Linux targets (Windows UIA/Narrator, macOS
  AX/VoiceOver) via the target-gated `accesskit` eframe feature, and stays off on
  Linux only because accesskit_unix talks zbus from a foreign thread next to the
  Tokio reactor (documented in `rivulet-gui/Cargo.toml`).

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
| ui-003 | Info | UX audit v0.65a | Screen readers (Windows/Mac) had no native AT bridge: AccessKit was unconditionally disabled | Closed — AccessKit now compiles in on non-Linux targets (target-gated `accesskit` feature) |

Open findings are intentionally low severity; they track ongoing hardening rather
than known release blockers. When a finding is fixed, move it below under
"Closed findings" with the commit/branch that resolved it.

## Closed findings

| ID | Severity | Area | Finding | Resolved by |
| --- | --- | --- | --- | --- |
| ui-003 | Info | UX audit v0.65a | Screen readers (Windows/Mac) had no native AT bridge: AccessKit was unconditionally disabled | `rivulet-gui/Cargo.toml` target-gated `accesskit` feature for `cfg(not(target_os = "linux"))` |
| ui-004 | Info | UX audit v0.65a | Destructive actions (source delete, audio source remove, chat account remove) ran immediately without confirmation | `PendingConfirmation` + `draw_confirmation_modal` in `rivulet-gui/src/app.rs`; Esc/backdrop/cancel discard, confirm uses error palette |
