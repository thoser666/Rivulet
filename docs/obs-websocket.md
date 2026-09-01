# OBS WebSocket remote control (Stream Deck / TouchPortal)

Rivulet ships a small, self-contained server that speaks the **OBS WebSocket
v5 (JSON)** protocol — the wire format used by the Stream Deck OBS plugin,
TouchPortal, and tools such as `obs-websocket-js`. It lets those ecosystem
tools switch scenes and control recording/streaming exactly like they would
against OBS Studio.

- Issue: [#72](https://github.com/thoser666/Rivulet/issues/72) (M5 roadmap)
- Crate: `rivulet-obs-websocket` (new workspace member)
- Wire format reference: <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>

## What is implemented

| Area | Requests / events |
|---|---|
| Session | `GetVersion`, `GetAuthRequired`, v5 Hello → Identify → Identified handshake, RPC version 1, `obswebsocket.json` subprotocol |
| Authentication | Optional SHA-256 challenge/response (`base64(SHA256(secret + challenge))`, `secret = base64(SHA256(password + salt))`), wrong password closes with `AuthenticationFailed` (4009) |
| Scenes | `GetSceneList`, `GetCurrentProgramScene`, `SetCurrentProgramScene` |
| Sources | `GetInputList` |
| Recording | `StartRecording`, `StopRecording`, `ToggleRecording`, `GetRecordStatus` |
| Streaming | `StartStreaming`, `StopStreaming`, `ToggleStreaming`, `GetStreamStatus` |
| Events | `CurrentProgramSceneChanged` (intent `Scenes`), `RecordStateChanged` / `StreamStateChanged` (intent `Outputs`) — delivered only to clients subscribed to that intent |
| Batching | `RequestBatch`/`RequestBatchResponse` with serial execution and `haltOnFailure` |

Status codes follow the v5 reference (`Success`=100, `UnknownRequestType`=204,
`OutputRunning`=500, `OutputNotRunning`=501, `ResourceNotFound`=600, …) and
failures always include a `comment`.

## How to enable

1. Open **Settings → OBS WebSocket (Stream Deck)**.
2. Tick **Enable remote control**.
3. Choose a **port** (default `4455`, OBS-compatible).
4. Optionally set a **password** — empty disables authentication.
5. The status line reports the listen address (`ws://127.0.0.1:4455`).

The server binds to **127.0.0.1 only** — it is not reachable from other
machines. If you need remote access, run it through a local tunnel/SSH
forward and enable the password.

## Connecting a client

In **OBS WebSocket settings** inside the Stream Deck plugin (or
`obs-websocket-js`), point at:

```
Host: 127.0.0.1
Port: 4455
Password: <the password you configured, or empty>
```

The client does a normal v5 handshake: `Hello` (op 0) → `Identify` (op 1) →
`Identified` (op 2), then requests (op 6) and batches (op 8). Subscribe to
events by sending `eventSubscriptions` in `Identify` (bitmask; e.g. `1 + 4 +
64` for General + Scenes + Outputs).

## Behaviour notes (honesty section)

- The server is **protocol-focused**. Application state (scene names, source
  names, output activity) is supplied through
  `rivulet_obs_websocket::backend::ObsBackend`, which the GUI implements over
  its `SceneManager` / engine. Read requests are answered from a snapshot the
  GUI refreshes every frame; mutating commands are executed on the UI thread
  on the next frame.
- **Scene switching** (`SetCurrentProgramScene`) is fully wired: it calls the
  same `SceneManager::switch_to` path the GUI uses, so remote switches match
  in-app switches.
- **Recording** start/stop uses the GUI's active capture path (the currently
  selected monitor/window/camera). If no source is selected the request fails
  with a comment instead of silently doing nothing.
- **Streaming** start uses the currently configured stream platform/ingest
  settings in the Stream view; if no ingest is configured the request fails
  with a comment.
- Events are broadcast as state **changes** — both requests made by clients
  and changes you make in the Rivulet window broadcast to subscribed clients.
- Not implemented (explicitly out of scope for #72): full source-parameter
  editing, filters, transitions, replay buffer, screenshots, `RequestBatch`
  parallel execution. Such request names return `UnknownRequestType`.
- `GetInputList` reports source *names* with a fixed `inputKind`
  (`rivulet_source`); it does not currently report per-scene scene items.

## Verification

The crate contains two layers of tests:

- **Unit tests** in `src/protocol.rs`, `src/backend.rs`, `src/server.rs`
  (auth vector, request-name round-trip, status/close-code constants, event
  intents, snapshot rendering).
- **End-to-end smoke tests** in `tests/client_smoke.rs` — a real (non-mock)
  WebSocket client connects over TCP to a server on an ephemeral port and
  exercises the full handshake, authentication (correct + wrong password),
  read requests, scene switching with event delivery, recording/streaming
  control, request batches incl. `haltOnFailure`, unknown request rejection,
  and clean shutdown. This is the “verified with a real client” acceptance
  criterion of the issue.
- **GUI integration**: settings toggle/port/password persistence is covered
  by the existing GUI test harness, and i18n keys exist in both locales.