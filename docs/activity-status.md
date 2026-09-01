# Activity status and Discord Rich Presence

Rivulet now exposes a platform-neutral activity-status contract for a Discord-style
status message. The GUI shows the current product activity using the **Rivulet**
name. Activity labels are localized through the selected UI locale and may include
the explicitly selected game name, for example:

- `Rivulet · Aufnahme` / `Aufnahme · Elden Ring`
- `Rivulet · Streamt` / `Streamt · Elden Ring`
- `Rivulet · Aufnahme + Stream` / `Aufnahme + Stream · Elden Ring`
- `Rivulet · Pausiert` / `Pausiert · Elden Ring`

Game names must be supplied from the user's explicit source selection (the
selected game-capture window or the selected window-capture title), not inferred
from window titles or captured content. When a game-capture window is selected
its title is used as the game name; otherwise a selected window-capture title is
used, and a plain monitor source sends no game name.

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

The **Discord Developer Application client id is configurable** in Settings →
Discord Rich Presence. Set it to the Client ID of a Discord application created
at <https://discord.com/developers/applications> with "Rich Presence" enabled.
Until a non-empty client id is entered and applied, the adapter stays off. The
value is persisted across sessions; changing it (via the Apply button) rebuilds
the adapter on the next frame.

## CI smoke test

CI runs the real adapter worker (`DiscordPresence`) end to end against a local
IPC listener, verifying that a pushed status produces a **handshake frame
followed by a `SET_ACTIVITY` frame** on the wire. On Linux/macOS the listener is
an actual `discord-ipc-0`-style Unix-domain socket in a temp dir (the config
`ipc_socket_path` override points the worker at it); on Windows the worker is
run against a local **named pipe** server. The step is `Discord adapter IPC smoke
Test` in `ci.yml` (job `Build & Test on <os>-latest`).

## When is the status updated?

The presence is pushed on **activity transitions** (Ready → Recording,
Recording → Paused, …), never every frame. The per-frame reconcile call lives in
the global UI update (`ui()`), **not** inside the Stream view's draw code:
starting or stopping a recording from the Record view updates the Discord status
immediately, without having to open the Stream tab. Only activity changes cross
the IPC boundary; an unchanged status is a no-op even when the sync runs every
frame.

## Status legend in the Stream view

The Stream view renders a **legend** below the current status: one row per
state (Fehler/Error, Aufnahme+Stream, Pausiert, Aufnahme, Streamt, Bereit) with
the currently active state highlighted. Hovering any row shows a tooltip that
explains **when the state appears** and **what it means** — e.g. "Ready" is
the default on startup and after stopping, "Paused" means frames are captured
but not written. The legend is generated from `PresenceActivity::all()` (so it
never drifts from the model) and every tooltip is localized through
`tooltip_i18n_key()` in both DE/EN (enforced by the i18n parity test and the
`presence_legend_lists_every_state_with_a_tooltip` ci_pinning guard).

## The "Error" state

`PresenceActivity::Error` is produced whenever the engine or a capture thread
reported a failure (`last_error` is set — e.g. a pipeline build/start/push
error or a capture that delivered no frames). It has **priority over the
activity labels**: even while a recording or stream is nominally active, a
fresh failure surfaces as "Error"/"Fehler" instead of the activity label.

The order of evaluation is: Error → Recording + streaming → Paused → Recording
→ Streaming → Ready.

The error state stays **privacy-safe**: only the localized "Error"/"Fehler"
label is sent, never the raw error text (which may contain paths or other
implementation details). The state recovers automatically on the next start —
recording starts and both streaming-start paths clear `last_error`, so the
status moves back to the activity label rather than sticking on "Error".
