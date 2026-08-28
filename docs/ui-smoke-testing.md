# Cross-platform UI smoke testing

Rivulet has a deterministic, headless UI smoke-test contract in
`rivulet-gui/tests/ui_smoke.rs`. It runs on Linux, Windows, and macOS without
requiring a GPU, display server, or native window manager.

## Covered checks

- Primary sidebar navigation is complete and has stable translation keys.
- Global keyboard shortcuts are guarded while a text field owns keyboard input.
- Capture and engine errors have a GUI-visible receiver and status field.
- The smoke screenshot contract uses a fixed 1280x800 viewport and verifies that
  navigation and status regions are present.
- Screenshot-like text is checked for stream-key and RTMP URL leakage.
- Focus/hover semantics and theme contrast hooks are present.
- The Stream view exposes health state, rate, queue fill, underflow/overflow
  counters, and sink latency with text labels; status is not conveyed by color
  alone.
- When multistreaming is configured, every target has its own labeled status,
  queue fill, underflow, and overflow values; one target's failure remains
  distinguishable from healthy targets.

The test intentionally checks the stable UI contract instead of pixel-perfect
raster output. Font, DPI, GPU driver, and compositor differences make exact
PNG snapshots unreliable across operating systems.

## Running locally

```bash
cargo test -p rivulet-gui --test ui_smoke
cargo test --workspace
```

For a manual evidence run, start the GUI at 1280x800 on each target platform,
visit Record, Scenes, Stream, and Settings, exercise Tab/Shift+Tab and the
recording error path, then attach redacted screenshots to the milestone review.
Do not include stream keys, personal paths, or window titles containing private
information.

## Accessibility contract

The `ui_accessibility` integration test is a deterministic semantic guard for
native egui screens. It checks that primary actions have labels, navigation has
a defined focus order, errors are surfaced in the UI, status is not conveyed by
color alone, narrow layouts remain responsive, and secrets remain redacted.
These assertions are intentionally platform-neutral; manual accessibility
review at the viewport/theme profiles remains required by the milestone gates.

Run it locally with:

```bash
cargo test -p rivulet-gui --test ui_accessibility
```

CI runs this test in the existing Windows, Linux, and macOS build matrix.

## egui regression snapshots

The CI matrix also runs `rivulet-gui/tests/ui_regression.rs`. It verifies stable
viewport contracts at 1280x800, 1024x768, and 800x600 plus deterministic
interaction sequences for sidebar navigation, Tab focus cycling, Undo/Redo, and
visible diagnostics.

These are semantic snapshots rather than pixel goldens. They intentionally avoid
font- and GPU-dependent PNG comparisons while still detecting accidental loss
of required controls or interaction paths. Native visual evidence remains part
of the milestone gate.

## Limitations

This headless contract does not replace manual visual review or a native
accessibility-tree scan. The M2/M3 quality gates therefore still require
platform evidence for DPI scaling, screen readers, native dialogs, and
Wayland/X11 or Windows/macOS compositor behavior.
