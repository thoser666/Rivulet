//! G6 – PipeWire portal screen capture for Linux (Wayland + X11).
//!
//! Uses the xdg-desktop-portal screencast interface via `ashpd` to request
//! a PipeWire stream from the compositor. On Wayland this is the native
//! capture path; on X11 it falls back to the portal as well (the portal
//! handles the X11 → PipeWire bridge internally).
//!
//! # Architecture
//!
//! 1. **Portal session** (async, runs on tokio): creates a screencast session,
//!    selects monitor/window sources, starts the cast, and obtains the
//!    PipeWire file descriptor.
//! 2. **PipeWire loop** (blocking, runs on a dedicated thread): connects to
//!    the PipeWire remote via the fd, creates a video stream on the given
//!    node, and dequeues frames into an `mpsc` channel.

use anyhow::{Context, Result};
use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::thread;

/// Metadata about a screen-cast stream returned by the portal.
#[derive(Debug, Clone)]
pub struct PortalStreamInfo {
    /// PipeWire node ID to connect the stream to.
    pub node_id: u32,
    /// Stream width in pixels (from the portal response).
    pub width: u32,
    /// Stream height in pixels.
    pub height: u32,
}

/// A raw RGBA frame captured from the PipeWire stream.
#[derive(Debug, Clone)]
pub struct PortalFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Request a screencast session via the xdg-desktop-portal and return the
/// PipeWire remote fd plus stream metadata.
///
/// This is an async function — call it from a tokio runtime.
pub async fn request_portal_session(
    prefer_monitor: bool,
) -> Result<(OwnedFd, PortalStreamInfo)> {
    use ashpd::desktop::screencast::{
        CursorMode, Screencast, SelectSourcesOptions, SourceType,
    };
    use ashpd::desktop::PersistMode;

    let proxy = Screencast::new()
        .await
        .context("Failed to connect to xdg-desktop-portal (is a compositor running?)")?;

    let session = proxy
        .create_session(Default::default())
        .await
        .context("Failed to create screencast session")?;

    let source_type = if prefer_monitor {
        SourceType::Monitor.into()
    } else {
        SourceType::Monitor | SourceType::Window
    };

    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Hidden)
                .set_sources(source_type)
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .context("Failed to select screencast sources")?;

    let response = proxy
        .start(&session, None, Default::default())
        .await
        .context("Failed to start screencast")?
        .response()
        .context("Screencast start was rejected by user or compositor")?;

    let stream = response
        .streams()
        .first()
        .context("No stream returned by the screencast portal")?
        .to_owned();

    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .context("Failed to open PipeWire remote")?;

    let (pw, ph) = stream
        .size()
        .map(|s| (s.0 as u32, s.1 as u32))
        .unwrap_or((1920, 1080));
    let info = PortalStreamInfo {
        node_id: stream.pipe_wire_node_id(),
        width: pw,
        height: ph,
    };

    tracing::info!(
        "PipeWire portal session: node={}, {}x{}",
        info.node_id,
        info.width,
        info.height
    );

    Ok((fd, info))
}

/// Spawn a PipeWire capture thread that reads frames from the given node
/// and sends them through the returned channel.
///
/// The thread runs a blocking PipeWire main loop. Dropping the returned
/// `PipeWireCaptureHandle` stops the loop.
pub fn start_pipewire_capture(
    fd: OwnedFd,
    node_id: u32,
) -> (mpsc::Receiver<PortalFrame>, PipeWireCaptureHandle) {
    let (tx, rx) = mpsc::channel();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_clone = stop.clone();

    let handle = PipeWireCaptureHandle { stop };

    thread::spawn(move || {
        if let Err(e) = pipewire_capture_loop(fd, node_id, tx, stop_clone) {
            tracing::error!("PipeWire capture loop ended with error: {e}");
        }
    });

    (rx, handle)
}

/// Handle to a running PipeWire capture session. Dropping stops the capture.
pub struct PipeWireCaptureHandle {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for PipeWireCaptureHandle {
    fn drop(&mut self) {
        self.stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// User data shared between PipeWire callbacks and the capture loop.
struct PipeWireUserData {
    /// Video format negotiated by param_changed.
    format: std::sync::Mutex<Option<spa::param::video::VideoInfoRaw>>,
    /// Stop flag — checked in the process callback.
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Frame sender.
    tx: mpsc::Sender<PortalFrame>,
}

/// Blocking PipeWire main loop — runs on a dedicated thread.
///
/// Uses `MainLoopRc` (shared ownership) so the main loop reference is valid
/// for the entire function lifetime. All PipeWire state is owned by this
/// stack frame.
fn pipewire_capture_loop(
    fd: OwnedFd,
    node_id: u32,
    tx: mpsc::Sender<PortalFrame>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    use pipewire as pw;
    use pw::properties::properties;
    use pw::spa;

    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)
        .context("Failed to create PipeWire main loop")?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .context("Failed to create PipeWire context")?;
    let core = context
        .connect_fd_rc(fd, None)
        .context("Failed to connect to PipeWire remote")?;

    let user_data = PipeWireUserData {
        format: std::sync::Mutex::new(None),
        stop,
        tx,
    };

    let stream = pw::stream::StreamRc::new(
        core.clone(),
        "rivulet-capture",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .context("Failed to create PipeWire stream")?;

    let _listener = stream
        .add_local_listener_with_user_data(user_data)
        .state_changed(|_, _, old, new| {
            tracing::debug!("PipeWire stream state: {:?} -> {:?}", old, new);
        })
        .param_changed(|_, user_data, id, param| {
            let Some(param) = param else {
                return;
            };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }

            let (media_type, media_subtype) =
                match spa::param::format_utils::parse_format(param) {
                    Ok(v) => v,
                    Err(_) => return,
                };

            if media_type != spa::param::format::MediaType::Video
                || media_subtype != spa::param::format::MediaSubtype::Raw
            {
                return;
            }

            let mut video_info = spa::param::video::VideoInfoRaw::default();
            if video_info.parse(param).is_ok() {
                let w = video_info.size().width;
                let h = video_info.size().height;
                tracing::info!("PipeWire video format: {}x{}", w, h);
                *user_data.format.lock().unwrap() = Some(video_info);
            }
        })
        .process(|stream, user_data| {
            if user_data.stop.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }

            match stream.dequeue_buffer() {
                None => {
                    tracing::warn!("PipeWire: no buffers available");
                }
                Some(mut buffer) => {
                    let datas = buffer.datas_mut();
                    if datas.is_empty() {
                        return;
                    }

                    let data = &mut datas[0];
                    let chunk_size = data.chunk().size() as usize;

                    if chunk_size == 0 {
                        return;
                    }

                    // Read frame data from the PipeWire buffer
                    let pixels = match data.data() {
                        Some(slice) => {
                            let len = chunk_size.min(slice.len());
                            slice[..len].to_vec()
                        }
                        None => return,
                    };

                    let (width, height) = {
                        let fmt_lock = user_data.format.lock().unwrap();
                        match fmt_lock.as_ref() {
                            Some(info) => (info.size().width, info.size().height),
                            None => return, // Format not yet negotiated
                        }
                    };

                    let _ = user_data.tx.send(PortalFrame {
                        data: pixels,
                        width,
                        height,
                    });
                }
            }
        })
        .register()
        .context("Failed to register PipeWire stream listener")?;

    // Negotiate video format — accept common formats
    let obj = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            spa::param::format::MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            spa::param::format::MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::RGBx,
            spa::param::video::VideoFormat::RGBA,
            spa::param::video::VideoFormat::BGRA,
            spa::param::video::VideoFormat::RGB,
            spa::param::video::VideoFormat::BGR
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                width: 1920,
                height: 1080
            },
            spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            spa::utils::Rectangle {
                width: 7680,
                height: 4320
            }
        ),
    );

    let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .context("Failed to serialize format negotiation pod")?
    .0
    .into_inner();

    let mut params = [spa::pod::Pod::from_bytes(&values)
        .context("Failed to parse format negotiation pod")?];

    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .context("Failed to connect PipeWire stream")?;

    tracing::info!("PipeWire capture stream connected to node {node_id}");

    mainloop.run();

    tracing::info!("PipeWire capture loop stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_stream_info_is_clone() {
        let info = PortalStreamInfo {
            node_id: 42,
            width: 1920,
            height: 1080,
        };
        let info2 = info.clone();
        assert_eq!(info.node_id, info2.node_id);
        assert_eq!(info.width, info2.width);
        assert_eq!(info.height, info2.height);
    }

    #[test]
    fn portal_frame_is_clone() {
        let frame = PortalFrame {
            data: vec![0u8; 1920 * 1080 * 4],
            width: 1920,
            height: 1080,
        };
        let frame2 = frame.clone();
        assert_eq!(frame.data.len(), frame2.data.len());
    }

    #[test]
    fn pipe_wire_capture_handle_stops_on_drop() {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let handle = PipeWireCaptureHandle {
            stop: stop.clone(),
        };
        assert!(!stop.load(std::sync::atomic::Ordering::Relaxed));
        drop(handle);
        assert!(stop.load(std::sync::atomic::Ordering::Relaxed));
    }
}
