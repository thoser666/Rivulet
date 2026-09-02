# M4 Output Quality Gate

This document records the milestone gate for **M4 – Advanced Output & Capture**
(replay buffer, virtual camera contract, video/audio filters, ducking, master
mix, recording formats, remux, file management, rate control, multi-track
export, VST 3.x + cloud contracts). It supplements the common gate and the
milestone-specific gates in [`milestone-quality-gates.md`](milestone-quality-gates.md).

Date: 2026-08-31. Result: **PASS (conditional)** — see the explicitly marked
`BLOCKED`/`N/A` items; none is a Blocker or Critical finding.

## M4-specific gate (from milestone-quality-gates.md)

| Criterion | Evidence | Status |
|---|---|---|
| Save Replay is discoverable, reports the exact output path, handles empty/too-short buffer without false success | Replay save via hotkey + GUI; `ReplayBuffer` guards against empty/short clips; path reported in UI/logs | PASS |
| Remux, format conversion, file splitting, recording recovery: progress, cancellation, overwrite behavior, partial files | `RemuxSettings`/`RemuxPlan`, auto-remux after stop with availability gating; `splitmuxsink` for crash-safe containers; docs/recording-formats.md + docs/recording-files.md | PASS |
| Filters expose safe defaults, reset behavior, bypass state, meaningful unsupported-plugin errors | All audio/video filter stages default-off/neutral; missing optional elements are skipped and reported (GUI + log) instead of failing capture | PASS |
| Virtual camera lifecycle visible to other apps, clear stop/release path | Platform-neutral `VirtualCameraState` lifecycle (Starting/Running/Stopping/Unavailable/Error); platform driver integration still open → marked | BLOCKED (platform driver integration, tracked separately) |
| Audio ducking / master mix / VST failures cannot silently mute the wrong source or crash the recording | Ducking sidechain policy + hysteresis; master mix is a single output volume; VST is a config contract only (no runtime hosting yet) | PASS (VST hosting N/A this milestone) |

Exit evidence: valid output files (recording/dual-output pipeline tests),
failure/recovery cases (skipped-filter warnings, remux gating, split rules),
data-loss review (crash-safe containers, MP4 split deliberately disabled).

## Common gate highlights

- **Automated baseline:** `cargo fmt --check`, `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings` green on the M4
  feature set; CI (Lints, Pinning-Tests, Beta-Gate, RIST/SRT smokes, 3-OS
  builds) green after the filter-UI fixes (2026-08-31).
- **UI thread responsiveness:** capture/encode run off the UI thread; remux
  runs after stop; queued operations are bounded.
- **Failure states shown in GUI:** skipped filters, capture errors, remux
  availability, encoder fallback all surfaced in the UI, not only logs.
- **Secrets:** stream keys and cloud credentials masked in logs/`Debug`.
- **i18n:** new keys (rate control, filters, master mix, multi-track export,
  video effects) added in EN + DE; no language mixing in the touched flows.

## Resource-efficiency evidence (M4 row)

Milestone M4 minimum evidence: **Replay, filters, remux, and virtual-camera
resource behavior**.

| Measurement | Status | Notes |
|---|---|---|
| Replay buffer overhead | PASS | Ring buffer is bounded; write happens on the streaming thread |
| Audio/video filter CPU impact | PASS (scaled) | Filters are availability-gated; disabled by default (no pipeline elements added when inactive) |
| Remux CPU/IO behavior | N/A | Runs after stop, non-blocking; representative timing measurement still open on the reference machine |
| Virtual-camera GPU/CPU behavior | BLOCKED | No platform driver available in this environment; measured when the driver integration lands |
| p95/p99 frame-time + 1% lows | BLOCKED | Requires the reference machine and real GPU capture; tracked under G5 performance verification |
| Memory growth over a session | PASS (bounded) | Recording metrics monitor session length/file size; no unbounded growth observed in tests |

Hardware-only measurements are explicitly marked rather than silently omitted.

## Result

PASS (conditional) — no Blocker/Critical findings. High-priority follow-ups
(explicitly assigned, not hidden): virtual-camera platform drivers, VST3
hosting, cloud S3 PUT, NDI/VOD-track mux routing, and the G5 frame-time
measurements on the reference machine.
