//! Stream Deck compatibility contract (CI gate).
//!
//! The obs-websocket ecosystem's Stream Deck integrations (obs-websocket-js
//! based plugins such as "OBS Tools", Bitfocus Companion, Touch Portal, …)
//! speak the OBS WebSocket **v5** protocol. This test drives the server with
//! exactly the request set those clients send on connect and during normal
//! button use, so a protocol drift (renamed request, dropped field, changed
//! version payload) fails CI instead of breaking real Stream Deck setups.
//!
//! Wire names are the v5 spellings (`StartRecord`, `ToggleStream`,
//! `ToggleInputMute`) — NOT the v4 ones (`StartRecording`, `ToggleStreaming`).
//! The final test pins that v4 names are rejected with `Unknown request type`.

use std::time::Duration;

use rivulet_obs_websocket::backend::{
    ObsBackend, ObsCommand, ObsCommandResult, ObsEvent, ObsSnapshot,
};
use rivulet_obs_websocket::{protocol, server};
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::{Message, WebSocket};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Error as WsError};

/// In-memory backend modelling the app: two scenes, one input, both outputs.
struct MemoryBackend {
    snapshot: Arc<std::sync::Mutex<ObsSnapshot>>,
    last_command: Arc<std::sync::Mutex<Option<ObsCommand>>>,
}

use std::sync::Arc;

impl MemoryBackend {
    fn new() -> Self {
        let snapshot = ObsSnapshot {
            scenes: vec!["Game".into(), "BRB".into()],
            current_scene: Some("Game".into()),
            sources: vec!["Mic".into()],
            ..Default::default()
        };
        Self {
            snapshot: Arc::new(std::sync::Mutex::new(snapshot)),
            last_command: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

impl ObsBackend for MemoryBackend {
    fn snapshot(&self) -> ObsSnapshot {
        self.snapshot.lock().unwrap().clone()
    }

    fn execute(&self, command: ObsCommand) -> ObsCommandResult {
        *self.last_command.lock().unwrap() = Some(command.clone());
        match &command {
            ObsCommand::SetCurrentScene(name) => {
                let mut snap = self.snapshot.lock().unwrap();
                if snap.scenes.iter().any(|s| s == name) {
                    snap.current_scene = Some(name.clone());
                    ObsCommandResult::Success(vec![ObsEvent::CurrentProgramSceneChanged {
                        scene_name: name.clone(),
                    }])
                } else {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::RESOURCE_NOT_FOUND,
                        comment: format!("Scene '{name}' not found"),
                    }
                }
            }
            ObsCommand::StartRecording => {
                let mut snap = self.snapshot.lock().unwrap();
                if snap.recording {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::OUTPUT_RUNNING,
                        comment: "Recording already active".into(),
                    }
                } else {
                    snap.recording = true;
                    snap.recording_paused = false;
                    ObsCommandResult::Success(vec![ObsEvent::RecordStateChanged {
                        active: true,
                        paused: false,
                    }])
                }
            }
            ObsCommand::StopRecording => {
                let mut snap = self.snapshot.lock().unwrap();
                if snap.recording {
                    snap.recording = false;
                    snap.recording_paused = false;
                    ObsCommandResult::Success(vec![ObsEvent::RecordStateChanged {
                        active: false,
                        paused: false,
                    }])
                } else {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::OUTPUT_NOT_RUNNING,
                        comment: "Recording not active".into(),
                    }
                }
            }
            ObsCommand::PauseRecording => {
                let mut snap = self.snapshot.lock().unwrap();
                if snap.recording && !snap.recording_paused {
                    snap.recording_paused = true;
                    ObsCommandResult::Success(vec![ObsEvent::RecordStateChanged {
                        active: true,
                        paused: true,
                    }])
                } else {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::OUTPUT_NOT_PAUSED,
                        comment: "Recording not active or already paused".into(),
                    }
                }
            }
            ObsCommand::UnpauseRecording => {
                let mut snap = self.snapshot.lock().unwrap();
                if snap.recording && snap.recording_paused {
                    snap.recording_paused = false;
                    ObsCommandResult::Success(vec![ObsEvent::RecordStateChanged {
                        active: true,
                        paused: false,
                    }])
                } else {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::OUTPUT_NOT_PAUSED,
                        comment: "Recording not paused".into(),
                    }
                }
            }
            ObsCommand::ToggleRecording => {
                let mut snap = self.snapshot.lock().unwrap();
                snap.recording = !snap.recording;
                snap.recording_paused = false;
                ObsCommandResult::Success(vec![ObsEvent::RecordStateChanged {
                    active: snap.recording,
                    paused: false,
                }])
            }
            ObsCommand::StartStreaming => {
                let mut snap = self.snapshot.lock().unwrap();
                if snap.streaming {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::OUTPUT_RUNNING,
                        comment: "Stream already active".into(),
                    }
                } else {
                    snap.streaming = true;
                    ObsCommandResult::Success(vec![ObsEvent::StreamStateChanged {
                        active: true,
                        reconnecting: false,
                    }])
                }
            }
            ObsCommand::StopStreaming => {
                let mut snap = self.snapshot.lock().unwrap();
                if snap.streaming {
                    snap.streaming = false;
                    ObsCommandResult::Success(vec![ObsEvent::StreamStateChanged {
                        active: false,
                        reconnecting: false,
                    }])
                } else {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::OUTPUT_NOT_RUNNING,
                        comment: "Stream not active".into(),
                    }
                }
            }
            ObsCommand::ToggleStreaming => {
                let mut snap = self.snapshot.lock().unwrap();
                snap.streaming = !snap.streaming;
                ObsCommandResult::Success(vec![ObsEvent::StreamStateChanged {
                    active: snap.streaming,
                    reconnecting: false,
                }])
            }
            ObsCommand::ToggleMute => {
                let mut snap = self.snapshot.lock().unwrap();
                snap.muted = !snap.muted;
                ObsCommandResult::Success(vec![ObsEvent::InputMuteStateChanged {
                    input_name: snap
                        .sources
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "Mic".into()),
                    muted: snap.muted,
                }])
            }
            ObsCommand::StartReplayBuffer => {
                let mut snap = self.snapshot.lock().unwrap();
                if snap.replay_buffer_active {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::OUTPUT_RUNNING,
                        comment: "Replay buffer already active".into(),
                    }
                } else {
                    snap.replay_buffer_active = true;
                    ObsCommandResult::Success(vec![ObsEvent::ReplayBufferStateChanged {
                        active: true,
                    }])
                }
            }
            ObsCommand::StopReplayBuffer => {
                let mut snap = self.snapshot.lock().unwrap();
                if snap.replay_buffer_active {
                    snap.replay_buffer_active = false;
                    ObsCommandResult::Success(vec![ObsEvent::ReplayBufferStateChanged {
                        active: false,
                    }])
                } else {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::OUTPUT_NOT_RUNNING,
                        comment: "Replay buffer not active".into(),
                    }
                }
            }
            ObsCommand::ToggleReplayBuffer => {
                let mut snap = self.snapshot.lock().unwrap();
                snap.replay_buffer_active = !snap.replay_buffer_active;
                ObsCommandResult::Success(vec![ObsEvent::ReplayBufferStateChanged {
                    active: snap.replay_buffer_active,
                }])
            }
            ObsCommand::SaveReplayBuffer => {
                let snap = self.snapshot.lock().unwrap();
                if snap.replay_buffer_active {
                    ObsCommandResult::Success(vec![ObsEvent::ReplayBufferSaved {
                        saved_replay_path: None,
                    }])
                } else {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::OUTPUT_NOT_RUNNING,
                        comment: "Replay buffer not active".into(),
                    }
                }
            }
            ObsCommand::SaveReplayBufferTo(path) => {
                let snap = self.snapshot.lock().unwrap();
                if snap.replay_buffer_active {
                    ObsCommandResult::Success(vec![ObsEvent::ReplayBufferSaved {
                        saved_replay_path: Some(path.clone()),
                    }])
                } else {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::OUTPUT_NOT_RUNNING,
                        comment: "Replay buffer not active".into(),
                    }
                }
            }
            ObsCommand::SetStudioMode(enabled) => {
                let enabled = *enabled;
                let mut snap = self.snapshot.lock().unwrap();
                if snap.studio_mode == enabled {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::RESOURCE_ALREADY_EXISTS,
                        comment: "Studio mode already in that state".into(),
                    }
                } else {
                    snap.studio_mode = enabled;
                    ObsCommandResult::Success(vec![ObsEvent::StudioModeStateChanged { enabled }])
                }
            }
        }
    }
}

/// A minimal obs-websocket-js-shaped client: Hello → Identify (with the
/// request-intent subscription real plugins use) → Identified, then typed
/// request helpers.
struct V5Client {
    ws: WebSocket<MaybeTlsStream<std::net::TcpStream>>,
}

impl V5Client {
    /// Connect like the Stream Deck plugins do: advertise the JSON
    /// subprotocol and (optionally) answer the auth challenge.
    fn connect(port: u16, password: Option<&str>) -> Self {
        Self::connect_with_intents(
            port,
            password,
            protocol::intent::SCENES | protocol::intent::OUTPUTS | protocol::intent::INPUTS,
        )
    }

    /// As [`Self::connect`], but with an explicit intent mask: plugins
    /// subscribe to the intents their buttons need (Scenes + Outputs is the
    /// classic record/stream/scene set; mute buttons also need Inputs, and
    /// studio-mode buttons need UI for `StudioModeStateChanged`).
    fn connect_with_intents(port: u16, password: Option<&str>, intents: u32) -> Self {
        let url = format!("ws://127.0.0.1:{port}");
        let mut request = url.into_client_request().unwrap();
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            protocol::JSON_SUBPROTOCOL.parse().unwrap(),
        );
        let mut ws = match connect(request) {
            Ok((ws, _)) => ws,
            Err(e) => panic!("websocket connect: {e}"),
        };

        // ── Hello ────────────────────────────────────────────────────────
        let hello = read_json(&mut ws);
        assert_eq!(hello["op"], 0, "first message must be Hello");
        let d = &hello["d"];
        // Version fields a plugin checks before offering actions.
        assert_eq!(
            d["obsWebSocketVersion"].as_str().unwrap(),
            "5.0.0",
            "plugins gate on the obs-websocket version"
        );
        assert_eq!(d["rpcVersion"], 1);
        assert!(
            d["obsStudioVersion"].as_str().is_some(),
            "Hello must carry a version string for the About dialog"
        );
        match password {
            Some(_) => assert!(
                d["authentication"]["salt"].as_str().is_some()
                    && d["authentication"]["challenge"].as_str().is_some(),
                "password-protected server must offer challenge + salt"
            ),
            None => assert!(
                d.get("authentication").is_none() || d["authentication"].is_null(),
                "auth-free server must not require authentication"
            ),
        }

        // ── Identify ─────────────────────────────────────────────────────
        let mut identify = serde_json::json!({
            "op": 1,
            "d": { "rpcVersion": 1, "eventSubscriptions": intents }
        });
        if let Some(password) = password {
            let salt = d["authentication"]["salt"].as_str().unwrap();
            let challenge = d["authentication"]["challenge"].as_str().unwrap();
            let secret = protocol::compute_secret(password, salt);
            identify["d"]["authentication"] =
                serde_json::json!(protocol::compute_auth_response(&secret, challenge));
        }
        ws.send(Message::Text(
            serde_json::to_string(&identify).unwrap().into(),
        ))
        .unwrap();

        // ── Identified ───────────────────────────────────────────────────
        let identified = read_json(&mut ws);
        assert_eq!(identified["op"], 2, "expected Identified, got {identified}");
        assert_eq!(identified["d"]["negotiatedRpcVersion"], 1);
        Self { ws }
    }

    fn request(&mut self, request_type: &str, data: serde_json::Value) -> serde_json::Value {
        let request_id = format!("sd-{request_type}");
        self.ws
            .send(Message::Text(
                serde_json::to_string(&serde_json::json!({
                    "op": 6,
                    "d": {
                        "requestType": request_type,
                        "requestId": request_id,
                        "requestData": data,
                    }
                }))
                .unwrap()
                .into(),
            ))
            .unwrap();
        let response = read_json(&mut self.ws);
        assert_eq!(response["op"], 7, "expected RequestResponse");
        assert_eq!(response["d"]["requestType"], request_type);
        assert_eq!(response["d"]["requestId"], request_id);
        response["d"].clone()
    }

    /// Assert a request succeeded and return its responseData.
    fn ok(&mut self, request_type: &str, data: serde_json::Value) -> serde_json::Value {
        let d = self.request(request_type, data);
        assert_eq!(
            d["requestStatus"]["code"],
            protocol::status::SUCCESS,
            "{request_type} must succeed: {d}"
        );
        d["responseData"].clone()
    }

    /// Read the next event frame (asserting its type).
    fn expect_event(&mut self, event_type: &str) -> serde_json::Value {
        let event = read_json(&mut self.ws);
        assert_eq!(event["op"], 5, "expected an event frame");
        assert_eq!(event["d"]["eventType"], event_type);
        event["d"]["eventData"].clone()
    }
}

fn read_json(ws: &mut WebSocket<MaybeTlsStream<std::net::TcpStream>>) -> serde_json::Value {
    loop {
        match ws.read().expect("read message") {
            Message::Text(text) => return serde_json::from_str(&text).expect("valid JSON"),
            Message::Ping(payload) => {
                ws.send(Message::Pong(payload)).unwrap();
            }
            Message::Close(_) => panic!("connection closed unexpectedly"),
            other => panic!("unexpected message type: {other:?}"),
        }
    }
}

fn start_server(password: Option<String>) -> (server::ObsServerHandle, u16, Arc<MemoryBackend>) {
    let backend = Arc::new(MemoryBackend::new());
    let handle = server::start(backend.clone(), password, 0).expect("server binds");
    let port = handle.local_addr().port();
    (handle, port, backend)
}

/// The exact request names Stream Deck OBS plugins need, as advertised by
/// GetVersion. obs-websocket-js callers frequently consult this list (or
/// display it) before binding actions.
#[test]
fn get_version_advertises_the_v5_action_set_plugins_bind() {
    let (_handle, port, _backend) = start_server(None);
    let mut client = V5Client::connect(port, None);

    let d = client.ok("GetVersion", serde_json::json!({}));
    assert_eq!(d["obsWebSocketVersion"], "5.0.0");
    assert_eq!(d["rpcVersion"], 1);
    assert!(d["obsVersion"].as_str().is_some());

    let available: Vec<&str> = d["availableRequests"]
        .as_array()
        .expect("availableRequests array")
        .iter()
        .map(|v| v.as_str().expect("string entry"))
        .collect();
    for request_name in [
        "GetVersion",
        "GetSceneList",
        "GetCurrentProgramScene",
        "SetCurrentProgramScene",
        "GetInputList",
        "GetRecordStatus",
        "StartRecord",
        "StopRecord",
        "ToggleRecord",
        "PauseRecord",
        "UnpauseRecord",
        "GetStreamStatus",
        "StartStream",
        "StopStream",
        "ToggleStream",
        "ToggleInputMute",
        "GetReplayBufferStatus",
        "StartReplayBuffer",
        "StopReplayBuffer",
        "ToggleReplayBuffer",
        "SaveReplayBuffer",
        "GetStudioModeEnabled",
        "SetStudioModeEnabled",
    ] {
        assert!(
            available.contains(&request_name),
            "availableRequests must advertise `{request_name}` (v5 wire name)"
        );
    }
}

/// The connect-then-control sequence of a record/stream/scene Stream Deck
/// profile: version check, scene fetch, scene switch, record start/stop,
/// stream start/stop — all with the v5 spellings and their events.
#[test]
fn full_deck_action_set_round_trips_with_events() {
    let (_handle, port, backend) = start_server(None);
    let mut client = V5Client::connect(port, None);

    // Scene list + current scene (the plugin populates its scene buttons).
    let d = client.ok("GetSceneList", serde_json::json!({}));
    assert_eq!(d["currentProgramSceneName"], "Game");
    assert_eq!(d["scenes"].as_array().unwrap().len(), 2);
    let d = client.ok("GetCurrentProgramScene", serde_json::json!({}));
    assert_eq!(d["currentProgramSceneName"], "Game");

    // Scene switch with the pushed event (button lights up on change).
    client.ok(
        "SetCurrentProgramScene",
        serde_json::json!({ "sceneName": "BRB" }),
    );
    let data = client.expect_event("CurrentProgramSceneChanged");
    assert_eq!(data["sceneName"], "BRB");

    // Input list (mute buttons validate against it).
    let d = client.ok("GetInputList", serde_json::json!({}));
    assert_eq!(d["inputs"][0]["inputName"], "Mic");

    // ── Record ───────────────────────────────────────────────────────
    let d = client.ok("GetRecordStatus", serde_json::json!({}));
    assert_eq!(d["outputActive"], false);
    client.ok("StartRecord", serde_json::json!({}));
    let data = client.expect_event("RecordStateChanged");
    assert_eq!(data["outputActive"], true);
    // Double start must fail with OUTPUT_RUNNING (plugin shows the error).
    let d = client.request("StartRecord", serde_json::json!({}));
    assert_eq!(d["requestStatus"]["code"], protocol::status::OUTPUT_RUNNING);

    // Pause semantics (Stream Deck has dedicated pause/unpause buttons).
    client.ok("PauseRecord", serde_json::json!({}));
    let data = client.expect_event("RecordStateChanged");
    assert_eq!(data["outputPaused"], true);
    // Pausing again must fail, unpausing succeeds.
    let d = client.request("PauseRecord", serde_json::json!({}));
    assert_eq!(
        d["requestStatus"]["code"],
        protocol::status::OUTPUT_NOT_PAUSED
    );
    client.ok("UnpauseRecord", serde_json::json!({}));
    let _ = client.expect_event("RecordStateChanged");
    let d = client.ok("GetRecordStatus", serde_json::json!({}));
    assert_eq!(d["outputActive"], true);
    assert_eq!(d["outputPaused"], false);

    client.ok("StopRecord", serde_json::json!({}));
    let data = client.expect_event("RecordStateChanged");
    assert_eq!(data["outputActive"], false);

    // ToggleRecord from idle starts recording (toggle buttons).
    client.ok("ToggleRecord", serde_json::json!({}));
    let _ = client.expect_event("RecordStateChanged");
    assert!(matches!(
        backend.last_command.lock().unwrap().as_ref(),
        Some(ObsCommand::ToggleRecording)
    ));
    client.ok("ToggleRecord", serde_json::json!({}));
    let _ = client.expect_event("RecordStateChanged");

    // ── Stream ───────────────────────────────────────────────────────
    client.ok("StartStream", serde_json::json!({}));
    let data = client.expect_event("StreamStateChanged");
    assert_eq!(data["outputActive"], true);
    let d = client.ok("GetStreamStatus", serde_json::json!({}));
    assert_eq!(d["outputActive"], true);
    client.ok("StopStream", serde_json::json!({}));
    let _ = client.expect_event("StreamStateChanged");
    // Toggle from idle turns it back on (toggle buttons read the state).
    client.ok("ToggleStream", serde_json::json!({}));
    let _ = client.expect_event("StreamStateChanged");
    client.ok("ToggleStream", serde_json::json!({}));
    let _ = client.expect_event("StreamStateChanged");

    // ── Mute ─────────────────────────────────────────────────────────
    client.ok("ToggleInputMute", serde_json::json!({ "inputName": "Mic" }));
    let data = client.expect_event("InputMuteStateChanged");
    assert_eq!(data["inputName"], "Mic");
    assert_eq!(data["inputMuted"], true);
    let d = client.request("ToggleInputMute", serde_json::json!({}));
    // Missing inputName is a 300 (plugins rely on the status code).
    assert_eq!(
        d["requestStatus"]["code"],
        protocol::status::MISSING_REQUEST_FIELD
    );
    let d = client.request(
        "ToggleInputMute",
        serde_json::json!({ "inputName": "DoesNotExist" }),
    );
    assert_eq!(
        d["requestStatus"]["code"],
        protocol::status::RESOURCE_NOT_FOUND
    );
}

/// The replay-buffer action set ("OBS Tools"-style replay buttons): status,
/// start with event, the not-running guard on save, an explicit-path save
/// echoed back through `ReplayBufferSaved`, and the toggle round-trip.
#[test]
fn replay_buffer_action_set_round_trips() {
    let (_handle, port, backend) = start_server(None);
    let mut client = V5Client::connect(port, None);

    // Disabled by default.
    let d = client.ok("GetReplayBufferStatus", serde_json::json!({}));
    assert_eq!(d["outputActive"], false);

    // Saving while disabled must fail with OUTPUT_NOT_RUNNING.
    let d = client.request("SaveReplayBuffer", serde_json::json!({}));
    assert_eq!(
        d["requestStatus"]["code"],
        protocol::status::OUTPUT_NOT_RUNNING
    );

    // Start + event; double start is an error.
    client.ok("StartReplayBuffer", serde_json::json!({}));
    let data = client.expect_event("ReplayBufferStateChanged");
    assert_eq!(data["outputActive"], true);
    let d = client.request("StartReplayBuffer", serde_json::json!({}));
    assert_eq!(d["requestStatus"]["code"], protocol::status::OUTPUT_RUNNING);

    // Save with an explicit path: the event carries it back verbatim.
    client.ok(
        "SaveReplayBuffer",
        serde_json::json!({ "saveReplayPath": "C:/clips/last.mp4" }),
    );
    let data = client.expect_event("ReplayBufferSaved");
    assert_eq!(data["savedReplayPath"], "C:/clips/last.mp4");
    assert!(matches!(
        backend.last_command.lock().unwrap().as_ref(),
        Some(ObsCommand::SaveReplayBufferTo(path)) if path == "C:/clips/last.mp4"
    ));

    // Toggle off, status reflects it.
    client.ok("ToggleReplayBuffer", serde_json::json!({}));
    let _ = client.expect_event("ReplayBufferStateChanged");
    let d = client.ok("GetReplayBufferStatus", serde_json::json!({}));
    assert_eq!(d["outputActive"], false);

    assert!(matches!(
        backend.last_command.lock().unwrap().as_ref(),
        Some(ObsCommand::ToggleReplayBuffer)
    ));
}

/// Studio-mode buttons: status read, enable with the UI-intent event,
/// idempotency guard, and disable.
#[test]
fn studio_mode_action_set_round_trips() {
    let (_handle, port, backend) = start_server(None);
    let mut client = V5Client::connect_with_intents(
        port,
        None,
        protocol::intent::SCENES
            | protocol::intent::OUTPUTS
            | protocol::intent::INPUTS
            | protocol::intent::UI,
    );

    let d = client.ok("GetStudioModeEnabled", serde_json::json!({}));
    assert_eq!(d["studioModeEnabled"], false);

    // Missing studioModeEnabled is a 300 (plugins rely on the status code).
    let d = client.request("SetStudioModeEnabled", serde_json::json!({}));
    assert_eq!(
        d["requestStatus"]["code"],
        protocol::status::MISSING_REQUEST_FIELD
    );

    // Enable → event → status reflects it → disable.
    client.ok(
        "SetStudioModeEnabled",
        serde_json::json!({ "studioModeEnabled": true }),
    );
    let data = client.expect_event("StudioModeStateChanged");
    assert_eq!(data["studioModeEnabled"], true);
    let d = client.ok("GetStudioModeEnabled", serde_json::json!({}));
    assert_eq!(d["studioModeEnabled"], true);
    client.ok(
        "SetStudioModeEnabled",
        serde_json::json!({ "studioModeEnabled": false }),
    );
    let data = client.expect_event("StudioModeStateChanged");
    assert_eq!(data["studioModeEnabled"], false);

    // Disabling while already off → RESOURCE_ALREADY_EXISTS, per the v5 spec.
    let d = client.request(
        "SetStudioModeEnabled",
        serde_json::json!({ "studioModeEnabled": false }),
    );
    assert_eq!(
        d["requestStatus"]["code"],
        protocol::status::RESOURCE_ALREADY_EXISTS
    );

    assert!(matches!(
        backend.last_command.lock().unwrap().as_ref(),
        Some(ObsCommand::SetStudioMode(false))
    ));
}

/// A batch as the plugins send it (initial refresh: version + scenes + inputs
/// in one round-trip) must execute serially and report per-request status.
#[test]
fn deck_batch_refresh_round_trips() {
    let (_handle, port, _backend) = start_server(None);
    let mut client = V5Client::connect(port, None);

    client
        .ws
        .send(Message::Text(
            serde_json::to_string(&serde_json::json!({
                "op": 8,
                "d": {
                    "requestId": "deck-refresh",
                    "requests": [
                        { "requestType": "GetVersion" },
                        { "requestType": "GetSceneList" },
                        { "requestType": "GetInputList" },
                    ],
                }
            }))
            .unwrap()
            .into(),
        ))
        .unwrap();

    let response = read_json(&mut client.ws);
    assert_eq!(response["op"], 9, "expected RequestBatchResponse");
    assert_eq!(response["d"]["requestId"], "deck-refresh");
    let results = response["d"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
    for result in results {
        assert_eq!(
            result["requestStatus"]["code"],
            protocol::status::SUCCESS,
            "batch member failed: {result}"
        );
    }
}

/// With a password configured, the full auth handshake must succeed for a
/// client that computes the SHA-256 response correctly (Stream Deck plugins
/// store the password the user typed into their settings).
#[test]
fn authed_deck_connect_and_first_action() {
    let (_handle, port, _backend) = start_server(Some("deck-password".into()));
    let mut client = V5Client::connect(port, Some("deck-password"));
    let d = client.ok("GetVersion", serde_json::json!({}));
    assert_eq!(d["obsWebSocketVersion"], "5.0.0");
}

/// Anti-regression: the v4 spellings (`StartRecording`, `ToggleStreaming`, …)
/// must NOT be accepted. Real v5 clients never send them, and silently
/// accepting them would mask a protocol mismatch in the other direction.
#[test]
fn v4_wire_names_are_rejected_as_unknown() {
    let (_handle, port, _backend) = start_server(None);
    let mut client = V5Client::connect(port, None);
    for legacy in [
        "StartRecording",
        "StopRecording",
        "ToggleRecording",
        "StartStreaming",
        "StopStreaming",
        "ToggleStreaming",
    ] {
        let d = client.request(legacy, serde_json::json!({}));
        assert_eq!(
            d["requestStatus"]["code"],
            protocol::status::UNKNOWN_REQUEST_TYPE,
            "v4 name `{legacy}` must be rejected"
        );
    }
}

/// Connect with a short retry loop so parallel test execution cannot flake
/// on the handshake (same rationale as `client_smoke.rs`).
#[allow(dead_code)]
fn connect_with_retry(url: &str) -> WebSocket<MaybeTlsStream<std::net::TcpStream>> {
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        protocol::JSON_SUBPROTOCOL.parse().unwrap(),
    );
    for attempt in 0..10 {
        match connect(request.clone()) {
            Ok((ws, _)) => return ws,
            Err(e) if attempt < 9 => {
                eprintln!("[client] connect attempt {attempt} failed: {e}; retrying");
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("websocket connect: {e}"),
        }
    }
    unreachable!("retry loop always returns or panics")
}

#[allow(dead_code)]
fn unused(_: &WsError) {}
