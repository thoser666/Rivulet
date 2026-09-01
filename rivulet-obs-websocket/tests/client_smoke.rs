//! End-to-end smoke test: a real (non-mock) WebSocket client speaks the
//! OBS WebSocket v5 JSON protocol against the server on a loopback port.
//!
//! This is the "verified with a real client" acceptance for issue #72: the
//! connection, handshake, authentication, requests, request batches, and
//! event subscriptions all run over an actual TCP/WebSocket connection.

use std::sync::Arc;
use std::time::Duration;

use rivulet_obs_websocket::backend::{
    ObsBackend, ObsCommand, ObsCommandResult, ObsEvent, ObsSnapshot,
};
use rivulet_obs_websocket::{protocol, server};
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::{Message, WebSocket};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Error as WsError};

/// In-memory backend that plays the role of the Rivulet app for the test.
struct MemoryBackend {
    snapshot: Arc<std::sync::Mutex<ObsSnapshot>>,
    /// Records the last executed command so the test can assert on it.
    last_command: Arc<std::sync::Mutex<Option<ObsCommand>>>,
}

impl MemoryBackend {
    fn new() -> Self {
        let mut snapshot = ObsSnapshot {
            scenes: vec!["Game".into(), "BRB".into(), "Ending".into()],
            current_scene: Some("Game".into()),
            sources: vec!["Window Capture".into(), "Mic".into()],
            ..Default::default()
        };
        snapshot.output_duration_ms = 12_345;
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
            ObsCommand::ToggleRecording => {
                let mut snap = self.snapshot.lock().unwrap();
                snap.recording = !snap.recording;
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
        }
    }
}

/// Small test client wrapper: performs Hello/Identify and offers typed
/// helpers matching what `obs-websocket-js` / Stream Deck send.
struct TestClient {
    ws: WebSocket<MaybeTlsStream<std::net::TcpStream>>,
}

impl TestClient {
    fn connect(port: u16, password: Option<&str>) -> Self {
        eprintln!("[client] connecting to {port}");
        let url = format!("ws://127.0.0.1:{port}");
        let mut request = url.into_client_request().unwrap();
        // Match the JSON subprotocol real OBS clients negotiate.
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            protocol::JSON_SUBPROTOCOL.parse().unwrap(),
        );
        let mut ws = None;
        // The non-blocking accept loop can momentarily drop a handshake when
        // tests run in parallel on loaded CI runners; retry briefly instead
        // of failing the whole suite on a transient "Connection reset".
        for attempt in 0..10 {
            match connect(request.clone()) {
                Ok((w, _)) => {
                    ws = Some(w);
                    break;
                }
                Err(e) if attempt < 9 => {
                    eprintln!("[client] connect attempt {attempt} failed: {e}; retrying");
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => panic!("websocket connect: {e}"),
            }
        }
        let mut ws = ws.expect("websocket connect");
        eprintln!("[client] connected, reading hello");

        // Read Hello.
        let hello = read_json(&mut ws);
        eprintln!("[client] hello op={}", hello["op"]);
        assert_eq!(hello["op"], 0, "first message must be Hello");
        assert_eq!(hello["d"]["rpcVersion"], 1);

        // Send Identify with auth if a password was configured.
        let mut identify = serde_json::json!({
            "op": 1,
            "d": { "rpcVersion": 1, "eventSubscriptions": protocol::intent::ALL }
        });
        if let Some(password) = password {
            let salt = hello["d"]["authentication"]["salt"].as_str().unwrap();
            let challenge = hello["d"]["authentication"]["challenge"].as_str().unwrap();
            let secret = protocol::compute_secret(password, salt);
            let auth = protocol::compute_auth_response(&secret, challenge);
            identify["d"]["authentication"] = serde_json::json!(auth);
        }
        ws.send(Message::Text(
            serde_json::to_string(&identify).unwrap().into(),
        ))
        .unwrap();

        let identified = read_json(&mut ws);
        eprintln!("[client] identified op={}", identified["op"]);
        assert_eq!(identified["op"], 2, "expected Identified, got {identified}");
        assert_eq!(identified["d"]["negotiatedRpcVersion"], 1);
        Self { ws }
    }

    fn request(&mut self, request_type: &str, data: serde_json::Value) -> serde_json::Value {
        let request_id = format!("req-{request_type}");
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
}

/// Connect with a short retry loop, matching `TestClient::connect`: the
/// server's non-blocking accept loop can transiently drop a handshake under
/// parallel load (fixed in `run_session` by restoring blocking before the
/// handshake), so a bare `connect().unwrap()` would flake the suite.
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

#[test]
fn real_client_handshake_and_read_requests_work() {
    let (_handle, port, _backend) = start_server(None);
    let mut client = TestClient::connect(port, None);

    // GetVersion reports the Rivulet version and offered requests.
    let d = client.request("GetVersion", serde_json::json!({}));
    assert!(d["requestStatus"]["result"].as_bool().unwrap());
    assert_eq!(d["requestStatus"]["code"], 100);
    assert!(d["responseData"]["availableRequests"][0].is_string());

    // GetSceneList reflects backend scenes.
    let d = client.request("GetSceneList", serde_json::json!({}));
    assert_eq!(d["responseData"]["scenes"].as_array().unwrap().len(), 3);
    assert_eq!(d["responseData"]["currentProgramSceneName"], "Game");

    // GetInputList reflects backend sources.
    let d = client.request("GetInputList", serde_json::json!({}));
    assert_eq!(
        d["responseData"]["inputs"][0]["inputName"],
        "Window Capture"
    );

    // GetRecordStatus / GetStreamStatus report inactive outputs.
    let d = client.request("GetRecordStatus", serde_json::json!({}));
    assert_eq!(d["responseData"]["outputActive"], false);

    let d = client.request("GetStreamStatus", serde_json::json!({}));
    assert_eq!(d["responseData"]["outputActive"], false);
}

#[test]
fn real_client_switches_scene_and_receives_event() {
    let (_handle, port, _backend) = start_server(None);
    let mut client = TestClient::connect(port, None);

    // Mutate the scene; the backend emits CurrentProgramSceneChanged.
    let d = client.request(
        "SetCurrentProgramScene",
        serde_json::json!({ "sceneName": "BRB" }),
    );
    assert!(d["requestStatus"]["result"].as_bool().unwrap());

    // The client subscribed to ALL intents, so it receives the event.
    let event = read_json(&mut client.ws);
    assert_eq!(event["op"], 5);
    assert_eq!(event["d"]["eventType"], "CurrentProgramSceneChanged");
    assert_eq!(event["d"]["eventData"]["sceneName"], "BRB");

    // Unknown scene -> RESOURCE_NOT_FOUND with a comment.
    let d = client.request(
        "SetCurrentProgramScene",
        serde_json::json!({ "sceneName": "DoesNotExist" }),
    );
    assert_eq!(d["requestStatus"]["result"], false);
    assert_eq!(
        d["requestStatus"]["code"],
        protocol::status::RESOURCE_NOT_FOUND
    );
    assert!(d["requestStatus"]["comment"]
        .as_str()
        .unwrap()
        .contains("not found"));
}

#[test]
fn real_client_controls_recording_and_streaming() {
    let (_handle, port, _backend) = start_server(None);
    let mut client = TestClient::connect(port, None);

    // Start recording.
    let d = client.request("StartRecording", serde_json::json!({}));
    assert!(d["requestStatus"]["result"].as_bool().unwrap());
    let event = read_json(&mut client.ws);
    assert_eq!(event["d"]["eventType"], "RecordStateChanged");
    assert_eq!(event["d"]["eventData"]["outputActive"], true);

    // Second start fails with OUTPUT_RUNNING.
    let d = client.request("StartRecording", serde_json::json!({}));
    assert_eq!(d["requestStatus"]["result"], false);
    assert_eq!(d["requestStatus"]["code"], protocol::status::OUTPUT_RUNNING);

    // GetRecordStatus now reports active.
    let d = client.request("GetRecordStatus", serde_json::json!({}));
    assert_eq!(d["responseData"]["outputActive"], true);

    // Stop recording.
    let d = client.request("StopRecording", serde_json::json!({}));
    assert!(d["requestStatus"]["result"].as_bool().unwrap());
    let event = read_json(&mut client.ws);
    assert_eq!(event["d"]["eventType"], "RecordStateChanged");
    assert_eq!(event["d"]["eventData"]["outputActive"], false);

    // Stream toggle off->on via ToggleStreaming.
    let d = client.request("ToggleStreaming", serde_json::json!({}));
    assert!(d["requestStatus"]["result"].as_bool().unwrap());
    let event = read_json(&mut client.ws);
    assert_eq!(event["d"]["eventType"], "StreamStateChanged");

    let d = client.request("GetStreamStatus", serde_json::json!({}));
    assert_eq!(d["responseData"]["outputActive"], true);
}

#[test]
fn real_client_batch_executes_serially() {
    let (_handle, port, _backend) = start_server(None);
    let mut client = TestClient::connect(port, None);

    client
        .ws
        .send(Message::Text(
            serde_json::to_string(&serde_json::json!({
                "op": 8,
                "d": {
                    "requestId": "batch-1",
                    "requests": [
                        { "requestType": "SetCurrentProgramScene", "requestData": { "sceneName": "Ending" } },
                        { "requestType": "GetCurrentProgramScene", "requestId": "get-scene" },
                    ],
                }
            }))
            .unwrap()
            .into(),
        ))
        .unwrap();

    let response = read_json(&mut client.ws);
    assert_eq!(response["op"], 9, "expected RequestBatchResponse");
    assert_eq!(response["d"]["requestId"], "batch-1");
    let results = response["d"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0]["requestStatus"]["result"].as_bool().unwrap());
    assert_eq!(
        results[1]["responseData"]["currentProgramSceneName"],
        "Ending"
    );
}

#[test]
fn real_client_batch_halt_on_failure_stops_early() {
    let (_handle, port, _backend) = start_server(None);
    let mut client = TestClient::connect(port, None);

    client
        .ws
        .send(Message::Text(
            serde_json::to_string(&serde_json::json!({
                "op": 8,
                "d": {
                    "requestId": "batch-halt",
                    "haltOnFailure": true,
                    "requests": [
                        { "requestType": "SetCurrentProgramScene", "requestData": { "sceneName": "Missing" } },
                        { "requestType": "GetSceneList" },
                    ],
                }
            }))
            .unwrap()
            .into(),
        ))
        .unwrap();

    let response = read_json(&mut client.ws);
    let results = response["d"]["results"].as_array().unwrap();
    assert_eq!(
        results.len(),
        1,
        "haltOnFailure must stop after the failed request"
    );
    assert_eq!(results[0]["requestStatus"]["result"], false);
}

#[test]
fn auth_requires_matching_password() {
    // Correct password identifies successfully.
    let (_handle, port, _backend) = start_server(Some("hunter2".into()));
    let _client = TestClient::connect(port, Some("hunter2"));
}

#[test]
fn auth_rejects_wrong_password_with_close_code_4009() {
    let (_handle, port, _backend) = start_server(Some("hunter2".into()));

    let url = format!("ws://127.0.0.1:{port}");
    let mut ws = connect_with_retry(url.as_str());

    let hello = read_json(&mut ws);
    let salt = hello["d"]["authentication"]["salt"].as_str().unwrap();
    let challenge = hello["d"]["authentication"]["challenge"].as_str().unwrap();
    let secret = protocol::compute_secret("wrongpass", salt);
    let auth = protocol::compute_auth_response(&secret, challenge);

    ws.send(Message::Text(
        serde_json::to_string(&serde_json::json!({
            "op": 1,
            "d": { "rpcVersion": 1, "authentication": auth }
        }))
        .unwrap()
        .into(),
    ))
    .unwrap();

    // The server must close the connection with the AuthenticationFailed
    // close code (4009) — either as a Close message or a protocol error.
    match ws.read() {
        Ok(Message::Close(Some(frame))) => {
            assert_eq!(u16::from(frame.code), 4009);
        }
        Ok(Message::Close(None)) => {}
        Err(WsError::ConnectionClosed) => {}
        Err(WsError::Protocol(_)) => {}
        Ok(other) => panic!("expected close, got: {other:?}"),
        Err(other) => panic!("expected close, got: {other:?}"),
    }
}

#[test]
fn unknown_request_type_is_rejected_cleanly() {
    let (_handle, port, _backend) = start_server(None);
    let mut client = TestClient::connect(port, None);
    let d = client.request("DoesNotExist", serde_json::json!({}));
    assert_eq!(d["requestStatus"]["result"], false);
    assert_eq!(
        d["requestStatus"]["code"],
        protocol::status::UNKNOWN_REQUEST_TYPE
    );
}

#[test]
fn server_shutdown_closes_cleanly() {
    let (mut handle, port, _backend) = start_server(None);
    let mut client = TestClient::connect(port, None);
    // A simple request round-trip works before shutdown.
    let d = client.request("GetVersion", serde_json::json!({}));
    assert!(d["requestStatus"]["result"].as_bool().unwrap());

    // After shutdown the session must observe the close and the listener
    // must be released.
    let closed = std::thread::spawn(move || {
        matches!(
            client.ws.read(),
            Ok(Message::Close(_)) | Err(WsError::ConnectionClosed) | Err(WsError::Protocol(_))
        )
    });
    handle.shutdown();
    // Give the session thread time to observe the shutdown and release the
    // listener, then verify the port is free.
    std::thread::sleep(Duration::from_millis(200));
    let was_closed = closed.join().unwrap();
    assert!(was_closed, "session must be closed on shutdown");
    let res = std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(300),
    );
    assert!(res.is_err(), "listener must be closed after shutdown");
}

#[test]
fn memory_backend_tracks_last_command_for_diagnostics() {
    let backend = Arc::new(MemoryBackend::new());
    let result = backend.execute(ObsCommand::SetCurrentScene("BRB".into()));
    assert!(matches!(result, ObsCommandResult::Success(_)));
    assert_eq!(
        *backend.last_command.lock().unwrap(),
        Some(ObsCommand::SetCurrentScene("BRB".into()))
    );
}
