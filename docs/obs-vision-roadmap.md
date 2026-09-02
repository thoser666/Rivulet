# OBS feature candidates aligned with the Rivulet vision

This is the curated companion to the automated
[`obs-vision-candidates.md`](obs-vision-candidates.md) review queue. Features
are added here only after maintainers confirm that they support Rivulet's own
vision rather than merely copying OBS.

## Approved roadmap candidates

| Feature | Vision fit | Planned milestone | Definition of done |
| --- | --- | --- | --- |
| WebRTC/WHIP first-class publishing | Low latency, modern transport, streamer value | M3 | H.264/Opus publish via SDP offer/answer to a WHIP endpoint, ICE/reconnect lifecycle, token-safe diagnostics, and local-SFU plus platform smoke evidence |
| SRT/RIST contribution | Reliable low-latency transport and professional workflows | M3 | Configuration contract validated (endpoint, latency, passphrase); negotiated media pipeline, reconnect/error states, and interoperability tests remain open |
| Multistreaming | Direct streamer value and automation | M3 | Multiple independent targets with per-target health, retry/isolation, secret redaction, and deterministic start/stop tests |
| NDI output | Cross-platform production interoperability | M3 | `NdiOutput` config contract done (validated name/group, quote-escaped GStreamer fragment, `ndisink` probe, off-by-default). Remaining: real NewTek NDI runtime/LAN interoperability evidence |
| VOD audio track | Streamer value and user control | M3 | `VodTrack` config model done (deterministic `enabled`/`recorded`, `ivod` flag, off-by-default, leakage-safe unit tests). Remaining: explicit routing UI and actual per-track GStreamer routing into the muxed output |
| Virtual camera | Reusable output and ecosystem interoperability | M4 | Platform-neutral contract with validated format/resolution/FPS and explicit lifecycle states; platform driver, permission handling, and consumer smoke test remain open |
| Audio ducking | Better live production workflow | M4 | Sidechain policy with bounded attenuation/attack/release, bypass/reset, tests, and clear mixer feedback |
| Remux and crash-safe file workflow | Reliability and deterministic output handling | M4 | MKV/MOV-to-MP4 remux with progress, cancellation, overwrite policy, recovery tests, and validated output files |
| Global hotkeys and remapping | Automation and accessible control | M5 | Conflict-aware rebinding, global scope, reserved-key handling, persistence, and platform-specific permission feedback |
| obs-websocket compatibility | Automation, embeddability, and ecosystem migration | M5 | **Done** — versioned v5 (JSON) compatible subset (scenes, sources, recording/streaming control), optional SHA-256 auth, deterministic request responses, event subscriptions, and end-to-end tests against a real WebSocket client (see `docs/obs-websocket.md`, #72) |
| Optional Discord Rich Presence adapter | Streamer-oriented status visibility without coupling the engine to a platform SDK | M5 | Non-blocking adapter consumes `PresenceStatus`, has an explicit opt-out, never sends secrets or local identifiers, handles Discord unavailability gracefully, and includes lifecycle/privacy tests |
| Chat dock (Twitch IRC) | Core streamer interaction: reading the live chat while streaming | M5 | Native chat view in the GUI connected to Twitch IRC (anonymous read or OAuth), parsing `PRIVMSG`/tags/actions/notice, PING/PONG keepalive, reconnect with backoff, message list with user color, i18n DE/EN, and tests against a local IRC listener |
| Alerts (follows/subs/donations) | Streamer engagement: surface events without coupling to a platform SDK | M5 | Event ingestion (Twitch EventSub or provider webhooks) mapped to localized alert entries, rendered as overlay entries or surfaced in the chat dock, with redaction of tokens and privacy tests |

## Decision rules

- The table is a roadmap commitment, not an implementation claim.
- Existing issues remain the execution source of truth; create or link an issue
  before marking a row complete.
- A candidate must have a named milestone, testable acceptance criteria, and a
  privacy/security review before promotion from the automated queue.
- Features that only reproduce OBS internals without improving Rivulet's
  determinism, embeddability, modern rendering, parity, privacy, or streamer
  workflow are intentionally excluded.

## Relationship to automated monitoring

The weekly OBS workflow classifies release-note candidates using
[`scripts/vision-criteria.json`](../scripts/vision-criteria.json). It updates
the automated queue, while this document is updated during maintainer review.
This separation prevents a noisy release note from silently changing the public
roadmap.
