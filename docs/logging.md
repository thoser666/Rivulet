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
context: <startup|recording|capture|update>
error: <sanitized error text>
===== END RIVULET CRASH =====
```

This makes crash blocks searchable and allows automated tooling to extract the
most recent failure without guessing from ordinary warnings. Do not include
stream keys, passwords, or unredacted personal paths in the marker.

## Reporting a problem

1. Note the Rivulet version and operating system.
2. Reproduce the issue once, if safe.
3. Copy the daily log covering the failure.
4. Include the complete crash block and the preceding ~50 lines.
5. Remove secrets, usernames, and private file paths before sharing.

The logging module has unit tests for date-based paths, retention filtering,
and crash-marker format. Future crash-report tooling can consume the stable
marker format without depending on console output.
