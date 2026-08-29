# Virtual camera (M4)

Rivulet now provides a platform-neutral virtual-camera contract in
`rivulet-core::virtual_camera`. It deliberately separates configuration and
lifecycle from platform driver code.

## Contract

`VirtualCameraConfig` validates:

- a non-empty device name;
- non-zero dimensions up to 8K;
- a frame rate from 1 to 240 FPS;
- RGBA, BGRA, and NV12 format selection.

`VirtualCamera` exposes explicit states:

`Stopped → Starting → Running → Stopping → Stopped`

A backend must acknowledge startup and shutdown with `mark_running()` and
`mark_stopped()`. Invalid transitions are rejected instead of silently claiming
that a camera is live. Driver or permission failures are represented by
`Unavailable(reason)` or `Error(reason)` and preserve an actionable reason for
the UI and diagnostics.

## Scope boundary

This increment is the reusable model and lifecycle foundation. It does not yet
open a real camera device. The next M4 work package adds platform adapters
(Windows, macOS, and Linux), format negotiation, permission handling, and a
consumer smoke test. Until then, the UI must not advertise a running virtual
camera solely because configuration succeeded.

The unit tests cover defaults, configuration validation, acknowledged lifecycle
transitions, repeated start/stop rejection, and reason preservation.
