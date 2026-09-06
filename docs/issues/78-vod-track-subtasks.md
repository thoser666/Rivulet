## Offen (Follow-up)

- **Routing**: Das `VodTrack`-Modell ist fertig (leakage-sicher, `ivod`-Flag,
  off-by-default, getestet). Verbleibende Arbeit ist das **pro-Track-GStreamer-Routing
  in den gemuxten Output** und die **Routing-UI** — das ist M8 (Issue [#78](https://github.com/thoser666/Rivulet/issues/78)).

## Subtasks

### Z78-1 — [M3] VOD track: extend the GStreamer recording pipeline so an active VodTrack emits a third, separate audio branch into the recording mux

When `StreamSettings.vod_track.active()` is true while streaming, the local
recording pipeline must emit a **third, independent audio branch** alongside
`audio_src_sys` and `audio_src_mic` into the recording muxer. The branch must
use the existing per-track mux-leg pattern (named request pads), must not alter
the live-stream FLV audio path, and must be gated on the same
`separate_audio_tracks && !is_streaming()` condition already used for the System
/ Microphone split so the code path stays consistent.

Acceptance criteria:
- [ ] `build_pipeline_str()` for a dual-output recording session where
  `vod_track.active()` is true contains a third audio branch feeding the
  recording muxer (branch naming follows the existing `audio_src_*` convention).
- [ ] The new branch is present only when the VodTrack is active; it is absent
  when `vod_track` is off or when streaming-only.
- [ ] The branch uses a named mux leg the same way the System/Microphone tracks
  do; no any-pad mux linking for the VOD branch.
- [ ] Existing recording/dual-output tests continue to pass.

### Z78-2 — [M3] VOD track: add leakage + parity tests for the extra VOD audio branch in the recording pipeline

Add the regression + parity tests that cover the new VOD audio branch as a
**pipeline-string contract** (no plugin binary required).

Acceptance criteria:
- [ ] A test asserts that a dual-output recording pipeline with an active VodTrack
  contains the extra VOD audio branch and that the muxer fragment includes it.
- [ ] A test asserts the VOD branch is absent when `vod_track` is inactive/off
  (leakage safety for the config).
- [ ] A test asserts the VOD branch is absent in a streaming-only pipeline
  (FLV single-track invariant still holds).
- [ ] A test asserts the VOD track is never present in any masked/redacted output
  string (`masked_key()` / `location()` / other redacted surfaces).
- [ ] The new tests run green in the normal test job **and** in the coverage job
  (no GStreamer factoryholes introduced).

### Z78-3 — [M3] VOD track: wire recording-side VOD routing into engine config + tests

Move from “branch exists in the pipeline string” to “engine config knows how to
enable/disable the VOD branch and the existing config contract already covers it”.

Acceptance criteria:
- [ ] The engine honours `StreamSettings.vod_track` when building the recording
  pipeline (the branch appears/disappears based on config, not hardcoded).
- [ ] `VodTrack` remains leakage-safe: enabling VOD without `recorded == true`
  still emits nothing (existing `with_recorded(false)` behaviour stays intact).
- [ ] Any engine-side toggle for VOD recording (if introduced) keeps parity with
  the existing `set_separate_audio_tracks` / per-source audio enable pattern.
- [ ] The configuration does not change the masked/redacted outputs.

### Z78-4 — [M3] VOD track: document the supported recording behavior, gaps, and M8 follow-ups

Update docs so the VOD-track feature is accurately represented **as-is** (config
done, routing not yet wired) and the remaining work is visible.

Acceptance criteria:
- [ ] `docs/m3-streaming-completion-report.md` (and/or the M3 quality gate) keeps
  the F-M3-005 / VOD-track follow-up accurate: config contract complete, per-track
  GStreamer routing + routing UI remaining, target milestone M8.
- [ ] Any README M3 / M9 cross-references to VOD track stay correct (no silent
  “done” claim for routing).
- [ ] `docs/vst3.md`-style honesty for the VOD-track gap: what is supported now,
  what is not, and what the follow-up is — in German + English where the rest of
  the docs use that pattern.
