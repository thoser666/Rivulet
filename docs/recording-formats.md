# Recording formats & crash-safe remux (M4)

This documents the M4 container-format and remux work: additional recording
containers beyond MP4 and lossless post-stop remux to MP4 (the OBS workflow).

## Why crash-safe containers matter

MP4 stores its index (the `moov` box) **at the end of the file**. If the app
crashes mid-write, the file is unreadable and the whole recording is lost.

MKV, MOV, and MPEG-TS stream their index (or tolerate truncation), so an
interrupted write stays playable up to the last complete packet. OBS therefore
records to MKV by default and *remuxes* to MP4 after stopping.

## Containers

`RecordingContainer` (`rivulet-core`, `container.rs`) models the four formats:

| Container | GStreamer muxer | Extension | Crash-safe |
|-----------|-----------------|-----------|------------|
| MP4       | `mp4mux`        | `mp4`     | No         |
| MKV       | `matroskamux`   | `mkv`     | Yes        |
| MOV       | `qtmux`         | `mov`     | Yes        |
| TS        | `mpegtsmux`     | `ts`      | Yes        |

The MP4 default preserves the codec-native muxer — H.264/H.265 keep `mp4mux`
and VP9 keeps `webmmux` — so existing behavior is unchanged unless a user
explicitly chooses a crash-safe intermediate. The GUI exposes a container
picker next to the video-codec picker.

## Remux (issue #71)

`RemuxPlan` validates that a source container is a crash-safe intermediate and
builds the lossless remux pipeline fragment (demux -> identity copy -> mux,
**never re-encodes**):

```
filesrc location=in.mkv ! matroskademux name=demux demux. ! queue ! mp4mux name=mux mux. ! filesink location=out.mp4
```

`RemuxSettings` carries `auto_remux_after_stop` (default on, mirroring OBS) and
the target container (MP4). The source extension is swapped for the output.

## Remux execution (issue #71)

`remux_to_mp4` actually runs the remux pipeline after recording stops when
`auto_remux_after_stop` is enabled and the source is a crash-safe intermediate:

- Builds a `filesrc -> demuxer -> mp4mux -> filesink` pipeline from the plan
  fragment using GStreamer's any-pad (`demux.` / `mux.`) syntax, so every
  encoded track is identity-copied into the MP4 container.
- Waits for EOS/error (bounded 60s timeout), then returns a `RemuxOutcome`:
  `Success { output_path }`, `Skipped(reason)` when a GStreamer element is
  unavailable, or an error.
- The engine runs this automatically when a session is finalized: after
  `stop_recording`, and on the background teardown thread after
  `stop_recording_background`. Auto-remux is enabled by default; the GUI
  exposes the toggle alongside the container picker.

## Stop is non-blocking

The stop sequence — pushing EOS, waiting (up to 10 s) for the muxer to
finalize the file, setting the pipeline to Null, then the optional auto-remux
and cloud upload — can take a while for long recordings. `stop_recording`
runs it synchronously on the caller's thread (used by the engine tests and
callers that need the file finalized when the call returns). The GUI stop
handlers call `stop_recording_background`, which resets the engine state
immediately (a new recording/stream can start at once) and runs the whole
teardown/finalize sequence on a background thread, so clicking "Stop" never
freezes the UI.

## Status

- [x] Container model + validation + GUI picker
- [x] Remux plan/settings + pipeline fragment
- [x] GStreamer remux **execution** after stop (auto, configurable in GUI)
- [ ] Recording file management (split by time/size) — separate M4 follow-up