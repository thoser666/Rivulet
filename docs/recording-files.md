# Recording file management (M4)

Documents the M4 "Recording file management" point: filename patterns, split
by time/size, and auto-record alongside the stream.

The policy model lives on `rivulet-core` (`file_management.rs`) with GUI
wiring for the settings. Splitting is realized in the live recording pipeline
via GStreamer's `splitmuxsink`, which rolls over to a new `%02d`-numbered file
automatically when a time or size boundary is crossed — no re-encoding.

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

### Unvalidated patterns (settings files)

`FileNamePattern` derives `Deserialize` and deliberately does **not** re-run
that construction-time validation: a settings file with a broken pattern must
still load so the user can repair it in the UI instead of losing every other
setting. `render` is therefore *total* — it never panics and always returns a
single, sandboxed filename component:

| Input (e.g. from a hand-edited settings file) | Rendered |
|-----------------------------------------------|----------|
| `rec{unclosed` | `recunclosed` |
| `{bogus}` | `bogus` |
| `../../pwned` | `___pwned` |
| `/etc/passwd` | `_etc_passwd` |
| `recording.v2_{name}` | `recording.v2_scene` |

Literal text (and the `{date}`/`{time}` values) goes through
`sanitize_literal`, which keeps alphanumerics, `-`, `_`, and a **single** `.`
so deliberate dots survive, while collapsing a run of two or more dots — that
is what stops a `../../` pattern in a settings file from writing outside the
recording directory. A placeholder that cannot be expanded renders as its own
text (without the braces) instead of aborting the recording.

`RivuletEngine::default_recording_path(dir, stream)` applies the configured
pattern to the current timestamp and container extension — the GUI's file
dialogs use it instead of the hard-coded `rivulet-recording-<timestamp>` name.

## Split rules

`SplitBy` selects no split, split by duration, or split by file size.
`RecordingSession` tracks the current part, bytes written and seconds elapsed
per part, and exposes `should_split`/`next_part`.

The engine maps `SplitBy` onto the live recording pipeline
(`recording_muxer_fragment`):

| `SplitBy`             | Pipeline effect                          |
|-----------------------|------------------------------------------|
| `Duration { seconds }`| `splitmuxsink name=mux ... max-size-time=<nanos>` |
| `Size { megabytes }`  | `splitmuxsink name=mux ... max-size-bytes=<bytes>` |
| `None`                | plain muxer + `filesink`                 |

Splitting is only enabled on **crash-safe** containers (MKV/MOV/TS); an
interrupted MP4 numbered part would be unreadable, so `SplitBy` is ignored
while the selected container is MP4. `splitmuxsink` produces files named
`<base>_01.ext`, `<base>_02.ext`, ... from the `%02d` in the location
(equivalent to the `{seq}` token of the filename pattern).

The GUI exposes "Split after (s)" (0 = off) and "Record automatically with
stream" toggles next to the container/remux controls, and pushes them into the
engine via `set_recording_file`.

## Auto-record alongside the stream

`RecordingFileSettings::auto_record_with_stream` toggles whether a recording
starts together with the stream. Combined with `filename_pattern`, stream-linked
recordings get distinct, pattern-named files.

## Status

- [x] Pattern model + validation + rendering
- [x] `render` hardened against patterns that skip construction validation
      (serde-loaded settings): no panic, no path escape, dots preserved
- [x] Split-by-time/size model + part sequencing
- [x] Auto-record flag + engine settings + GUI toggles
- [x] `default_recording_path` used by GUI dialogs
- [x] Live GStreamer re-split of the recording pipeline (`splitmuxsink` with
      `max-size-time`/`max-size-bytes` and `%02d` part numbering, crash-safe
      containers only)