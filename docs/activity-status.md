# Activity status and Discord Rich Presence

Rivulet now exposes a platform-neutral activity-status contract for a Discord-style
status message. The GUI shows the current product activity using the **Rivulet**
name. Activity labels are localized through the selected UI locale and may include
the explicitly selected game name, for example:

- `Rivulet · Aufnahme` / `Aufnahme · Elden Ring`
- `Rivulet · Streamt` / `Streamt · Elden Ring`
- `Rivulet · Aufnahme + Stream` / `Aufnahme + Stream · Elden Ring`
- `Rivulet · Pausiert` / `Pausiert · Elden Ring`

Game names must be supplied from the user's game selection, not inferred from
window titles or captured content.

The contract lives in `rivulet-core::presence` and is deliberately separate from
any Discord SDK or network client. The optional non-blocking adapter in
`rivulet-core::discord` consumes the payload and forwards it to the local
Discord desktop client over its RPC IPC socket (`discord-ipc-0`: Unix domain
socket on Linux/macOS, named pipe on Windows).

## Privacy boundary

The payload contains only the application name, a localized activity description,
and an optional user-selected game name in the state. It must not contain stream
keys, ingest URLs, local paths, window titles, usernames, or captured content.
The adapter makes the integration explicitly optional and provides a clear opt-out
in Settings (the toggle shown next to the status in the Stream view).

## Roadmap

The optional Discord Rich Presence adapter is implemented as part of **M5 –
Ecosystem & Platform Parity**. The adapter is intentionally not part of the
recording or streaming critical path. Its Definition of Done is documented in the
curated [`docs/obs-vision-roadmap.md`](obs-vision-roadmap.md): opt-out,
non-blocking lifecycle, privacy-safe payloads, graceful degradation, and tests.

## Recommended integration

The adapter (`rivulet-core::discord`) implements the recommended contract:

1. it uses the existing `PresenceStatus` payload;
2. it updates presence only on activity transitions;
3. it disconnects cleanly when Rivulet exits or the user disables the feature;
4. it never blocks capture, encoding, or the UI thread (all IPC runs on a
   dedicated worker thread; the fast path only enqueues into a channel);
5. it degrades silently to the in-app status when Discord is unavailable — the
   worker retries on the next transition without panicking or spinning.

The opt-out toggle is persisted. When disabled, no worker thread is spawned at
all and `PresenceStatus::set_activity` is a no-op.
