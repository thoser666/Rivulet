# OBS WebSocket remote control (Stream Deck / TouchPortal)

Rivulet ships a small, self-contained server that speaks the **OBS WebSocket
v5 (JSON)** protocol — the wire format used by the Stream Deck OBS plugin,
TouchPortal, and tools such as `obs-websocket-js`. It lets those ecosystem
tools switch scenes and control recording/streaming exactly like they would
against OBS Studio.

- Issue: [#72](https://github.com/thoser666/Rivulet/issues/72) (M5 roadmap)
- Crate: `rivulet-obs-websocket` (new workspace member)
- Wire format reference: <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>

> **Compatibility / risk boundary:** this is a protocol-compatible surface for
> ecosystem tooling (Stream Deck, TouchPortal), **not** an OBS Studio
> equivalent — see [Compatibility / risk boundary](#compatibility--risk-boundary-m5-gate).

## What is implemented

| Area | Requests / events |
|---|---|
| Session | `GetVersion`, `GetAuthRequired`, v5 Hello → Identify → Identified handshake, RPC version 1, `obswebsocket.json` subprotocol |
| Authentication | Optional SHA-256 challenge/response (`base64(SHA256(secret + challenge))`, `secret = base64(SHA256(password + salt))`), wrong password closes with `AuthenticationFailed` (4009) |
| Scenes | `GetSceneList`, `GetCurrentProgramScene`, `SetCurrentProgramScene` |
| Sources | `GetInputList` |
| Recording | `StartRecord`, `StopRecord`, `ToggleRecord`, `PauseRecord`, `UnpauseRecord`, `GetRecordStatus` |
| Streaming | `StartStream`, `StopStream`, `ToggleStream`, `GetStreamStatus` |
| Audio | `ToggleInputMute` (requires `inputName`; validates against `GetInputList`) |
| Replay buffer | `GetReplayBufferStatus`, `StartReplayBuffer`, `StopReplayBuffer`, `ToggleReplayBuffer`, `SaveReplayBuffer` (optional `saveReplayPath` override, echoed back through the saved event) |
| Studio mode | `GetStudioModeEnabled`, `SetStudioModeEnabled` (requires boolean `studioModeEnabled`) |
| Events | `CurrentProgramSceneChanged` (intent `Scenes`), `RecordStateChanged` / `StreamStateChanged` / `ReplayBufferStateChanged` / `ReplayBufferSaved` (intent `Outputs`), `InputMuteStateChanged` (intent `Inputs`), `StudioModeStateChanged` (intent `UI`) — delivered only to clients subscribed to that intent |
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

The server binds to **127.0.0.1 only** by default — it is not reachable from
other machines. If you need remote access, use the **Remote companion**
(`docs/remote-companion.md`, M6): ticking *Allow access from the network (LAN)*
there binds the obs-websocket server **and** the companion page to `0.0.0.0`
and *requires* a password (a passwordless LAN bind is refused). Alternatively
run it through a local tunnel/SSH forward and enable the password. Remote
stream **start/stop** additionally requires the explicit permission flag in the
Remote companion settings — scene and recording control work on the LAN
without it.

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
64` for General + Scenes + Outputs). The `Hello` frame reports
`obsWebSocketVersion 5.0.0` and `rpcVersion 1` — the exact values Stream Deck
plugins check before offering their actions, and pinned by the Stream Deck
compat test (`tests/streamdeck_compat.rs`).

## Stream Deck (OBS plugin)

1. Install a Stream Deck plugin that speaks the OBS WebSocket v5 protocol
   Rivulet implements (obs-websocket-js based, e.g. BarRaider's "OBS Tools",
   or the Bitfocus Companion / Touch Portal integrations). Note: the official
   Elgato "OBS Studio" plugins are internal OBS plugins and do not connect
   over obs-websocket.
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
| Record | `ToggleRecord` | `{}` |
| Stream | `ToggleStream` | `{}` |
| Record state (icon) | `GetRecordStatus` | `{}` |
| Mute | `ToggleInputMute` | `{"inputName": "<input>"}` |
| Stream state (icon) | `GetStreamStatus` | `{}` |
| Replay Save | `SaveReplayBuffer` | `{}` (or `{"saveReplayPath": "<path>"}`) |
| Replay On/Off | `ToggleReplayBuffer` | `{}` |
| Replay state (icon) | `GetReplayBufferStatus` | `{}` |
| Studio Mode | `SetStudioModeEnabled` | `{"studioModeEnabled": true/false}` |

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
{"requestType": "ToggleRecord", "requestData": {}}
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
{"obsVersion": "0.65.0-alpha.55", "obsWebSocketVersion": "5.0.0", "rpcVersion": 1, "availableRequests": ["GetVersion", "GetSceneList", "StartRecord", "ToggleInputMute", "…"]}
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

**Toggle recording** — request (v5 wire name; the v4 `ToggleRecording` is NOT accepted):

```json
{"op": 6, "d": {"requestType": "ToggleRecord", "requestId": "r1", "requestData": {}}}
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
{"op": 8, "d": {"requestId": "b1", "haltOnFailure": true, "executionType": 1, "requests": [{"requestType": "SetCurrentProgramScene", "requestData": {"sceneName": "Game"}}, {"requestType": "StartStream", "requestData": {}}]}}
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

## Compatibility / risk boundary (M5 gate)

Rivulet's OBS WebSocket server is a **compatibility surface, not an OBS parity
mode** (M5 gate item "OBS compatibility mode is explicitly marked as a
compatibility/risk boundary", `docs/milestone-quality-gates.md`). It exists so
ecosystem tools keep working against Rivulet; it deliberately does not
implement everything OBS Studio exposes:

- **Wire-format conformance only.** Requests outside the implemented subset
  (source-parameter editing, filters, transitions, replay buffer, screenshots,
  parallel `RequestBatch`) are rejected honestly with `UnknownRequestType`
  (204). A workflow that works in OBS Studio is not guaranteed to exist here.
- **Local-only control.** The server binds to `127.0.0.1` only and is not
  reachable from other machines; remote access requires an explicit
  tunnel/SSH forward.
- **Authentication is optional.** With an empty password, **any local process**
  can switch scenes or start/stop recording/streaming. On shared machines set a
  password; never expose the port without one.
- **State is Rivulet state.** `SetCurrentProgramScene`/recording/streaming act
  on Rivulet's own `SceneManager`/engine through `ObsBackend` and return honest
  errors (e.g. `501 "no source selected"`) instead of pretending success.
- **Protocol drift.** The server pins the upstream obs-websocket **v5** RPC
  (`rpcVersion 1`) and the CI smoke test drives the real handshake over TCP;
  a future upstream protocol change must be validated against the pinned
  reference, not assumed compatible.

This boundary marker, the matrix in `docs/platform-feature-matrix.md`, and their
README links are pinned by `rivulet-core/tests/ci_pinning.rs` so the documents
cannot drift out of the M5 gate evidence.

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
- **Stream Deck compatibility contract** in `tests/streamdeck_compat.rs` — a
  v5 client drives the exact connect-then-control sequence the ecosystem's
  Stream Deck plugins use: version check against `availableRequests`, scene
  list/switch, record start/pause/unpause/stop + `ToggleRecord`, stream
  start/stop + `ToggleStream`, `ToggleInputMute` (missing/unknown input →
  `300`/`600`), the replay-buffer action set (status, start, save with an
  explicit path echoed through `ReplayBufferSaved`, toggle, not-running
  guard), the studio-mode set (status, enable/disable with the `UI`-intent
  event, idempotency guard), a batch refresh, and the authenticated handshake.
  It also pins that the v4 spellings (`StartRecording`, `ToggleStreaming`, …)
  are rejected with `UnknownRequestType`, so a silent protocol regression
  cannot pass CI.
- **CI loopback smoke** — the `OBS WebSocket Smoke` job in `.github/workflows/ci.yml`
  runs `cargo test -p rivulet-obs-websocket --test client_smoke` on every push,
  starting the server on `127.0.0.1` and driving it with the real tungstenite
  client inside the pipeline. The job is wired into the required `CI`
  aggregate check, and `rivulet-core/tests/ci_pinning.rs` fails if the wiring
  drifts (job name, real-loopback usage, aggregate dependency).

  **Handshake robustness:** the accept loop runs a non-blocking listener so
  shutdown stays responsive; on Windows the accepted socket *inherits* the
  non-blocking mode, which intermittently broke tungstenite's handshake read
  (`Protocol(HandshakeIncomplete)` under parallel test load). `run_session`
  therefore restores blocking **before** the handshake (not after), and the
  auth-rejection smoke uses the same retry loop as `TestClient::connect` so a
  transient drop cannot flake the suite. A **parallel load test**
  (`parallel_clients_all_complete_handshake_under_load`) bursts 24 clients at
  one server simultaneously — released through a `Barrier` so the TCP connects
  and WebSocket handshakes really overlap — and requires every client to
  complete Hello/Identify plus a request round-trip, then proves a fresh
  client still connects afterwards. All of it (blocking order, shared retry,
  load test) is locked in by `ci_pinning.rs`.
- **GUI integration**: settings toggle/port/password persistence is covered
  by the existing GUI test harness, and i18n keys exist in both locales.