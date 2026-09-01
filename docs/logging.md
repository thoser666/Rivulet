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
4. If file logging cannot be initialized, startup continues and a
   `RIVULET CRASH` block is written to the intended path when possible; the
   bootstrap error is also sent to stderr.
5. If the file remains empty, collect the Windows Event Viewer entry or a
   debugger/minidump because the failure occurred before Rust tracing could run.

Default locations:

- Windows: `%LOCALAPPDATA%\\Rivulet\\logs\\`
- Linux/macOS: the local user data directory under `Rivulet/logs/`

## Reporting a problem

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
