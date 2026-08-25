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

This makes crash blocks searchable and allows automated tooling to extract the
most recent failure without guessing from ordinary warnings. Do not include
stream keys, passwords, or unredacted personal paths in the marker.

## When the application starts but the log is empty

The logger creates the daily file before GStreamer and the GUI are initialized.
A zero-byte file therefore usually means the process exited before the first
tracing event, or that the file being inspected is not today's file. Check the
following:

1. Confirm the current local date in the filename (`rivulet-YYYY-MM-DD.log`).
2. Start Rivulet once with `RUST_LOG=info` to include informational startup
   events.
3. Check the platform-specific directory below and verify that the process has
   write permission.
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
5. Remove secrets, usernames, and private file paths before sharing.

Production diagnostics use `tracing` rather than direct `println!`/`eprintln!`
output, so levels, fields, and daily file routing remain consistent. The only
remaining direct stderr output is the deliberate bootstrap fallback when the
logging subscriber itself cannot be initialized, plus test-only dependency
messages.

The logging module has unit tests for date-based paths, retention filtering,
and crash-marker format. Future crash-report tooling can consume the stable
marker format without depending on console output.
