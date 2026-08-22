# Game Capture Strategy — Decision Record (G1)

**Status:** G2 (DXGI Desktop Duplication), G3 (Vulkan layer), and G4 (OpenGL hook) implemented; G5/G6 open — G5 benchmarks the "within the overhead budget" clause of the G3/G4 DoDs
**Date:** 2026-08-20
**Tracked in:** [Issue #54](https://github.com/thoser666/Rivulet/issues/54) (M2 roadmap, *Priority for gaming streamers:*)
**Scope:** zero-overhead fullscreen game capture (D3D/Vulkan/OpenGL), fallback strategy, overhead budget, abort criteria

This document is the G1 deliverable. It recommends a capture approach per
graphics API, defines the overhead budget and measurement methodology, and
states the abort/fallback criteria that G2–G5 implementations must honour.

---

## 1. Goal and constraints

The goal is a **zero-overhead fullscreen game capture**: capturing exactly what
the game renders, without visible frame-time impact, so that Rivulet can
replace OBS for gaming streamers.

Constraints:

- **Overhead budget:** <1% of frame time (see §5 for definition and measurement).
- **Zero-copy:** the captured frame must reach the encoder as a GPU texture;
  system-memory readback is forbidden in the hot path.
- **Fallback chains:** every capture approach must degrade gracefully — when a
  path is unavailable or blocked, capture continues via the next best path.
- **Anti-cheat awareness:** game-process injection can be blocked by EAC,
  BattlEye, Vanguard etc. A blocked path must never silently produce no frames;
  it must fall back and surface a status to the GUI.
- **Scene-source integration:** game capture must be usable as an M2 scene
  source (same transform/properties pipeline as every other source).

---

## 2. Capture landscape in Rivulet today

| Path | Platform | Mechanism | Status |
| --- | --- | --- | --- |
| Window-based game capture | Windows | Windows Graphics Capture (via `windows-capture` in the GUI) | Done (M2) |
| Window-based game capture | Linux | `xcap` per-window capture (X11) | Done (M2) |
| Window enumeration | Linux | `xdotool` | Done (M2, CI-verified) |
| Fullscreen zero-overhead hook | all | — | **this G-series** |

G2–G6 add the missing zero-overhead fullscreen path. The existing
window-based capture stays as the windowed fallback.

---

## 3. Candidate approaches (Windows)

| Approach | How it works | Overhead | Capture scope | Risks |
| --- | --- | --- | --- | --- |
| **DXGI Desktop Duplication** (`IDXGIOutputDuplication`) | Duplicates the desktop backbuffer per output; zero-copy texture hand-off | Low–moderate (~1–3% GPU), no CPU copy | Fullscreen **and** windowed D3D9/10/11/12, GDI | No protected content; exclusive-fullscreen timing quirks; cursor handled separately |
| **Windows Graphics Capture (WGC)** | Captures via the DWM compositor, per window or monitor | Moderate (composition path) | Windowed/borderless mainly | Slightly higher overhead than hooks; needs permission flow |
| **In-process D3D hook** (OBS `graphics-hook`) | DLL injected into the game, hooks `IDXGISwapChain::Present`/`Present1` | Lowest (captures the real backbuffer in-process) | D3D9/11/12 fullscreen | **Injection** — anti-cheat friction; DLL must be signed/updated |
| **Vulkan implicit layer** (`VK_LAYER_OBS_HOOK` style) | Layer injected via `VK_INSTANCE_LAYERS`, hooks `vkQueuePresentKHR`, grabs the swapchain image | Lowest (in-process) | Vulkan fullscreen | Layer must be installed; some anti-cheats block layers |
| **OpenGL hook** (`wglSwapBuffers`) | Hooks `wglSwapBuffers`/`wglSwapLayerBuffers`, grabs the default framebuffer | Lowest (in-process) | OpenGL fullscreen | **Injection**; legacy API, less common in modern titles |

### Linux

Linux has **no in-process hook equivalent** — the compositor is authoritative:

| Approach | How it works | Overhead | Risks |
| --- | --- | --- | --- |
| **PipeWire / xdg-desktop-portal** | Screencast via the compositor (Wayland native, X11 via portal) | Moderate, compositor-bound | Per-compositor behaviour; needs portal permission |
| **xcap composite (X11)** | `XComposite` redirect + capture of the window pixmap | Moderate | Already implemented for windowed capture |

On Linux, "zero-overhead" is effectively the compositor path (G6); there is no
game-process injection comparable to Windows.

---

## 4. Recommendation per graphics API

| Scenario | Recommended primary | Fallback |
| --- | --- | --- |
| D3D9/11/12 fullscreen (G2) | **DXGI Desktop Duplication** (baseline; broad coverage, no injection) | WGC (windowed) |
| D3D11/12 — later phase | In-process swapchain hook (zero-overhead, behind a toggle) | Desktop Duplication |
| Vulkan fullscreen (G3) | **Implicit Vulkan layer** (swapchain image, `vkQueuePresentKHR`) | Desktop Duplication |
| OpenGL fullscreen (G4) | **wglSwapBuffers hook** | Desktop Duplication |
| Any windowed game | WGC (existing) | — |
| Linux Wayland (G6) | **PipeWire portal** | xcap composite (X11) |

**Rationale:** Desktop Duplication is the recommended G2 starting point because
it covers every D3D version and fullscreen mode **without injection** — the
broadest win with the least anti-cheat risk. The in-process hooks (D3D/Vulkan/
OpenGL) are progressive enhancements for the last few percent of overhead,
each gated by a mode toggle with automatic fallback to Desktop Duplication when
injection is blocked or fails.

---

## 5. Overhead budget and measurement

**Budget:** <1% of frame time, measured as the **p99 frame-time delta** with and
without capture on the reference machine:

| Refresh | Budget (p99 delta) |
| --- | --- |
| 60 Hz | < 0.17 ms |
| 120 Hz | < 0.08 ms |
| 144 Hz | < 0.07 ms |

**Methodology (G5 deliverable):**

- Same synthetic scene, three refresh rates (60/120/144 Hz).
- Metrics: FPS p50/p99, frame-time p50/p99, GPU busy time, encoder load.
- A/B: capture off vs. on, per backend (Desktop Duplication / Vulkan layer /
  OpenGL hook).
- **Reference baseline:** the same measurement against OBS on identical
  hardware, so "overhead" is relative to the tool we aim to replace.
- Zero-copy verified by asserting the hot path never performs a CPU readback.
- CI regression: `scripts/` benchmark on the Linux reference runner with a
  hard threshold; fails the job when the budget is exceeded.

---

## 6. Abort criteria and fallback rules

A capture approach is **aborted** (and the next fallback engaged) when:

1. **Injection blocked** — hook/layer DLL fails to load or is rejected by
   anti-cheat. → automatic fallback to Desktop Duplication (or WGC).
2. **Overhead exceeded** — measured p99 delta > budget on the reference
   machine. → fallback to the next approach; record the result in the G5
   report.
3. **No frames** — capture thread produces no frames within the no-frame
   timeout (same mechanism as the recording pipeline). → surface an error in
   the GUI, stop with a clear message, never silently continue.
4. **Protected content** (DRM) — not capturable by any path. → explicit "not
   capturable" status instead of a black frame.
5. **API unavailable** — e.g. WGC/Desktop Duplication absent on the target. →
   clear "unsupported" status (Rivulet targets Windows 10+; treat as
   theoretical).

A capture run must always report which backend is active and whether a fallback
occurred, so the GUI can inform the user (mirrors the
`skipped_filters`/no-frame UX already built for audio and recording).

---

## 7. Phased plan

| Phase | Work package | Deliverable | DoD | Status |
| --- | --- | --- | --- | --- |
| 1 | **G2 – DXGI backend** | Desktop Duplication capture, zero-copy GPU path to encoder, scene-source integration | DX9/11/12 fullscreen captured via zero-copy, tests + docs | ✅ Done |
| 2 | **G3 – Vulkan hook** | Implicit layer, swapchain image capture | Vulkan fullscreen within budget, tests + docs | ✅ Done (implementation); budget number pending G5 |
| 2 | **G4 – OpenGL hook** | wglSwapBuffers interception (IAT patch + GDI readback + SHM) | OpenGL fullscreen within budget, tests + docs | ✅ Implementation complete |
| 3 | **G5 – Performance verification** | Benchmark harness + CI regression | `scripts/` benchmark, budget verified for all backends | Open |
| 4 | **G6 – Linux fullscreen** | PipeWire portal (Wayland), X11 fallback | Wayland + X11 capture, tests + docs | Open |

G2 is the recommended first step: it delivers fullscreen game capture for the
majority of titles with no injection risk and unblocks the scene-source
integration. G3/G4 follow independently. G5 remains the gate for the
"within the overhead budget" clause of the G3/G4 DoDs: those hooks are checked
as *implementation-complete* with tests + docs, but the measured overhead
number is only final once G5 runs.

---

## 8. Open questions for the team

- **G2 priority:** Desktop Duplication first (broad, no injection) vs. the D3D
  swapchain hook first (best overhead, injection risk)? *Recommendation:
  Desktop Duplication first.*
- **Hook mode UX:** one "Game capture (hook)" toggle with auto-fallback, or
  explicit per-API selection? *Recommendation: toggle + auto-fallback, with the
  active backend shown in the GUI.*
- **Anti-cheat stance:** should the Vulkan/OpenGL/D3D hooks be shipped in
  stable builds at all, or restricted to an opt-in "advanced capture" setting?
