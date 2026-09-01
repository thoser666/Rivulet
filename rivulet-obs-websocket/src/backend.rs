//! Backend abstraction between the OBS WebSocket server and the Rivulet app.
//!
//! The server is protocol-agnostic about the underlying state: it dispatches
//! requests against a [`ObsBackend`] and broadcasts events produced by it.
//! Rivulet implements this trait against its real scene manager / engine, and
//! tests implement it against an in-memory model.

use std::sync::{Arc, Mutex};

use crate::protocol::{intent, RequestType};

/// Snapshot of the state a client can read (scenes, sources, outputs).
#[derive(Debug, Clone, Default)]
pub struct ObsSnapshot {
    /// Scene names in display order.
    pub scenes: Vec<String>,
    /// Name of the current program scene.
    pub current_scene: Option<String>,
    /// Source (input) names.
    pub sources: Vec<String>,
    /// Whether the recording output is active.
    pub recording: bool,
    /// Whether the recording output is paused.
    pub recording_paused: bool,
    /// Whether the streaming output is active.
    pub streaming: bool,
    /// Whether the streaming output is currently reconnecting.
    pub reconnecting: bool,
    /// Total output bytes sent so far (0 when unknown).
    pub output_bytes: u64,
    /// Total output duration in milliseconds (0 when unknown).
    pub output_duration_ms: u64,
    /// Skipped frames (0 when unknown).
    pub skipped_frames: u64,
    /// Total frames (0 when unknown).
    pub total_frames: u64,
}

/// A mutating command a client can issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObsCommand {
    SetCurrentScene(String),
    StartRecording,
    StopRecording,
    ToggleRecording,
    StartStreaming,
    StopStreaming,
    ToggleStreaming,
}

impl ObsCommand {
    /// The request type this command was derived from (for responses).
    pub fn request_type(&self) -> RequestType {
        match self {
            ObsCommand::SetCurrentScene(_) => RequestType::SetCurrentProgramScene,
            ObsCommand::StartRecording => RequestType::StartRecording,
            ObsCommand::StopRecording => RequestType::StopRecording,
            ObsCommand::ToggleRecording => RequestType::ToggleRecording,
            ObsCommand::StartStreaming => RequestType::StartStreaming,
            ObsCommand::StopStreaming => RequestType::StopStreaming,
            ObsCommand::ToggleStreaming => RequestType::ToggleStreaming,
        }
    }
}

/// Result of executing a command: either events to broadcast, or an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObsCommandResult {
    /// The command succeeded. Any events listed here are broadcast to
    /// subscribed clients (e.g. output state changed).
    Success(Vec<ObsEvent>),
    /// The command failed with the given status code and comment.
    Failure { status_code: u16, comment: String },
}

/// Events emitted by the backend when state changes (both request-driven and
/// internally driven — e.g. the user clicking record in the GUI).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ObsEvent {
    /// The current program scene changed.
    CurrentProgramSceneChanged { scene_name: String },
    /// The recording output state changed.
    RecordStateChanged { active: bool, paused: bool },
    /// The streaming output state changed.
    StreamStateChanged { active: bool, reconnecting: bool },
}

impl ObsEvent {
    /// The intent bit clients must subscribe to in order to receive this event.
    pub fn intent(&self) -> u32 {
        match self {
            ObsEvent::CurrentProgramSceneChanged { .. } => intent::SCENES,
            ObsEvent::RecordStateChanged { .. } | ObsEvent::StreamStateChanged { .. } => {
                intent::OUTPUTS
            }
        }
    }

    /// The event type name as sent on the wire.
    pub fn event_type(&self) -> &'static str {
        match self {
            ObsEvent::CurrentProgramSceneChanged { .. } => "CurrentProgramSceneChanged",
            ObsEvent::RecordStateChanged { .. } => "RecordStateChanged",
            ObsEvent::StreamStateChanged { .. } => "StreamStateChanged",
        }
    }

    /// The event data payload.
    pub fn data(&self) -> serde_json::Value {
        match self {
            ObsEvent::CurrentProgramSceneChanged { scene_name } => {
                serde_json::json!({ "sceneName": scene_name })
            }
            ObsEvent::RecordStateChanged { active, paused } => {
                serde_json::json!({ "outputActive": active, "outputPaused": paused })
            }
            ObsEvent::StreamStateChanged {
                active,
                reconnecting,
            } => {
                serde_json::json!({ "outputActive": active, "outputReconnecting": reconnecting })
            }
        }
    }
}

/// The backend contract the websocket server dispatches against.
///
/// Implementations must be cheap to call and safe to invoke from the server's
/// connection threads. Read-only views return [`ObsSnapshot`]; state changes
/// go through [`ObsBackend::execute`].
pub trait ObsBackend: Send + Sync + 'static {
    /// Read the current state snapshot (scenes, sources, output flags).
    fn snapshot(&self) -> ObsSnapshot;

    /// Execute a mutating command. The backend decides success/failure and
    /// returns events that the server should broadcast to subscribed clients.
    fn execute(&self, command: ObsCommand) -> ObsCommandResult;
}

/// Helper used by request handlers to answer with standardized failures.
pub fn missing_request_field(field: &str) -> ObsCommandResult {
    ObsCommandResult::Failure {
        status_code: crate::protocol::status::MISSING_REQUEST_FIELD,
        comment: format!("Parameter: {field}"),
    }
}

pub type SharedBackend = Arc<dyn ObsBackend>;

/// Bridge backend used by the Rivulet GUI.
///
/// Reads are answered from a shared snapshot that the GUI refreshes every
/// frame; commands are forwarded over a channel and the GUI executes them on
/// the UI thread, replying on the per-command response channel.
///
/// This keeps the websocket threads free of GUI state while still letting the
/// server dispatch real mutating commands.
pub struct ChannelBackend {
    snapshot: Arc<Mutex<ObsSnapshot>>,
    commands: std::sync::mpsc::Sender<(ObsCommand, std::sync::mpsc::Sender<ObsCommandResult>)>,
}

impl ChannelBackend {
    pub fn new(
        snapshot: Arc<Mutex<ObsSnapshot>>,
        commands: std::sync::mpsc::Sender<(ObsCommand, std::sync::mpsc::Sender<ObsCommandResult>)>,
    ) -> Self {
        Self { snapshot, commands }
    }
}

impl ObsBackend for ChannelBackend {
    fn snapshot(&self) -> ObsSnapshot {
        self.snapshot.lock().unwrap().clone()
    }

    fn execute(&self, command: ObsCommand) -> ObsCommandResult {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel::<ObsCommandResult>();
        if self.commands.send((command.clone(), reply_tx)).is_err() {
            return ObsCommandResult::Failure {
                status_code: crate::protocol::status::REQUEST_PROCESSING_FAILED,
                comment: "GUI not available".into(),
            };
        }
        // The GUI answers within one frame; allow a generous timeout in case
        // the UI thread is busy (e.g. a file dialog is open).
        match reply_rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(result) => result,
            Err(_) => ObsCommandResult::Failure {
                status_code: crate::protocol::status::REQUEST_PROCESSING_FAILED,
                comment: "GUI did not respond in time".into(),
            },
        }
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;

    #[test]
    fn channel_backend_forwards_commands_and_returns_replies() {
        let snapshot = Arc::new(Mutex::new(ObsSnapshot {
            scenes: vec!["Game".into()],
            ..Default::default()
        }));
        let (commands_tx, commands_rx) = std::sync::mpsc::channel();
        let backend = ChannelBackend::new(snapshot.clone(), commands_tx);

        // A command arrives at the host; the host replies on the response
        // channel.
        let host = std::thread::spawn(move || {
            let (command, reply) = commands_rx.recv().unwrap();
            assert_eq!(command, ObsCommand::ToggleRecording);
            reply
                .send(ObsCommandResult::Success(vec![
                    ObsEvent::RecordStateChanged {
                        active: true,
                        paused: false,
                    },
                ]))
                .unwrap();
        });

        let result = backend.execute(ObsCommand::ToggleRecording);
        assert!(matches!(result, ObsCommandResult::Success(_)));
        host.join().unwrap();
    }

    #[test]
    fn channel_backend_reports_when_host_is_gone() {
        let snapshot = Arc::new(Mutex::new(ObsSnapshot::default()));
        let (commands_tx, _commands_rx) = std::sync::mpsc::channel();
        drop(_commands_rx);
        let backend = ChannelBackend::new(snapshot, commands_tx);
        let result = backend.execute(ObsCommand::StartRecording);
        assert!(matches!(result, ObsCommandResult::Failure { .. }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_request_type_mapping() {
        assert_eq!(
            ObsCommand::SetCurrentScene("x".into()).request_type(),
            RequestType::SetCurrentProgramScene
        );
        assert_eq!(
            ObsCommand::ToggleRecording.request_type(),
            RequestType::ToggleRecording
        );
        assert_eq!(
            ObsCommand::ToggleStreaming.request_type(),
            RequestType::ToggleStreaming
        );
    }

    #[test]
    fn events_carry_correct_intents_and_payloads() {
        let scene = ObsEvent::CurrentProgramSceneChanged {
            scene_name: "Game".into(),
        };
        assert_eq!(scene.intent(), intent::SCENES);
        assert_eq!(scene.event_type(), "CurrentProgramSceneChanged");
        assert_eq!(scene.data(), serde_json::json!({ "sceneName": "Game" }));

        let rec = ObsEvent::RecordStateChanged {
            active: true,
            paused: false,
        };
        assert_eq!(rec.intent(), intent::OUTPUTS);
        assert_eq!(
            rec.data(),
            serde_json::json!({ "outputActive": true, "outputPaused": false })
        );
    }

    #[test]
    fn missing_request_field_reports_parameter() {
        let result = missing_request_field("sceneName");
        assert!(matches!(
            result,
            ObsCommandResult::Failure { status_code, ref comment }
                if status_code == crate::protocol::status::MISSING_REQUEST_FIELD
                    && comment == "Parameter: sceneName"
        ));
    }
}
