# Resource efficiency goal: game-first performance

Rivulet should leave as much CPU, GPU, memory, and battery capacity as possible to
the game. Capture and streaming quality must be achieved without turning the
recorder into the dominant workload, especially on older hardware and laptops.

## Product goal

> A streamer should be able to record and stream without visible game
> stutter, avoidable frame-time spikes, or unexplained resource growth.

This is a cross-cutting goal for game capture (G2–G6), streaming (M3), advanced
output (M4), and the modern renderer (M9). It is not a promise that every
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
5. Attach the redacted JSON report to the milestone quality report. The
   `scripts/resource-metrics.py` harness emits the same shape, so CI runs and
   hardware runs are directly comparable.
6. Investigate any regression before changing thresholds; do not hide it by
   averaging it away.

CI runs two layers, and they answer different questions:

- `scripts/resource-efficiency-check.py` validates the report **schema** and the
  absolute targets any machine can be held to (idle CPU, memory growth, RSS
  slope, positive startup and frame time, a reason on every `BLOCKED`/`N/A`
  record). It keeps accepting reports that only carry the original four fields,
  so the M6 harness and the deterministic fixture stay valid.
- `scripts/resource-metrics.py` **measures the real application** and compares
  it with a committed baseline per profile. It is the layer that fails when a
  change makes Rivulet worse.

## What CI measures, and what stays manual

The harness drives the shipped binary (`rivulet metrics`, issue #363) through a
defined scenario and samples the process from the outside — a process that
reports its own footprint can be wrong about it. The phases are opt-in and each
one is bracketed by `phase_start`/`phase_end` markers on stdout, so a sample is
attributed to the window it was taken in:

| Phase | Flag | What it measures |
| --- | --- | --- |
| startup | always | wall-clock from spawn to engine ready, plus the GStreamer bootstrap the engine reports itself |
| idle | `--idle-seconds N` | process CPU, RSS slope, RSS growth, peak RSS and thread count, with the first 2 s dropped as settle time |
| preview | `--config <scene.toml> --frames N` | p50/p95/p99 frame time and the 1 % low through `rivulet render`'s code path (scene model + snapshot + compositor) on the committed 720p workload (`scripts/ci-resource-metrics/preview.toml`) |
| record | `--record-seconds N` | the capture-on half of the A/B: synthetic RGBA frames pushed through a real recording pipeline |

Frame percentiles are **nearest-rank** over the frames that were actually
rendered: p50, p95 and p99 each name one measured frame (the `ceil(p · n)`-th
smallest) rather than an interpolation between two, so no reported number is a
frame time the run never produced. With an even frame count p50 is therefore
the lower of the two middle frames, and the 1 % low next to it stays the mean of
the slowest 1 % of frames.

Automated on every CI run (`--profile ci-linux`): startup, engine bootstrap,
idle CPU, RSS slope and growth, peak RSS, thread count, headless frame time, and
the synthetic capture-on/off CPU delta. The job measures a **release** build:
a debug binary's render path is orders of magnitude slower, so a baseline taken
from one would be the wrong reference for every later run (the harness refuses
`--write-baseline` from a debug binary unless explicitly allowed).

Trusting the sampler is the whole point, so the report has to justify it. An
idle process genuinely reads ~0 % CPU and a flat RSS, which means a sampler that
silently stopped reading looks exactly like a healthy quiet run — every delta is
zero, every gate passes. Every report therefore carries a `sampler` object
(backend, sample count, CPU seconds, RSS range) and the run aborts when that
evidence is impossible: zero resident memory, no CPU time at all across a run
that initialised the engine and rendered frames, or frames rendered with no
sampled CPU during the preview phase. The schema gate rejects a measured report
without the object, and `--binary` must be the executable itself — a launcher's
child is a different pid, so measuring a wrapper would report a shell that looks
idle while the renderer works.

Deliberately **manual**, because a shared runner cannot produce the number:
GPU utilisation, the game-side p99 frame-time delta, real capture-backend
overhead, encoder-specific behaviour, and battery/thermal behaviour on laptop
hardware. Those are recorded as `PASS`, `BLOCKED`, or `N/A` with the reason —
never inferred, never fabricated. The report carries `gpu` and `power` objects
whose status is `n/a`/`blocked` with a reason on CI, and `--power` collects the
OS telemetry (`/sys/class/power_supply`, `Win32_Battery`, `pmset`) where the
machine has it.

## Baselines and thresholds

A number only says something relative to a recorded one, so each profile has a
committed baseline under `scripts/resource-metrics-baseline/`. A run fails when
a metric exceeds **both** its relative and its absolute allowance:

| Metric | Relative allowance | Absolute target |
| --- | --- | --- |
| Idle CPU | baseline + 2 % | 2 % |
| RSS slope | baseline + 4 MiB/min | 4 MiB/min |
| Memory growth | — | 64 MiB |
| Startup | 3× baseline | +1500 ms |
| p99 frame time | 2× baseline | +20 ms |

The absolute target caps the relative allowance: a baseline recorded near the
target cannot buy a larger budget. Updating a baseline is a deliberate commit
(`--write-baseline`); CI never writes it, and the harness refuses to write one
from a debug build, because a baseline is the reference every later run is
compared against. A profile without a measured baseline
does not gate — the run records its numbers, says `seed` in the report, and the
next deliberate run commits the baseline. That is how the first measurement of
a new runner gets recorded without turning its noise into a red build.

The startup floor is measured rather than guessed. The same workload on the same
runner image reported 411 ms and then 1144 ms on consecutive runs — a 2.8×
spread with no code change. A +500 ms floor left 8 % of headroom and would have
failed a healthy build, which is how a gate teaches people to ignore it. Below
~1.5 s a startup difference on a shared runner is scheduling, not code; a
synchronous plugin scan or a network wait still trips the 3× factor.

`ci-linux` is measured (2026-11-04, `ubuntu-latest`, release build, seed state
replaced from the job's own artifact). Its idle CPU, RSS slope and memory growth
are honestly `0.0`: an idle recorder does nothing, so the absolute targets are
the bound that matters there, and the `sampler` evidence in each run is what
keeps that zero honest. The gating numbers on this runner are startup
(≈0.4 s) and headless p99 frame time (≈84 ms for the 720p workload) — both are
compared with their own baseline, so a shared-runner outlier within the budget
is noise rather than a red build, while a real doubling of the render cost fails
the run and names itself.

A regression names the metric, the baseline and the delta, and the report is
uploaded as a CI artifact so the trend stays visible: a snapshot with no history
would let a slow drift pass one run at a time.

## Ownership and roadmap

- G5 owns benchmark definitions and regression reporting.
- G2–G6 own backend-specific overhead and fallback evidence.
- M3 owns streaming queue isolation and adaptive-quality behavior.
- M9 owns zero-copy, WebGPU, and compute-path improvements.

The goal is reviewed at every M0–M11 milestone quality gate. Each milestone report must include the applicable resource evidence or an explicit `BLOCKED`/`N/A` reason. A release must not claim
"low overhead" solely because compilation or synthetic unit tests pass.
