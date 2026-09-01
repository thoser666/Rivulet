//! OBS WebSocket v5-compatible server (JSON subprotocol).
//!
//! Runs a small TCP listener on `127.0.0.1`, accepts WebSocket connections,
//! performs the v5 Hello/Identify handshake (optionally with SHA-256
//! challenge/response authentication), and dispatches requests against a
//! [`crate::backend::ObsBackend`]. Events produced by the backend are
//! broadcast to all identified clients that subscribed to the relevant
//! intent.

use std::collections::HashMap;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rand::random as random_bytes;
use tungstenite::protocol::frame::Utf8Bytes;
use tungstenite::protocol::{CloseFrame, Message};
use tungstenite::{Error as WsError, WebSocket};

use crate::backend::{ObsCommand, ObsCommandResult, ObsEvent, ObsSnapshot, SharedBackend};
use crate::protocol::{self, RequestType};

/// Session functions return the (large) tungstenite error boxed so the hot
/// path stays small; `?` works via the standard `From<E> for Box<E>` impl.
type SessionResult<T> = Result<T, Box<WsError>>;

/// Default TCP port (matches the OBS WebSocket default).
pub const DEFAULT_PORT: u16 = 4455;

/// How long a fresh connection may wait for its `Identify` message.
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);

/// A running server. Dropping or calling [`ObsServerHandle::shutdown`] stops
/// the listener and closes all sessions.
pub struct ObsServerHandle {
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    addr: SocketAddr,
    state: Arc<ServerState>,
}

impl ObsServerHandle {
    /// The address the listener is bound to (useful with port 0 in tests).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Request the server to stop accepting and close all sessions.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        // Ask every live session to close; this releases our listener once
        // the session threads (which hold a clone of the server state) exit.
        let sessions = self.state.sessions.lock().unwrap();
        for session in sessions.values() {
            let _ = session.tx.send(SessionMessage::Shutdown);
        }
    }

    /// Broadcast events to all identified sessions (GUI-initiated state
    /// changes, e.g. the user switching scenes in the window rather than via
    /// a remote request). Subscription masks are honored per session.
    pub fn broadcast(&self, events: Vec<ObsEvent>) {
        let sessions = self.state.sessions.lock().unwrap();
        for session in sessions.values() {
            for event in &events {
                let _ = session.tx.send(SessionMessage::Event(event.clone()));
            }
        }
    }
}

impl Drop for ObsServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Shared server state.
struct ServerState {
    backend: SharedBackend,
    /// Password to challenge against (None = auth disabled).
    password: Option<Arc<str>>,
    shutdown: Arc<AtomicBool>,
    /// Registered identified sessions for event broadcast.
    sessions: Mutex<HashMap<u64, Session>>,
    next_session_id: AtomicU64,
}

/// Messages pushed to a session from other threads (broadcast + shutdown).
enum SessionMessage {
    Event(ObsEvent),
    Shutdown,
}

struct Session {
    tx: std::sync::mpsc::Sender<SessionMessage>,
}

/// A connected, identified WebSocket session.
struct SessionThread {
    ws: WebSocket<TcpStream>,
    backend: SharedBackend,
    state: Arc<ServerState>,
    subscription_mask: u32,
}

/// Start the OBS WebSocket server on `127.0.0.1:port`.
///
/// `password` enables authentication: every connection receives a challenge
/// and is closed with `AUTHENTICATION_FAILED` (4009) if the `Identify`
/// response does not match.
pub fn start(
    backend: SharedBackend,
    password: Option<String>,
    port: u16,
) -> io::Result<ObsServerHandle> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let state = Arc::new(ServerState {
        backend,
        password: password.map(Arc::from),
        shutdown: shutdown.clone(),
        sessions: Mutex::new(HashMap::new()),
        next_session_id: AtomicU64::new(1),
    });

    tracing::info!(%addr, auth = state.password.is_some(), "obs-webSocket server listening");

    let thread_state = state.clone();
    let thread = thread::Builder::new()
        .name("obs-websocket-accept".into())
        .spawn(move || accept_loop(listener, thread_state))
        .map_err(io::Error::other)?;

    Ok(ObsServerHandle {
        shutdown,
        thread: Some(thread),
        addr,
        state,
    })
}

fn accept_loop(listener: TcpListener, state: Arc<ServerState>) {
    loop {
        if state.shutdown.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nodelay(true);
                let state = state.clone();
                let _ = thread::Builder::new()
                    .name(format!(
                        "obs-ws-session-{}",
                        state.next_session_id.fetch_add(1, Ordering::SeqCst)
                    ))
                    .spawn(move || {
                        if let Err(err) = run_session(stream, state) {
                            tracing::debug!(error = %err, "obs-websocket session ended");
                        }
                    });
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // No pending connection: sleep briefly so a shutdown request
                // can be observed (the listener is non-blocking).
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(_) => continue,
        }
    }
}

/// Handle one WebSocket connection: Hello, Identify (auth + RPC), then loop
/// reading requests until the client disconnects.
fn run_session(stream: TcpStream, state: Arc<ServerState>) -> SessionResult<()> {
    // The accept loop uses a non-blocking listener for clean shutdown; on
    // some platforms (Windows in particular) the accepted socket *inherits*
    // that mode. A non-blocking socket breaks tungstenite's handshake read
    // with an intermittent WouldBlock, so the client saw
    // `Protocol(HandshakeIncomplete)` under parallel test load. Restore
    // blocking **before** the handshake — not after, which was too late.
    let _ = stream.set_nonblocking(false);
    let mut ws = match tungstenite::accept_hdr(
        stream,
        #[allow(clippy::result_large_err)]
        |req: &tungstenite::handshake::server::Request,
         mut resp: tungstenite::handshake::server::Response| {
            // Advertise the JSON subprotocol that real OBS clients negotiate; if
            // the client did not request it, continue without one (lenient).
            let requested = req
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .split(',')
                .map(str::trim)
                .any(|p| p == protocol::JSON_SUBPROTOCOL);
            if requested {
                resp.headers_mut().insert(
                    "sec-websocket-protocol",
                    protocol::JSON_SUBPROTOCOL.parse().unwrap(),
                );
            }
            Ok(resp)
        },
    ) {
        Ok(ws) => {
            // Belt and braces: also restore blocking on the upgraded socket
            // (already done pre-handshake above, but harmless to repeat for
            // platforms where the WebSocket wrapper changed the handle).
            let _ = ws.get_ref().set_nonblocking(false);
            ws
        }
        Err(err) => {
            return Err(match err {
                tungstenite::handshake::HandshakeError::Failure(f) => Box::new(f),
                other => Box::new(WsError::Io(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    format!("websocket handshake failed: {other:?}"),
                ))),
            })
        }
    };

    let hello_auth = if let Some(password) = state.password.clone() {
        let salt = random_b64(16);
        let challenge = random_b64(24);
        let hello = protocol::envelope(
            protocol::op::HELLO,
            serde_json::json!({
                "obsStudioVersion": rivulet_version(),
                "obsWebSocketVersion": "5.0.0",
                "rpcVersion": protocol::RPC_VERSION,
                "authentication": { "challenge": challenge, "salt": salt },
            }),
        );
        ws.send(text(hello))?;
        Some((password, salt, challenge))
    } else {
        let hello = protocol::envelope(
            protocol::op::HELLO,
            serde_json::json!({
                "obsStudioVersion": rivulet_version(),
                "obsWebSocketVersion": "5.0.0",
                "rpcVersion": protocol::RPC_VERSION,
            }),
        );
        ws.send(text(hello))?;
        None
    };

    let (subscription_mask, negotiated_rpc) = match recv_identify(&mut ws, hello_auth.as_ref())? {
        Some(identify) => identify,
        None => {
            let _ = ws.close(Some(CloseFrame {
                code: protocol::close::NOT_IDENTIFIED.into(),
                reason: Utf8Bytes::from("Identify timeout"),
            }));
            return Err(session_error(
                protocol::close::NOT_IDENTIFIED,
                "Identify timeout",
            ));
        }
    };

    ws.send(text(protocol::envelope(
        protocol::op::IDENTIFIED,
        serde_json::json!({ "negotiatedRpcVersion": negotiated_rpc }),
    )))?;

    let id = state.next_session_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::channel::<SessionMessage>();
    state.sessions.lock().unwrap().insert(id, Session { tx });

    let mut session = SessionThread {
        ws,
        backend: state.backend.clone(),
        state: state.clone(),
        subscription_mask,
    };

    let result = session.loop_forever(rx, &state.shutdown);
    state.sessions.lock().unwrap().remove(&id);
    result
}

impl SessionThread {
    fn loop_forever(
        &mut self,
        rx: std::sync::mpsc::Receiver<SessionMessage>,
        shutdown: &Arc<AtomicBool>,
    ) -> SessionResult<()> {
        loop {
            if shutdown.load(Ordering::SeqCst) && rx.try_recv().is_err() {
                break;
            }
            // Deliver messages queued while we were processing.
            if self.drain_events(&rx) {
                // A Shutdown message was observed.
                break;
            }

            // Poll with a short read timeout so broadcast/shutdown messages
            // are not starved by a silent connection.
            match recv_with_timeout(&mut self.ws, Duration::from_secs(1))? {
                Some(Message::Text(text)) => self.handle_text(&text)?,
                Some(Message::Close(_)) => return Ok(()),
                Some(Message::Ping(payload)) => {
                    self.ws.send(Message::Pong(payload))?;
                }
                Some(Message::Binary(_)) => {
                    return self.close_with(
                        protocol::close::MESSAGE_DECODE_ERROR,
                        "JSON subprotocol only",
                    );
                }
                Some(Message::Pong(_)) | Some(Message::Frame(_)) => {}
                None => {}
            }
            if self.drain_events(&rx) {
                break;
            }
        }
        Ok(())
    }

    /// Deliver queued session messages. Returns `true` if a shutdown was
    /// observed (the session should end).
    fn drain_events(&mut self, rx: &std::sync::mpsc::Receiver<SessionMessage>) -> bool {
        let mut shutdown = false;
        while let Ok(message) = rx.try_recv() {
            match message {
                SessionMessage::Shutdown => shutdown = true,
                SessionMessage::Event(event) => {
                    if event.intent() & self.subscription_mask == 0 {
                        continue;
                    }
                    let msg = protocol::envelope(
                        protocol::op::EVENT,
                        serde_json::json!({
                            "eventType": event.event_type(),
                            "eventIntent": event.intent(),
                            "eventData": event.data(),
                        }),
                    );
                    let _ = self.ws.send(text(msg));
                }
            }
        }
        shutdown
    }

    fn handle_text(&mut self, text: &str) -> SessionResult<()> {
        let value: serde_json::Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => {
                return self.close_with(protocol::close::MESSAGE_DECODE_ERROR, "Invalid JSON")
            }
        };
        let op = match value.get("op").and_then(serde_json::Value::as_u64) {
            Some(op) => op as u8,
            None => return self.close_with(protocol::close::UNKNOWN_OP_CODE, "Missing op"),
        };
        let data = value.get("d").cloned().unwrap_or(serde_json::Value::Null);
        match op {
            protocol::op::REQUEST => self.handle_request(data),
            protocol::op::REQUEST_BATCH => self.handle_request_batch(data),
            protocol::op::REIDENTIFY => {
                if let Some(mask) = data
                    .get("eventSubscriptions")
                    .and_then(serde_json::Value::as_u64)
                {
                    self.subscription_mask = mask as u32;
                }
                Ok(())
            }
            protocol::op::IDENTIFY => {
                self.close_with(protocol::close::ALREADY_IDENTIFIED, "Already identified")
            }
            _ => self.close_with(protocol::close::UNKNOWN_OP_CODE, "Unknown op"),
        }
    }

    fn handle_request(&mut self, data: serde_json::Value) -> SessionResult<()> {
        let Some(request_type) = data.get("requestType").and_then(serde_json::Value::as_str) else {
            return self.respond_status(
                None,
                None,
                protocol::status::MISSING_REQUEST_TYPE,
                "Missing requestType",
            );
        };
        let request_id = data
            .get("requestId")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let request_data = data
            .get("requestData")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let Some(rt) = RequestType::from_str(request_type).ok() else {
            return self.respond_status(
                request_id.as_deref(),
                Some(request_type),
                protocol::status::UNKNOWN_REQUEST_TYPE,
                "Unknown request type",
            );
        };
        let (result, response_data) = self.run_one(rt, request_data);
        self.send_response(rt.as_str(), request_id.as_deref(), result, response_data)
    }

    fn handle_request_batch(&mut self, data: serde_json::Value) -> SessionResult<()> {
        let request_id = data
            .get("requestId")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let halt_on_failure = data
            .get("haltOnFailure")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let requests = data
            .get("requests")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut results = Vec::with_capacity(requests.len());
        for request in requests {
            let raw_type = request
                .get("requestType")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let inner_id = request
                .get("requestId")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned);
            let inner_data = request
                .get("requestData")
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            let (result, response_data) = match RequestType::from_str(raw_type).ok() {
                Some(rt) => self.run_one(rt, inner_data),
                None => (
                    ObsCommandResult::Failure {
                        status_code: protocol::status::UNKNOWN_REQUEST_TYPE,
                        comment: "Unknown request type".into(),
                    },
                    serde_json::Value::Null,
                ),
            };
            let failed = matches!(result, ObsCommandResult::Failure { .. });
            results.push(serde_json::json!({
                "requestType": raw_type,
                "requestId": inner_id,
                "requestStatus": status_json(&result),
                "responseData": response_data,
            }));
            if failed && halt_on_failure {
                break;
            }
        }

        self.ws.send(text(protocol::envelope(
            protocol::op::REQUEST_BATCH_RESPONSE,
            serde_json::json!({ "requestId": request_id, "results": results }),
        )))?;
        Ok(())
    }

    /// Execute one request. Returns (result, responseData).
    fn run_one(
        &mut self,
        rt: RequestType,
        request_data: serde_json::Value,
    ) -> (ObsCommandResult, serde_json::Value) {
        // Read-only requests are answered from the snapshot.
        if rt.is_read_only() {
            return self.read_response(rt);
        }
        match rt {
            RequestType::SetCurrentProgramScene => {
                let scene_name = match request_data
                    .get("sceneName")
                    .and_then(serde_json::Value::as_str)
                {
                    Some(name) => name.to_owned(),
                    None => {
                        return (
                            crate::backend::missing_request_field("sceneName"),
                            serde_json::Value::Null,
                        )
                    }
                };
                (
                    self.run_command(ObsCommand::SetCurrentScene(scene_name)),
                    serde_json::Value::Null,
                )
            }
            RequestType::StartRecording => (
                self.run_command(ObsCommand::StartRecording),
                serde_json::Value::Null,
            ),
            RequestType::StopRecording => (
                self.run_command(ObsCommand::StopRecording),
                serde_json::Value::Null,
            ),
            RequestType::ToggleRecording => (
                self.run_command(ObsCommand::ToggleRecording),
                serde_json::Value::Null,
            ),
            RequestType::StartStreaming => (
                self.run_command(ObsCommand::StartStreaming),
                serde_json::Value::Null,
            ),
            RequestType::StopStreaming => (
                self.run_command(ObsCommand::StopStreaming),
                serde_json::Value::Null,
            ),
            RequestType::ToggleStreaming => (
                self.run_command(ObsCommand::ToggleStreaming),
                serde_json::Value::Null,
            ),
            _ => (
                ObsCommandResult::Failure {
                    status_code: protocol::status::UNKNOWN_REQUEST_TYPE,
                    comment: "Not implemented".into(),
                },
                serde_json::Value::Null,
            ),
        }
    }

    fn run_command(&mut self, command: ObsCommand) -> ObsCommandResult {
        let result = self.backend.execute(command.clone());
        if let ObsCommandResult::Success(events) = &result {
            if !events.is_empty() {
                self.broadcast(events.clone());
            }
        }
        result
    }

    /// Broadcast events to every identified session (including this one; its
    /// subscription mask is honored again in `drain_events`).
    fn broadcast(&self, events: Vec<ObsEvent>) {
        let sessions = self.state.sessions.lock().unwrap();
        for session in sessions.values() {
            for event in &events {
                let _ = session.tx.send(SessionMessage::Event(event.clone()));
            }
        }
    }

    fn read_response(&self, rt: RequestType) -> (ObsCommandResult, serde_json::Value) {
        let snapshot = self.backend.snapshot();
        let data = read_response_data(rt, &snapshot);
        (ObsCommandResult::Success(Vec::new()), data)
    }

    fn send_response(
        &mut self,
        request_type: &str,
        request_id: Option<&str>,
        result: ObsCommandResult,
        response_data: serde_json::Value,
    ) -> SessionResult<()> {
        self.ws.send(text(protocol::envelope(
            protocol::op::REQUEST_RESPONSE,
            serde_json::json!({
                "requestType": request_type,
                "requestId": request_id,
                "requestStatus": status_json(&result),
                "responseData": response_data,
            }),
        )))?;
        Ok(())
    }

    fn respond_status(
        &mut self,
        request_id: Option<&str>,
        request_type: Option<&str>,
        code: u16,
        comment: &str,
    ) -> SessionResult<()> {
        self.ws.send(text(protocol::envelope(
            protocol::op::REQUEST_RESPONSE,
            serde_json::json!({
                "requestType": request_type,
                "requestId": request_id,
                "requestStatus": { "result": false, "code": code, "comment": comment },
            }),
        )))?;
        Ok(())
    }

    fn close_with(&mut self, code: u16, reason: &str) -> SessionResult<()> {
        let _ = self.ws.close(Some(CloseFrame {
            code: code.into(),
            reason: Utf8Bytes::from(reason),
        }));
        Err(session_error(code, reason))
    }
}

impl RequestType {
    fn is_read_only(self) -> bool {
        matches!(
            self,
            RequestType::GetVersion
                | RequestType::GetAuthRequired
                | RequestType::GetSceneList
                | RequestType::GetCurrentProgramScene
                | RequestType::GetInputList
                | RequestType::GetRecordStatus
                | RequestType::GetStreamStatus
        )
    }
}

/// Response payload for read-only requests per the v5 protocol reference.
fn read_response_data(rt: RequestType, snapshot: &ObsSnapshot) -> serde_json::Value {
    match rt {
        RequestType::GetVersion => serde_json::json!({
            "obsVersion": rivulet_version(),
            "obsWebSocketVersion": "5.0.0",
            "rpcVersion": protocol::RPC_VERSION,
            "availableRequests": [
                "GetVersion", "GetAuthRequired", "GetSceneList",
                "GetCurrentProgramScene", "SetCurrentProgramScene",
                "GetInputList", "StartRecording", "StopRecording",
                "ToggleRecording", "GetRecordStatus", "StartStreaming",
                "StopStreaming", "ToggleStreaming", "GetStreamStatus",
            ],
        }),
        RequestType::GetAuthRequired => serde_json::json!({ "authRequired": false }),
        RequestType::GetSceneList => serde_json::json!({
            "currentProgramSceneName": snapshot.current_scene,
            "currentPreviewSceneName": serde_json::Value::Null,
            "scenes": snapshot
                .scenes
                .iter()
                .enumerate()
                .map(|(i, name)| serde_json::json!({
                    "sceneName": name,
                    "sceneIndex": i,
                }))
                .collect::<Vec<_>>(),
        }),
        RequestType::GetCurrentProgramScene => serde_json::json!({
            "currentProgramSceneName": snapshot.current_scene,
        }),
        RequestType::GetInputList => serde_json::json!({
            "inputs": snapshot
                .sources
                .iter()
                .map(|name| serde_json::json!({
                    "inputName": name,
                    "inputKind": "rivulet_source",
                }))
                .collect::<Vec<_>>(),
        }),
        RequestType::GetRecordStatus => serde_json::json!({
            "outputActive": snapshot.recording,
            "outputPaused": snapshot.recording_paused,
            "outputTimecode": timecode(snapshot.output_duration_ms),
            "outputDuration": snapshot.output_duration_ms,
            "outputBytes": snapshot.output_bytes,
        }),
        RequestType::GetStreamStatus => serde_json::json!({
            "outputActive": snapshot.streaming,
            "outputReconnecting": snapshot.reconnecting,
            "outputTimecode": timecode(snapshot.output_duration_ms),
            "outputDuration": snapshot.output_duration_ms,
            "outputCongestion": 0,
            "outputBytes": snapshot.output_bytes,
            "outputSkippedFrames": snapshot.skipped_frames,
            "outputTotalFrames": snapshot.total_frames,
        }),
        _ => serde_json::Value::Null,
    }
}

/// Render a duration in milliseconds as HH:MM:SS.mmm (or HH:MM:SS.mmm with up
/// to three fraction digits).
fn timecode(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let millis = ms % 1000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

/// JSON form of a command result.
fn status_json(result: &ObsCommandResult) -> serde_json::Value {
    match result {
        ObsCommandResult::Success(_) => {
            serde_json::json!({ "result": true, "code": protocol::status::SUCCESS })
        }
        ObsCommandResult::Failure {
            status_code,
            comment,
        } => {
            serde_json::json!({ "result": false, "code": status_code, "comment": comment })
        }
    }
}

fn text(value: serde_json::Value) -> Message {
    Message::Text(Utf8Bytes::from(
        serde_json::to_string(&value).unwrap_or_default(),
    ))
}

/// Poll the socket for the Identify message with a timeout.
fn recv_identify(
    ws: &mut WebSocket<TcpStream>,
    auth: Option<&(Arc<str>, String, String)>,
) -> SessionResult<Option<(u32, u32)>> {
    let deadline = std::time::Instant::now() + IDENTIFY_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        let msg = match recv_with_timeout(ws, remaining)? {
            Some(msg) => msg,
            None => return Ok(None),
        };
        match msg {
            Message::Text(text) => match parse_identify(&text, auth) {
                Ok(identify) => return Ok(Some(identify)),
                Err(close_code) => {
                    let _ = ws.close(Some(CloseFrame {
                        code: close_code.into(),
                        reason: Utf8Bytes::from("Identify rejected"),
                    }));
                    return Err(session_error(close_code, "Identify rejected"));
                }
            },
            Message::Ping(payload) => {
                ws.send(Message::Pong(payload))?;
            }
            Message::Close(_) => return Ok(None),
            _ => {
                let _ = ws.close(Some(CloseFrame {
                    code: protocol::close::MESSAGE_DECODE_ERROR.into(),
                    reason: Utf8Bytes::from("Expected Identify"),
                }));
                return Err(session_error(
                    protocol::close::MESSAGE_DECODE_ERROR,
                    "Expected Identify",
                ));
            }
        }
    }
}

fn recv_with_timeout(
    ws: &mut WebSocket<TcpStream>,
    timeout: Duration,
) -> SessionResult<Option<Message>> {
    // tungstenite's `read` blocks until a message arrives; to enforce a
    // timeout we set a read timeout on the underlying stream and interpret
    // WouldBlock as "no message yet".
    let old = ws.get_ref().read_timeout().ok().flatten();
    ws.get_ref()
        .set_read_timeout(Some(timeout))
        .map_err(|e| Box::new(WsError::Io(e)))?;
    let result = ws.read();
    let _ = ws.get_ref().set_read_timeout(old);
    match result {
        Ok(msg) => Ok(Some(msg)),
        Err(WsError::Io(e))
            if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
        {
            Ok(None)
        }
        Err(e) => Err(Box::new(e)),
    }
}

/// A session error carrying the OBS close code; used internally, only logged.
fn session_error(code: u16, reason: &str) -> Box<WsError> {
    Box::new(WsError::Io(io::Error::new(
        io::ErrorKind::ConnectionAborted,
        format!("obs-websocket closed with {code}: {reason}"),
    )))
}

fn random_b64(bytes: usize) -> String {
    use base64::Engine;
    let buf: Vec<u8> = match bytes {
        16 => random_bytes::<[u8; 16]>().to_vec(),
        24 => random_bytes::<[u8; 24]>().to_vec(),
        n => {
            // General fallback for other sizes: generate in 24-byte chunks.
            let mut buf = Vec::with_capacity(n);
            while buf.len() < n {
                buf.extend_from_slice(&random_bytes::<[u8; 24]>());
            }
            buf.truncate(n);
            buf
        }
    };
    base64::engine::general_purpose::STANDARD.encode(&buf)
}

fn rivulet_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Parse an Identify message, validating auth (if enabled) and RPC version.
fn parse_identify(
    text: &str,
    auth: Option<&(Arc<str>, String, String)>,
) -> Result<(u32, u32), u16> {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Err(protocol::close::MESSAGE_DECODE_ERROR),
    };
    if value.get("op").and_then(serde_json::Value::as_u64) != Some(protocol::op::IDENTIFY as u64) {
        return Err(protocol::close::NOT_IDENTIFIED);
    }
    let data = value.get("d").ok_or(protocol::close::MISSING_DATA_FIELD)?;

    let rpc = data
        .get("rpcVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or(protocol::close::UNSUPPORTED_RPC_VERSION)? as u32;
    if rpc > protocol::RPC_VERSION {
        return Err(protocol::close::UNSUPPORTED_RPC_VERSION);
    }

    if let Some((password, salt, challenge)) = auth {
        let provided = data
            .get("authentication")
            .and_then(serde_json::Value::as_str)
            .ok_or(protocol::close::AUTHENTICATION_FAILED)?;
        if !crate::protocol::verify_authentication(password, salt, challenge, provided) {
            return Err(protocol::close::AUTHENTICATION_FAILED);
        }
    }

    let mask = data
        .get("eventSubscriptions")
        .and_then(serde_json::Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(protocol::intent::ALL);
    Ok((mask, rpc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;

    #[test]
    fn parse_identify_rejects_wrong_op() {
        let auth: Option<(StdArc<str>, String, String)> = None;
        let err = parse_identify(r#"{"op":6,"d":{"rpcVersion":1}}"#, auth.as_ref()).unwrap_err();
        assert_eq!(err, protocol::close::NOT_IDENTIFIED);
    }

    #[test]
    fn parse_identify_accepts_authed_subscription_mask() {
        let password: StdArc<str> = "secret".into();
        let salt = "c2FsdA==".to_string();
        let challenge = "Y2hhbGxlbmdl".to_string();
        let secret = crate::protocol::compute_secret(&password, &salt);
        let response = crate::protocol::compute_auth_response(&secret, &challenge);
        let auth = Some((password, salt, challenge));
        let text = format!(
            r#"{{"op":1,"d":{{"rpcVersion":1,"authentication":"{response}","eventSubscriptions":4}}}}"#
        );
        let (mask, rpc) = parse_identify(&text, auth.as_ref()).unwrap();
        assert_eq!(mask, 4);
        assert_eq!(rpc, 1);
    }

    #[test]
    fn parse_identify_rejects_bad_auth() {
        let password: StdArc<str> = "secret".into();
        let auth = Some((password, "c2FsdA==".to_string(), "Y2hhbGxlbmdl".to_string()));
        let text = r#"{"op":1,"d":{"rpcVersion":1,"authentication":"d3Jvbmc="}}"#;
        assert_eq!(
            parse_identify(text, auth.as_ref()).unwrap_err(),
            protocol::close::AUTHENTICATION_FAILED
        );
    }

    #[test]
    fn parse_identify_rejects_unsupported_rpc_version() {
        let auth: Option<(StdArc<str>, String, String)> = None;
        let text = r#"{"op":1,"d":{"rpcVersion":99}}"#;
        assert_eq!(
            parse_identify(text, auth.as_ref()).unwrap_err(),
            protocol::close::UNSUPPORTED_RPC_VERSION
        );
    }

    #[test]
    fn parse_identify_defaults_subscriptions_to_all() {
        let auth: Option<(StdArc<str>, String, String)> = None;
        let text = r#"{"op":1,"d":{"rpcVersion":1}}"#;
        let (mask, rpc) = parse_identify(text, auth.as_ref()).unwrap();
        assert_eq!(mask, protocol::intent::ALL);
        assert_eq!(rpc, 1);
    }

    #[test]
    fn random_b64_is_variable_width_base64() {
        let a = random_b64(16);
        let b = random_b64(24);
        assert_eq!(a.len(), 24);
        assert_eq!(b.len(), 32);
        assert_ne!(a, b);
    }

    #[test]
    fn timecode_formats_hh_mm_ss_mmm() {
        assert_eq!(timecode(0), "00:00:00.000");
        assert_eq!(timecode(1_234), "00:00:01.234");
        assert_eq!(timecode(3_661_000), "01:01:01.000");
    }

    #[test]
    fn read_response_data_renders_scenes_and_outputs() {
        let snapshot = ObsSnapshot {
            scenes: vec!["Game".into(), "BRB".into()],
            current_scene: Some("Game".into()),
            sources: vec!["Window Capture".into()],
            recording: true,
            recording_paused: false,
            streaming: false,
            reconnecting: false,
            output_bytes: 42,
            output_duration_ms: 1_000,
            skipped_frames: 1,
            total_frames: 30,
        };
        let scenes = read_response_data(RequestType::GetSceneList, &snapshot);
        assert_eq!(scenes["scenes"][0]["sceneName"], "Game");
        assert_eq!(scenes["scenes"][0]["sceneIndex"], 0);
        assert_eq!(scenes["scenes"][1]["sceneName"], "BRB");
        assert_eq!(scenes["currentProgramSceneName"], "Game");

        let inputs = read_response_data(RequestType::GetInputList, &snapshot);
        assert_eq!(inputs["inputs"][0]["inputName"], "Window Capture");

        let rec = read_response_data(RequestType::GetRecordStatus, &snapshot);
        assert_eq!(rec["outputActive"], true);
        assert_eq!(rec["outputTimecode"], "00:00:01.000");

        let stream = read_response_data(RequestType::GetStreamStatus, &snapshot);
        assert_eq!(stream["outputActive"], false);
        assert_eq!(stream["outputSkippedFrames"], 1);
    }
}
