//! Windows Graphics Capture (WGC) session, backing
//! [`crate::capture_backend::MetalWgcBackend`].
//!
//! WGC captures through the DWM compositor: the caller names an `HWND` (the
//! window id a [`crate::game_capture::GameWindow`] carries) and the compositor
//! delivers a frame whenever the window's content changes. There is no
//! enumeration here — the window list belongs to the game-capture picker, which
//! already owns one.
//!
//! Frames arrive on WGC's own callback thread and are forwarded through an
//! `mpsc` channel as [`GameCaptureFrame`]s (BGRA8, `width * height * 4` bytes,
//! no row padding).
//!
//! Windows-only module: gated via `#[cfg(target_os = "windows")]` in `lib.rs`,
//! so the workspace still builds on Linux/macOS (CI).

use crate::game_capture::{GameCaptureConfig, GameCaptureFrame};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

/// The WGC frame callback: forwards every delivered frame into the channel and
/// stops the session when the caller drops the capture handle or the window
/// closes.
struct WgcHandler {
    frames: Sender<GameCaptureFrame>,
    stop: Arc<AtomicBool>,
}

impl GraphicsCaptureApiHandler for WgcHandler {
    type Flags = (Sender<GameCaptureFrame>, Arc<AtomicBool>);
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let (frames, stop) = context.flags;
        Ok(Self { frames, stop })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.stop.load(Ordering::SeqCst) {
            capture_control.stop();
            return Ok(());
        }

        // Read the geometry before borrowing the buffer: the `FrameBuffer`
        // borrows the frame mutably, so no immutable access is allowed after.
        let width = frame.width();
        let height = frame.height();

        let frame_buffer = frame.buffer()?;
        // Row padding removed: WGC buffers are aligned per row, so the raw
        // buffer of a window whose width is not a multiple of the alignment
        // would come out skewed. The scratch Vec is only the destination for
        // the packed copy; the frame itself is the Sender's.
        let mut packed = Vec::new();
        let data = frame_buffer.as_nopadding_buffer(&mut packed).to_vec();

        if self
            .frames
            .send(GameCaptureFrame {
                data,
                width,
                height,
            })
            .is_err()
        {
            // The consumer is gone; nothing left to deliver.
            capture_control.stop();
        }

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        tracing::warn!("WGC capture session ended by the system");
        self.stop.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// Start a WGC session for `hwnd`, delivering frames through the returned
/// receiver.
///
/// The second return value is the session's stop flag: setting it (or dropping
/// the capture handle that owns it) ends the session on the next delivered
/// frame. `hwnd` must be a live top-level window handle; `0` is rejected
/// before the compositor is touched, which is what makes the typed failure
/// path testable on a headless runner.
pub(crate) fn start_wgc_capture(
    hwnd: u64,
    config: &GameCaptureConfig,
) -> Result<(Receiver<GameCaptureFrame>, Arc<AtomicBool>), &'static str> {
    if hwnd == 0 {
        return Err("window handle is not valid");
    }

    let (frames, receiver) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_handler = stop.clone();

    // WGC is change-driven, so `fps` cannot pace it; the setting is accepted
    // (and documented) but does not rate-limit the compositor.
    let _ = config;

    let settings = Settings::new(
        Window::from_raw_hwnd(hwnd as *mut _),
        CursorCaptureSettings::Default,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        (frames, stop_for_handler),
    );

    std::thread::spawn(move || {
        tracing::info!("Starting WGC capture session");
        if let Err(err) = WgcHandler::start(settings) {
            tracing::error!("WGC capture session failed: {err}");
        }
        tracing::info!("WGC capture session stopped");
    });

    Ok((receiver, stop))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_handle_is_rejected_as_a_typed_failure() {
        let err = start_wgc_capture(0, &GameCaptureConfig::default())
            .expect_err("a zero HWND must not start a session");
        assert_eq!(err, "window handle is not valid");
    }
}
