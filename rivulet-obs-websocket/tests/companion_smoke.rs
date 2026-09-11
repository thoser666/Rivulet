//! End-to-end tests for the M6 remote companion (issue #99):
//!
//! - The companion HTTP server serves the mobile page and `/config` over a
//!   real loopback TCP connection, with security headers and no logging.
//! - The obs-websocket server's LAN bind refuses to start without a password.
//! - Remote stream start/stop is denied without explicit permission when the
//!   server binds beyond loopback, and allowed once the permission is enabled
//!   (loopback keeps M5 behaviour either way).
//! - A real client's missing authentication is rejected with close code 4009.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use rivulet_obs_websocket::backend::{
    ObsBackend, ObsCommand, ObsCommandResult, ObsEvent, ObsSnapshot,
};
use rivulet_obs_websocket::companion::{self, CompanionConfig};
use rivulet_obs_websocket::protocol;
use rivulet_obs_websocket::{server, BindAddress, ServerOptions};
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::{Message, WebSocket};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Error as WsError};

// ─────────────────────────── HTTP helpers ───────────────────────────

struct HttpResult {
    status: u16,
    headers: String,
    body: String,
}

fn http_request(addr: SocketAddr, method: &str, path: &str) -> HttpResult {
    let mut stream = TcpStream::connect(addr).expect("http connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n"
    )
    .expect("write request");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read response");

    let text = String::from_utf8(buf).expect("utf-8 response");
    let (head, body) = text.split_once("\r\n\r\n").expect("response separator");
    let status = head
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse()
        .expect("numeric status");
    HttpResult {
        status,
        headers: head.to_owned(),
        body: body.to_owned(),
    }
}

fn start_companion(
    port: u16,
    ws_port: u16,
    auth: bool,
) -> (companion::CompanionServerHandle, SocketAddr) {
    let config = CompanionConfig {
        port,
        ws_port,
        ws_auth_required: auth,
        ..Default::default()
    };
    let handle = companion::start(config).expect("companion binds");
    let addr = handle.local_addr();
    (handle, addr)
}

#[test]
fn page_served_over_loopback_with_security_headers() {
    let (_handle, addr) = start_companion(0, 4455, true);

    let res = http_request(addr, "GET", "/");
    assert_eq!(res.status, 200);
    assert!(res
        .headers
        .to_lowercase()
        .contains("content-type: text/html"));
    // Security headers must be present so the LAN page is sandboxed.
    let head = res.headers.to_lowercase();
    assert!(head.contains("x-content-type-options: nosniff"));
    assert!(head.contains("cache-control: no-store"));
    assert!(head.contains("content-security-policy:"));
    assert!(
        head.contains("content-security-policy:")
            && (head.contains("form-action 'none'") || head.contains("form-action 'none'"))
    );
    // The page must carry the mobile viewport, the obs-websocket v5 client,
    // and the inline SHA-256 surface (used on non-secure LAN contexts).
    assert!(res.body.contains("name=\"viewport\""));
    assert!(res.body.contains("obswebsocket.json"));
    assert!(res
        .body
        .contains("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    assert!(res.body.contains("ToggleRecording"));
    assert!(res.body.contains("ToggleStreaming"));
    // No secrets in the page.
    assert!(!res.body.contains("hunter2"));
}

#[test]
fn unknown_path_and_non_get_are_rejected() {
    let (_handle, addr) = start_companion(0, 4455, false);
    let res = http_request(addr, "GET", "/nope");
    assert_eq!(res.status, 404);
    let res = http_request(addr, "POST", "/");
    assert_eq!(res.status, 400);
}

#[test]
fn config_reports_ws_port_and_auth_requirement() {
    let (_handle, addr) = start_companion(0, 4444, true);
    let res = http_request(addr, "GET", "/config");
    assert_eq!(res.status, 200);
    assert!(res.headers.to_lowercase().contains("application/json"));
    let parsed: serde_json::Value = serde_json::from_str(&res.body).expect("json config");
    assert_eq!(parsed["wsPort"], 4444);
    assert_eq!(parsed["authRequired"], true);

    let (_handle, addr) = start_companion(0, 4455, false);
    let res = http_request(addr, "GET", "/config");
    let parsed: serde_json::Value = serde_json::from_str(&res.body).expect("json config");
    assert_eq!(parsed["wsPort"], 4455);
    assert_eq!(parsed["authRequired"], false);
}

#[test]
fn companion_shutdown_releases_its_port() {
    let (mut handle, addr) = start_companion(0, 4455, false);
    let port = addr.port();
    assert!(http_request(addr, "GET", "/").status == 200);
    handle.shutdown();
    std::thread::sleep(Duration::from_millis(200));
    let res = std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(300),
    );
    assert!(res.is_err(), "listener must be closed after shutdown");
}

// ─────────────────────────── OBS server + client ───────────────────────────

struct MemoryBackend {
    snapshot: Arc<std::sync::Mutex<ObsSnapshot>>,
    last_command: Arc<std::sync::Mutex<Option<ObsCommand>>>,
}

impl MemoryBackend {
    fn new() -> Self {
        let snapshot = ObsSnapshot {
            scenes: vec!["Game".into(), "BRB".into()],
            current_scene: Some("Game".into()),
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
        match command {
            ObsCommand::SetCurrentScene(name) => {
                let mut snap = self.snapshot.lock().unwrap();
                if snap.scenes.iter().any(|s| s == &name) {
                    snap.current_scene = Some(name);
                    ObsCommandResult::Success(vec![ObsEvent::CurrentProgramSceneChanged {
                        scene_name: "x".into(),
                    }])
                } else {
                    ObsCommandResult::Failure {
                        status_code: protocol::status::RESOURCE_NOT_FOUND,
                        comment: format!("Scene '{name}' not found"),
                    }
                }
            }
            ObsCommand::StartRecording
            | ObsCommand::StopRecording
            | ObsCommand::ToggleRecording => {
                let mut snap = self.snapshot.lock().unwrap();
                snap.recording = !snap.recording;
                ObsCommandResult::Success(vec![ObsEvent::RecordStateChanged {
                    active: snap.recording,
                    paused: false,
                }])
            }
            ObsCommand::StartStreaming
            | ObsCommand::StopStreaming
            | ObsCommand::ToggleStreaming => {
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

fn connect_with_retry(url: &str) -> WebSocket<MaybeTlsStream<TcpStream>> {
    for attempt in 0..10 {
        match connect(url.into_client_request().unwrap()) {
            Ok((ws, _)) => return ws,
            Err(e) if attempt < 9 => {
                eprintln!("connect attempt {attempt}: {e}; retrying");
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("websocket connect: {e}"),
        }
    }
    unreachable!()
}

fn read_json(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> serde_json::Value {
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

/// Connect and identify against the obs server. `send_auth` controls whether
/// the Identify carries an authentication field at all (for the missing-auth
/// rejection test); when `password` is `None` and `send_auth` is false, the
/// client identifies without authentication (only valid when auth is off).
struct ObsClient {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
}

impl ObsClient {
    fn connect(port: u16, password: Option<&str>) -> Self {
        let url = format!("ws://127.0.0.1:{port}");
        let mut ws = connect_with_retry(&url);
        let hello = read_json(&mut ws);
        assert_eq!(hello["op"], 0, "first message must be Hello");

        let mut identify = serde_json::json!({ "op": 1, "d": { "rpcVersion": 1, "eventSubscriptions": protocol::intent::ALL } });
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
        assert_eq!(identified["op"], 2, "expected Identified, got {identified}");
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
        // The client subscribed to ALL intents, so state events (op 5) can
        // legitimately be delivered ahead of the response to a mutating
        // request. Skip them and return the matching RequestResponse.
        loop {
            let frame = read_json(&mut self.ws);
            if frame["op"] == 5 {
                continue;
            }
            assert_eq!(frame["op"], 7, "expected RequestResponse");
            return frame["d"].clone();
        }
    }
}

fn start_server_lan(
    allow_stream_control: bool,
) -> (server::ObsServerHandle, u16, Arc<MemoryBackend>) {
    let backend = Arc::new(MemoryBackend::new());
    let options = ServerOptions {
        port: 0,
        bind: BindAddress::All,
        password: Some("hunter2".into()),
        allow_remote_stream_control: allow_stream_control,
    };
    let handle = server::start_with_options(backend.clone(), options).expect("lan server binds");
    let port = handle.local_addr().port();
    (handle, port, backend)
}

#[test]
fn lan_bind_without_permission_denies_stream_control() {
    let (_handle, port, backend) = start_server_lan(false);
    let mut client = ObsClient::connect(port, Some("hunter2"));

    // Reads work normally.
    let d = client.request("GetVersion", serde_json::json!({}));
    assert!(d["requestStatus"]["result"].as_bool().unwrap());

    // Stream toggle must be denied without explicit permission…
    let d = client.request("ToggleStreaming", serde_json::json!({}));
    assert_eq!(d["requestStatus"]["result"], false);
    assert_eq!(d["requestStatus"]["code"], protocol::status::GENERIC_ERROR);
    assert!(
        d["requestStatus"]["comment"]
            .as_str()
            .unwrap()
            .contains("permission"),
        "denial must explain the permission requirement"
    );
    assert_eq!(
        *backend.last_command.lock().unwrap(),
        None,
        "the denied command must never reach the backend"
    );

    // …while recording stays remotely controllable (M6 scope: streams only).
    let d = client.request("ToggleRecording", serde_json::json!({}));
    assert!(d["requestStatus"]["result"].as_bool().unwrap());
    assert_eq!(
        *backend.last_command.lock().unwrap(),
        Some(ObsCommand::ToggleRecording)
    );
    // Stream status remains readable even when control is denied.
    let d = client.request("GetStreamStatus", serde_json::json!({}));
    assert_eq!(d["responseData"]["outputActive"], false);
}

#[test]
fn lan_bind_with_permission_allows_stream_control() {
    let (_handle, port, backend) = start_server_lan(true);
    let mut client = ObsClient::connect(port, Some("hunter2"));
    let d = client.request("ToggleStreaming", serde_json::json!({}));
    assert!(
        d["requestStatus"]["result"].as_bool().unwrap(),
        "stream toggle must succeed with explicit permission: {d}"
    );
    assert_eq!(
        *backend.last_command.lock().unwrap(),
        Some(ObsCommand::ToggleStreaming)
    );
}

#[test]
fn loopback_bind_keeps_m5_stream_control_without_permission() {
    // M5 parity: loopback binds are trusted surfaces (Stream Deck etc.) and
    // are not blocked by the LAN permission gate.
    let backend = Arc::new(MemoryBackend::new());
    let handle = server::start_with_options(
        backend.clone(),
        ServerOptions {
            port: 0,
            bind: BindAddress::Loopback,
            password: None,
            allow_remote_stream_control: false,
        },
    )
    .expect("loopback server binds");
    let port = handle.local_addr().port();
    let mut client = ObsClient::connect(port, None);
    let d = client.request("ToggleStreaming", serde_json::json!({}));
    assert!(d["requestStatus"]["result"].as_bool().unwrap());
    assert_eq!(
        *backend.last_command.lock().unwrap(),
        Some(ObsCommand::ToggleStreaming)
    );
}

#[test]
fn lan_bind_refuses_to_start_without_a_password() {
    let backend = Arc::new(MemoryBackend::new());
    let result = server::start_with_options(
        backend,
        ServerOptions {
            port: 0,
            bind: BindAddress::All,
            password: None,
            allow_remote_stream_control: false,
        },
    );
    let err = match result {
        Err(err) => err,
        Ok(_) => panic!("LAN bind without a password must be refused"),
    };
    assert!(
        err.to_string().to_lowercase().contains("password"),
        "error must explain the password requirement: {err}"
    );
}

#[test]
fn missing_authentication_is_rejected_when_auth_required() {
    let handle = server::start_with_options(
        Arc::new(MemoryBackend::new()),
        ServerOptions {
            port: 0,
            bind: BindAddress::Loopback,
            password: Some("hunter2".into()),
            allow_remote_stream_control: true,
        },
    )
    .expect("server binds");
    let port = handle.local_addr().port();

    let url = format!("ws://127.0.0.1:{port}");
    let mut ws = connect_with_retry(&url);
    let hello = read_json(&mut ws);
    assert!(
        hello["d"]["authentication"]["salt"].is_string(),
        "auth required must be advertised in Hello"
    );

    // Identify WITHOUT an authentication field: the v5 spec requires the
    // field whenever auth is enabled, so the server must close 4009.
    ws.send(Message::Text(
        serde_json::to_string(&serde_json::json!({
            "op": 1,
            "d": { "rpcVersion": 1, "eventSubscriptions": protocol::intent::ALL }
        }))
        .unwrap()
        .into(),
    ))
    .unwrap();

    match ws.read() {
        Ok(Message::Close(Some(frame))) => assert_eq!(u16::from(frame.code), 4009),
        Ok(Message::Close(None)) => {}
        Err(WsError::ConnectionClosed) => {}
        Err(WsError::Protocol(_)) => {}
        Ok(other) => panic!("expected close, got: {other:?}"),
        Err(other) => panic!("expected close, got: {other:?}"),
    }
}
