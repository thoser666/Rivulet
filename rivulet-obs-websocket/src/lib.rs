//! OBS WebSocket v5-compatible remote-control server for Rivulet.
//!
//! Provides a small, self-contained server that speaks the OBS WebSocket v5
//! (JSON) protocol — the wire format used by Stream Deck / TouchPortal
//! integrations and tools like `obs-websocket-js`. The server is
//! protocol-focused: application state is supplied through the
//! [`backend::ObsBackend`] trait, so it stays testable against an in-memory
//! model and can be bridged to real Rivulet state by the GUI.
//!
//! Supported surface (see [`protocol::RequestType`]):
//! - Authentication (SHA-256 challenge/response, opt-in via password)
//! - Scenes: `GetSceneList`, `GetCurrentProgramScene`, `SetCurrentProgramScene`
//! - Sources: `GetInputList`
//! - Recording: `StartRecording`, `StopRecording`, `ToggleRecording`, `GetRecordStatus`
//! - Streaming: `StartStreaming`, `StopStreaming`, `ToggleStreaming`, `GetStreamStatus`
//! - Events: `CurrentProgramSceneChanged`, `RecordStateChanged`, `StreamStateChanged`
//! - Version queries: `GetVersion`, `GetAuthRequired`
//!
//! The wire format is verified against the generated protocol reference
//! (<https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>)
//! and exercised end-to-end in
//! `tests/client_smoke.rs` with a real (non-mock) WebSocket client.

pub mod backend;
pub mod protocol;
pub mod server;

pub use backend::{
    ChannelBackend, ObsBackend, ObsCommand, ObsCommandResult, ObsEvent, ObsSnapshot,
};
pub use server::{start, ObsServerHandle, DEFAULT_PORT};
