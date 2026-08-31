# Recording file management (M4)

Documents the M4 "Recording file management" point: filename patterns, split
by time/size, and auto-record alongside the stream.

This is a **policy model** on `rivulet-core` (`file_management.rs`) plus GUI
wiring for the settings. It is deterministic and fully unit-tested; live
mid-recording GStreamer re-split of an open pipeline is a separate integration
follow-up (like remux execution was).

## Filename patterns

`FileNamePattern` renders an OBS-style template with `{token}` placeholders:

| Token     | Meaning                              | Example |
|-----------|--------------------------------------|---------|
| `{name}`  | Base name (e.g. `rivulet-recording`) | `game`  |
| `{date}`  | ISO date                             | `2026-08-31` |
| `{time}`  | Time                                 | `14-05-09` |
| `{seq}`   | Zero-padded part number              | `01`    |
| `{stream}`| Platform/stream label                | `twitch`|

Unknown placeholders, unterminated braces, and path-hostile characters are
rejected at construction so a pattern can never break the output path. Free
text in `{name}`/`{stream}` is sanitized and doubled separators collapsed.

`RivuletEngine::default_recording_path(dir, stream)` applies the configured
pattern to the current timestamp and container extension — the GUI's file
dialogs use it instead of the hard-coded `rivulet-recording-<timestamp>` name.

## Split rules

`SplitBy` selects no split, split by duration, or split by file size.
`RecordingSession` tracks the current part, bytes written and seconds elapsed
per part, and exposes `should_split`/`next_part` so the engine can roll to the
next `{seq}` file when a boundary is crossed.

The GUI exposes "Split after (s)" (0 = off) and "Record automatically with
stream" toggles next to the container/remux controls, and pushes them into the
engine via `set_recording_file`.

## Auto-record alongside the stream

`RecordingFileSettings::auto_record_with_stream` toggles whether a recording
starts together with the stream. Combined with `filename_pattern`, stream-linked
recordings get distinct, pattern-named files.

## Status

- [x] Pattern model + validation + rendering
- [x] Split-by-time/size model + part sequencing
- [x] Auto-record flag + engine settings + GUI toggles
- [x] `default_recording_path` used by GUI dialogs
- [ ] Live GStreamer re-split of an open pipeline (integration follow-up)