# Activity status and Discord Rich Presence

Rivulet now exposes a platform-neutral activity-status contract for a Discord-style
status message. The GUI shows the current product activity using the **Rivulet**
name, for example:

- `Rivulet nimmt auf`
- `Rivulet streamt`
- `Rivulet nimmt auf und streamt`
- `Rivulet pausiert`

The contract lives in `rivulet-core::presence` and is deliberately separate from
any Discord SDK or network client. It is therefore testable on every platform and
can later be connected to Discord Rich Presence without coupling the recording
engine to a desktop integration.

## Privacy boundary

The payload contains only the application name, a generic activity description,
and a generic state. It must not contain stream keys, ingest URLs, local paths,
window titles, usernames, or captured content. The current implementation does
not contact Discord; a future adapter must make the integration explicitly
optional and provide a clear opt-out in Settings.

## Roadmap

The optional Discord Rich Presence adapter is planned for **M5 – Ecosystem &
Platform Parity**. The adapter is intentionally not part of the recording or
streaming critical path. Its Definition of Done is documented in the curated
[`docs/obs-vision-roadmap.md`](obs-vision-roadmap.md): opt-out, non-blocking
lifecycle, privacy-safe payloads, graceful degradation, and tests.

## Recommended integration

A future Discord adapter should:

1. use the existing `PresenceStatus` payload;
2. update presence only on activity transitions or a bounded interval;
3. disconnect cleanly when Rivulet exits or the user disables the feature;
4. never block capture, encoding, or the UI thread;
5. degrade silently to the in-app status when Discord is unavailable.
