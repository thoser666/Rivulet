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

## Stream Deck (OBS plugin)

1. Install the **OBS Studio** plugin from the Stream Deck store (BarRaider's
   or the official OBS plugin) — it speaks the OBS WebSocket v5 protocol
   Rivulet implements.
2. Start Rivulet and enable the server: **Settings → OBS WebSocket (Stream
   Deck) → Enable remote control** (default port `4455`, password optional).
3. Open Stream Deck → add the **OBS Studio** action you want (e.g. *Switch
   Scene*, *Record*, *Stream*).
4. In the action's settings, create a new **Connection**:
   - **Host:** `127.0.0.1`
   - **Port:** `4455`
   - **Password:** the one you configured (leave empty if authentication is
     off)
   - **Version:** 5.x (the plugin asks for the WebSocket version — choose 5)
5. The action's dropdowns (scenes, …) are populated from the live snapshot
   via `GetSceneList`. If a dropdown is empty, press **Refresh** in the
   action settings while Rivulet is running.

Common actions and the requests they issue:

| Action | Request | Payload |
|---|---|---|
| Switch Scene | `SetCurrentProgramScene` | `{"sceneName": "<name>"}` |
| Record | `ToggleRecording` | `{}` |
| Stream | `ToggleStreaming` | `{}` |
| Record state (icon) | `GetRecordStatus` | `{}` |
| Stream state (icon) | `GetStreamStatus` | `{}` |

## TouchPortal

1. Install the **OBS WebSocket** TouchPortal plugin (third-party, speaks
   obs-websocket v5).
2. Enable the server in Rivulet as above.
3. In TouchPortal → OBS WebSocket settings, set the same host/port/password
   (`127.0.0.1` / `4455`).
4. Add buttons for *Scene Switch* (pick a scene), *Start/Stop Recording*,
   *Start/Stop Streaming*, or a **Custom Request** button.

For a **Custom Request** button, the request body is the v5 payload exactly
as sent on the wire, e.g.:

```json
{"requestType": "ToggleRecording", "requestData": {}}
```

The plugin wraps this into the `{ "op": 6, "d": … }` envelope for you.

## Example requests (raw WebSocket JSON)

These are the exact messages the plugins send under the hood — useful for
scripts, `websocat`, or `obs-websocket-js`. Each request is `op` 6 with a
`requestId` of your choice; the response (`op` 7) echoes it.

**GetVersion** — request:

```json
{"op": 6, "d": {"requestType": "GetVersion", "requestId": "v1", "requestData": {}}}
```

Response data (abridged):

```json
{"obsVersion": "0.65.0-alpha.55", "obsWebSocketVersion": "5.0.0", "rpcVersion": 1, "availableRequests": ["GetVersion", "GetSceneList", "StartRecording", "…"]}
```

**GetSceneList** — request:

```json
{"op": 6, "d": {"requestType": "GetSceneList", "requestId": "s1", "requestData": {}}}
```

Response data:

```json
{"currentProgramSceneName": "Game", "currentPreviewSceneName": null, "scenes": [{"sceneName": "Game", "sceneIndex": 0}, {"sceneName": "Cam", "sceneIndex": 1}]}
```

**Switch scene** — request:

```json
{"op": 6, "d": {"requestType": "SetCurrentProgramScene", "requestId": "sw1", "requestData": {"sceneName": "Cam"}}}
```

Success → `{"result": true, "code": 100}`. Subscribed clients also receive
`CurrentProgramSceneChanged` (`op` 5, event intent `Scenes`):

```json
{"op": 5, "d": {"eventType": "CurrentProgramSceneChanged", "eventIntent": 4, "eventData": {"sceneName": "Cam"}}}
```

**Toggle recording** — request:

```json
{"op": 6, "d": {"requestType": "ToggleRecording", "requestId": "r1", "requestData": {}}}
```

If no capture source is selected, this fails honestly instead of silently
doing nothing:

```json
{"requestStatus": {"result": false, "code": 501, "comment": "no source selected"}}
```

**GetRecordStatus** — request and response data:

```json
{"op": 6, "d": {"requestType": "GetRecordStatus", "requestId": "st1", "requestData": {}}}
```

```json
{"outputActive": true, "outputPaused": false, "outputTimecode": "00:01:23.456", "outputDuration": 83456, "outputBytes": 12345678}
```

**Batch** (switch scene *and* start streaming atomically, stop on failure) —
`op` 8 with `haltOnFailure`:

```json
{"op": 8, "d": {"requestId": "b1", "haltOnFailure": true, "executionType": 1, "requests": [{"requestType": "SetCurrentProgramScene", "requestData": {"sceneName": "Game"}}, {"requestType": "StartStreaming", "requestData": {}}]}}
```

Each entry answers with its own `requestStatus`; the batch response (`op` 9)
contains one result per request in order.

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
- **CI loopback smoke** — the `OBS WebSocket Smoke` job in `.github/workflows/ci.yml`
  runs `cargo test -p rivulet-obs-websocket --test client_smoke` on every push,
  starting the server on `127.0.0.1` and driving it with the real tungstenite
  client inside the pipeline. The job is wired into the required `CI`
  aggregate check, and `rivulet-core/tests/ci_pinning.rs` fails if the wiring
  drifts (job name, real-loopback usage, aggregate dependency).
- **GUI integration**: settings toggle/port/password persistence is covered
  by the existing GUI test harness, and i18n keys exist in both locales.