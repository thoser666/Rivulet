# Resource efficiency goal: game-first performance

Rivulet should leave as much CPU, GPU, memory, and battery capacity as possible to
the game. Capture and streaming quality must be achieved without turning the
recorder into the dominant workload, especially on older hardware and laptops.

## Product goal

> A streamer should be able to record and stream without visible game
> stutter, avoidable frame-time spikes, or unexplained resource growth.

This is a cross-cutting goal for game capture (G2–G6), streaming (M3), advanced
output (M4), and the modern renderer (M8). It is not a promise that every
hardware combination reaches the same quality; unsupported hardware must instead
fall back clearly and remain responsive.

## Initial measurable targets

Targets are evaluated on a documented reference machine and a representative
older-hardware profile. They are guardrails, not claims about every GPU:

| Metric | Target | Measurement |
| --- | --- | --- |
| Capture overhead | < 1% p99 frame-time delta where the backend supports the G5 budget | Capture off/on A/B |
| Capture frame-time spike | No new sustained p99 regression over the approved baseline | 60/120/144 Hz frame-time samples |
| Idle CPU overhead | ≤ 2% additional process CPU after startup settles | 60-second idle sample |
| Memory stability | No unbounded growth during a 30-minute session | RSS samples and slope |
| Battery/thermal behavior | Reported, not inferred; no thermal-throttling regression on laptop profile | OS power/thermal telemetry when available |
| Quality fallback | Prefer hardware/zero-copy paths; expose fallback reason when unavailable | Backend and encoder diagnostics |

The exact baseline, hardware, driver, OS, resolution, refresh rate, encoder,
scene, and game build are stored with every benchmark report. Averages alone are
insufficient: p95/p99 frame time and 1% lows matter more for games.

## Engineering rules

- Prefer GPU-direct and zero-copy paths in the hot path.
- Avoid synchronous CPU readbacks, allocations, and locks per frame.
- Keep capture, encoding, and network queues bounded.
- Never let a stalled streaming target block the game/capture path.
- Reduce quality or bitrate conservatively before increasing latency or dropping
  the game below its frame-time budget.
- Run diagnostics off the UI and capture threads where possible.
- Show active backend, encoder, fallback, and resource impact in diagnostics.

## Verification workflow

1. Run the G5 benchmark with capture disabled.
2. Repeat with each capture backend and the intended encoder.
3. Compare p50, p95, and p99 frame time, 1% lows, CPU, GPU, memory, and power.
4. Repeat on the older-hardware profile.
5. Attach the redacted JSON report to the milestone quality report.
6. Investigate any regression before changing thresholds; do not hide it by
   averaging it away.

CI can validate report schema, budgets, and deterministic policy tests. Real GPU,
power, and game measurements are hardware evidence and must be marked
`PASS`, `BLOCKED`, or `N/A` with the reason.

## Ownership and roadmap

- G5 owns benchmark definitions and regression reporting.
- G2–G6 own backend-specific overhead and fallback evidence.
- M3 owns streaming queue isolation and adaptive-quality behavior.
- M8 owns zero-copy, WebGPU, and compute-path improvements.

The goal is reviewed at every M0–M9 milestone quality gate. Each milestone report must include the applicable resource evidence or an explicit `BLOCKED`/`N/A` reason. A release must not claim
"low overhead" solely because compilation or synthetic unit tests pass.
