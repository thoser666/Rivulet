//! G6 – PipeWire portal screen capture for Linux (Wayland + X11).
//!
//! The implementation lives in `rivulet_core::pipewire_capture` since
//! F-M8-004: `PipeWireBackend` is part of `rivulet-core`'s public API and a
//! crate cannot depend on its own dependent, so the portal session moved to
//! the crate that owns the backend. Re-exported here so the existing
//! `rivulet_capture::pipewire_portal::…` paths and the GUI keep working.

pub use rivulet_core::pipewire_capture::{
    request_portal_session, start_pipewire_capture, PipeWireCaptureHandle, PortalFrame,
    PortalStreamInfo,
};
