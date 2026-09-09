# Opt-in usage telemetry

Rivulet ships a privacy-first, opt-in telemetry pipeline as part of **M5 –
Ecosystem & Platform Parity**. It is designed so that a future collector
cannot accidentally leak user data, and the shipped build transmits
**nothing**.

## The policy (enforced)

- **Off by default.** Telemetry is never captured until the user enables the
  persisted toggle in Settings → Telemetry. The runtime collector mirrors the
  toggle on startup (`apply_telemetry_policy` in `RivuletApp`) and on every
  toggle change.
- **Opt-out clears everything.** Disabling the toggle drops all pending events
  immediately. Nothing recorded while opted in survives an opt-out.
- **No free-form text, ever.** The event model (`TelemetryEvent` in
  `rivulet-core/src/telemetry.rs`) carries only enums, numeric codes and
  booleans. Window titles, paths, URLs, stream keys and usernames are not part
  of the model — a future sender cannot leak them even if it only serializes
  the batch as-is. A deterministic test pins the serialized JSON against
  free-form content (no spaces, slashes, at-signs or percent signs).
- **No transport in the shipped build.** The collector hands batches to an
  optional `TelemetrySink`; the shipping `RivuletApp` wires **no** sink.
  Nothing leaves the device. A future transport must be reviewed separately
  before it is allowed to install a sink.
- **Bounded memory.** Pending events flush automatically at 128
  (`DEFAULT_AUTO_FLUSH_AT`), so an enabled collector can never grow without
  bound.

## What is collected (when opted in)

| Event | Payload | Notes |
|---|---|---|
| `Startup` | — | Once per session, after restore while opted in |
| `RecordingStop` | `duration_secs: u32`, `healthy: bool` | Emitted by every platform stop path (Windows/Linux/macOS/aux); `healthy` is best-effort at stop time |
| `RecordingError` | `error: TelemetryErrorKind` | Defined for future error wiring |
| `SceneSwitch` | — | Defined for future scene-switch wiring |
| `ChatConnect` | `ok: bool` | Defined for future chat wiring |

Every batch additionally carries the compile-time app version
(`CARGO_PKG_VERSION`) and a `cfg!`-resolved platform code (`windows`,
`linux`, `macos`, `other`).

## Why a transport is intentionally missing

A telemetry endpoint is infrastructure: an HTTPS ingestion service, batching,
retries, backoff, rate limiting against the service, and privacy review of
what arrives on the wire. None of that exists yet — so instead of wiring a
half-baked sender, the M5 milestone ships the **client-side contract and
policy** (opt-in, redaction-safe model, bounded deterministic collector,
verified by tests) and leaves transmission as an explicitly documented
follow-up. This mirrors how other staged contracts ship (e.g. the NDI
"config contract done, runtime evidence remaining" pattern).

## Wiring

- **Core:** `rivulet-core/src/telemetry.rs` — `TelemetryEvent`,
  `TelemetryBatch`, `TelemetrySink`, `TelemetryReporter`, `platform_code()`.
  Deterministic unit tests cover opt-in behavior, auto-flush, clearing on
  opt-out, JSON round-trip and free-form-text redaction.
- **GUI:** `rivulet-gui/src/app.rs` — persisted `telemetry_enabled` toggle
  (Settings → Telemetry), runtime `TelemetryReporter` (`#[serde(skip)]`),
  `apply_telemetry_policy()` after restore, and
  `complete_recording_session_telemetry()` in every platform stop path.
- **Security:** the privacy posture is documented in
  [`docs/security.md`](security.md) and pinned by the ci_pinning guard
  `m5_telemetry_opt_in_is_privacy_safe_and_pinned`.

## Verification

```text
cargo test -p rivulet-core --lib telemetry
cargo test -p rivulet-gui --bin rivulet-gui telemetry
cargo test -p rivulet-core --test ci_pinning
```

The redaction test is the contract: if the serialized payload ever contains a
character class that free-form text needs, CI fails instead of shipping a
leak.