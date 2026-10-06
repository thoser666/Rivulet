# Logging and crash diagnostics

Rivulet initializes file logging before the GUI starts. Logs are written as
`rivulet-YYYY-MM-DD.log` in the per-user application data directory, one file
per local calendar day. ANSI color sequences are disabled so files can be
attached directly to bug reports.

## Retention

The default retention is **14 days**. Configure it before launching Rivulet:

```text
RIVULET_LOG_RETENTION_DAYS=30
```

The value is clamped to at least one day. Files older than the retention window
are removed at startup; unrelated `.log` files are never touched.

## Crash markers

A crash or fatal startup failure should be recorded using the delimiters:

```text
===== RIVULET CRASH =====
context: <startup|recording|capture|update|logging>
error: <sanitized error text>
===== END RIVULET CRASH =====
```

Windows packaged builds can also contain early-startup markers:

```text
===== RIVULET PRE-RUST EVENT =====
context: launcher-start
message: starting rivulet-gui.exe
===== END RIVULET PRE-RUST EVENT =====
```

`PRE-RUST EVENT` confirms that the launcher ran. `PRE-RUST DIAGNOSTIC` blocks
indicate that the launcher could not start the GUI or that the GUI exited with
a non-zero status. These blocks cover failures before the GUI's tracing
subscriber is available. Do not include stream keys, passwords, or unredacted
personal paths in any marker.

## Pre-Rust crash capture on Windows

Windows release bundles include a tiny `rivulet.exe` launcher. It starts
`rivulet-gui.exe`, records launcher errors and non-zero exit codes using a
`RIVULET PRE-RUST DIAGNOSTIC` block, and therefore covers failures that happen
before the GUI process can initialize Rust logging. The launcher itself has no
GStreamer or GUI dependencies.

For native Windows crashes, set `RIVULET_ENABLE_CRASH_DUMPS=1` before starting
the launcher. The launcher then opts the current user into Windows Error
Reporting full dumps for `rivulet-gui.exe` and stores them in the user's
`%LOCALAPPDATA%\\Rivulet\\crash-dumps` directory. This is intentionally opt-in
because it changes the user's WER LocalDumps registry settings. Disable it by
removing the variable; existing dumps can be deleted manually after diagnosis.

This cannot catch failures before Windows can execute the launcher itself
(e.g. a corrupt launcher binary, blocked SmartScreen policy, or system-wide
loader failure). In those cases use Event Viewer → Windows Logs → Application
and the Windows Error Reporting entry.

## When the application starts but the log is empty

The launcher writes a pre-Rust event before starting the GUI, and the GUI logger
creates the daily file before GStreamer and egui are initialized. A zero-byte
file in a packaged Windows build therefore means the launcher itself did not
run or could not write to `%LOCALAPPDATA%\\Rivulet\\logs`; inspect Event Viewer
and the Windows Error Reporting entry in that case. Otherwise check:

1. Confirm the current local date in the filename (`rivulet-YYYY-MM-DD.log`).
2. Check the platform-specific directory below and verify that the process has
   write permission.

> **Log level:** the daily log defaults to `info` level, so startup, engine,
> and Discord Rich Presence diagnostics are captured out of the box. To
> increase detail (e.g. `debug`/`trace` for GStreamer or IPC internals), set
> `RUST_LOG` (e.g. `RUST_LOG=debug` or `RUST_LOG=rivulet=debug,info`) before
> starting Rivulet — an explicit value always overrides the default. Note
> that older builds (before this default existed) required `RUST_LOG=info`
> to write *any* Rust events at all; a log file containing only
> `RIVULET PRE-RUST EVENT` blocks indicates such a build.

> **Recording start is observable:** pressing Record opens the save dialog;
> if the dialog is cancelled, `File selection cancelled` is written to the
> daily log at `info` level (on all platforms: Windows, PipeWire portal and
> the Linux fallback). A recording that "never starts" after Record is
> therefore always diagnosable: either the dialog was cancelled (entry
> present) or the capture thread logged `Starting capture thread (...)` —
> with neither entry the Record button was never enabled (no source
> selected).

## Pipeline construction errors

When GStreamer rejects the recording/streaming pipeline, the GUI shows a short
error (`Could not create the recording pipeline: <message>`). The daily log
keeps the full diagnostic:

- the GStreamer error **domain and code** (e.g. a bare `syntax error` from
  `parse_launch` becomes `(GStreamer domain gst-parse-error, code 4)` so the
  exact parser failure is identifiable), and
- the **redacted pipeline description** under the `pipeline` log field.

Pipeline descriptions can embed the RTMP(S) ingest URL *including the stream
key* (`rtmp2sink location="rtmps://host/app/KEY"`). Such values are replaced
with `<redacted stream URL>` before logging, so stream keys never reach the
daily log; the rest of the pipeline stays intact for debugging. A pipeline that
refuses to parse is therefore diagnosable from the log alone without leaking
credentials.

> **A failed pipeline build is reported once, not once per frame.** The pipeline
> is constructed lazily from the first video frame. When construction fails
> (parse failure or the pipeline refusing to enter `Playing`), the engine
> **arms a retry guard** and ends the session: the error is surfaced exactly
> once and subsequent frames stop attempting to rebuild the same broken
> pipeline. Otherwise every frame of an active capture would re-run
> `parse_launch` against the identical string, flooding the daily log with one
> identical `Could not create the recording pipeline: ...` line per frame (a
> 21-line storm in under a second in real logs). The user starts a new session
> explicitly once the configuration is corrected — that explicit start clears
> the guard, so the next frame gets exactly one fresh attempt whose result is
> reported once.

4. If file logging cannot be initialized, startup continues and a
   `RIVULET CRASH` block is written to the intended path when possible; the
   bootstrap error is also sent to stderr.
5. If the file remains empty, collect the Windows Event Viewer entry or a
   debugger/minidump because the failure occurred before Rust tracing could run.

Default locations:

- Windows: `%LOCALAPPDATA%\\Rivulet\\logs\\`
- Linux/macOS: the local user data directory under `Rivulet/logs/`

## Crash reports from the Settings tab

**Settings → Crash report → *Copy crash report and open a prefilled issue***
builds the bug report for you: it reads today's daily log, extracts the
`RIVULET CRASH` blocks together with the ~50 log lines before each one, and
then

1. **redacts** the result — every `rtmp(s)`/`rtmpt`/`srt` URL (including an
   SRT passphrase) becomes `<redacted stream URL>`, and your home directory
   plus your user name become `<user>`,
2. **copies** the finished report to the clipboard,
3. **opens** a GitHub issue composer with the title and the body already
   filled in.

**Rivulet transmits nothing.** There is no token, no endpoint and no
background sender — what leaves the machine is only what you paste into the
issue yourself. That is deliberate: a crash log is exactly the data that must
not be uploaded uninvited, and an "automatically file a report" feature would
need a personal access token in the app.

Two details worth knowing:

- **Every redaction is counted and shown** in the report header (`stream URLs
  redacted: 1`, `home paths redacted: 3`). Silent cleaning teaches you that
  the tool already removed everything, so it does not.
- **An empty log is reported as empty.** If today's file contains no crash
  block, the report says so and asks you to describe the problem instead of
  shipping a log dump with no defect in it. A Windows startup failure writes
  `RIVULET PRE-RUST DIAGNOSTIC` instead of `RIVULET CRASH`; those are included
  in the excerpt.

The excerpt is bounded (3 crash blocks, 24 000 characters, oldest lines
dropped first) so a runaway log cannot produce a paste that no issue box
accepts. Paths are replaced segment-aware, so a neighbouring account such as
`/home/thouser2` is not mangled into `<user>2`.

## Crash reports from the menu bar

**Help → Crash reports → *Upload previous crash report*** is the OBS path, and
it exists because of one structural fact: the release profile is
`panic = "abort"`. A panic still runs the hook, but the process is gone the
instant the hook returns — there is no unwinding, no worker thread, and no
time for a network round trip. **An upload at the moment of the crash is
therefore impossible**, so the chain is deliberately split:

1. **The crash writes a record.** A panic hook installed as the first thing
   `main()` does writes `crashes/crash-<timestamp>.txt` — panic site, message,
   timestamp — synchronously, and keeps only the newest five so a crash loop
   cannot fill the disk. The records live outside `logs/` so the retention
   pass can never delete an unreported crash.
2. **The next launch offers it.** Rivulet reads that directory at startup. If
   a record is there, *Upload previous crash report* is enabled; otherwise it
   is greyed out and says that the last session was clean.
3. **You click it.** The record is appended to the daily log as a
   `RIVULET CRASH` block, so it travels through the same redaction as
   everything else, and is delivered like the report above.
4. **It is then forgotten.** The records are deleted once they have been
   handed over, so a crash you already dealt with is not offered again.

**What the build does not do.** There is **no** upload destination wired up
in the shipped build: no token, no endpoint, no background sender, and
nothing is ever filed automatically. The `CrashReportSink` trait is the seam a
reviewed transport would plug into later — modelled on the telemetry sink,
unit-tested, and installed by nobody. Until one exists, clicking the menu
item copies the redacted report to the clipboard and opens a prefilled issue,
and the status line says exactly that rather than implying an upload.

When a sink *is* wired, its URL is what you get, and it is **copied** rather
than opened, so you keep the window you are reading. A sink that fails falls
back to the manual path with a status that says the upload failed, instead of
leaving you with nothing.

Records carry only what the panic itself stated. A payload that is neither a
string nor a `Debug` primitive becomes the fixed marker
`non-string panic payload`, because formatting an arbitrary type would dump
its fields — application state — into a file that is meant to be shareable.

## Reporting a problem manually

The button is a shortcut, not a requirement — everything below works with a
plain text editor, and stays the fallback if you prefer not to open a
browser.

1. Note the Rivulet version and operating system.
2. Reproduce the issue once, if safe.
3. Copy the daily log covering the failure.
4. Include the complete crash block and the preceding ~50 lines.
5. On Windows, include the matching `.dmp` file when crash dumps were enabled.
6. Remove secrets, usernames, and private file paths before sharing.

Production diagnostics use `tracing` rather than direct `println!`/`eprintln!`
output, so levels, fields, and daily file routing remain consistent. The only
remaining direct stderr output is the deliberate bootstrap fallback when the
logging subscriber itself cannot be initialized, plus test-only dependency
messages.

The logging module has unit tests for date-based paths, retention filtering,
and crash-marker format. The dependency-free launcher has tests for its
pre-Rust marker format and date handling.
