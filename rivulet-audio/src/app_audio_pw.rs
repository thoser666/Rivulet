//! PipeWire per-application audio capture (issue #154 Phase 4).
//!
//! While Windows per-app capture rides on WASAPI process loopback, the Linux
//! equivalent is a native PipeWire capture stream targeting the application's
//! *sink-input* node: PipeWire routes everything a client plays through a
//! per-stream node in the graph (`media.class = Stream/Output/Audio`), and a
//! capture stream whose `target.object` points at that node receives exactly
//! its audio.
//!
//! Design (mirrors `process_loopback.rs`, the Windows backend):
//! - 48 kHz stereo interleaved f32 — the engine's routed appsrc caps, so no
//!   resampling is needed on either side.
//! - One dedicated capture thread per source with a private PipeWire loop,
//!   context and connection; teardown is signaled via an atomic flag and the
//!   thread exits within one iterate slice.
//! - Enumeration is a registry scan classified by pure helper functions that
//!   are unit-tested without a daemon on every Linux CI run.
//!
//! Identifier convention: a routed Application source stores its capture
//! target as `pid:<n>` on every platform (see `rivulet_core::audio_source`).
//! On Windows `<n>` is the Win32 process id, on Linux it is the PipeWire
//! sink-input *node id* — both are session-scoped handles to "the thing the
//! user picked in the picker", so the engine and persistence layers stay
//! platform-agnostic.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context as _, Result};

use rivulet_core::AudioFrame;

use crate::capture::AudioFrameCallback;

/// Format delivered to the engine: interleaved f32, stereo, 48 kHz — exactly
/// the routed appsrc caps (`audio/x-raw,format=F32LE,...,rate=48000`).
pub const PW_CAPTURE_RATE: u32 = 48_000;
pub const PW_CAPTURE_CHANNELS: u16 = 2;

/// media.class of the nodes that carry what an application plays.
pub const SINK_INPUT_MEDIA_CLASS: &str = "Stream/Output/Audio";

/// PipeWire key holding the node's media class (`PW_KEY_MEDIA_CLASS`).
pub const KEY_MEDIA_CLASS: &str = "media.class";
/// PipeWire key holding the human label (`PW_KEY_NODE_DESCRIPTION`).
pub const KEY_NODE_DESCRIPTION: &str = "node.description";
/// PipeWire key holding the node name (`PW_KEY_NODE_NAME`).
pub const KEY_NODE_NAME: &str = "node.name";
/// PipeWire key holding the application name (`PW_KEY_APP_NAME`).
pub const KEY_APP_NAME: &str = "app.name";

/// One capturable application audio stream offered by the picker.
///
/// Field-compatible with the Windows `process_loopback::AppAudioProcess` so
/// the GUI treats both backends identically: `pid` carries the PipeWire
/// sink-input *node id* (what the capture stream targets) and `name` is the
/// display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppAudioProcess {
    pub pid: u32,
    pub name: String,
}

/// An active per-application capture. Dropping it signals the capture loop;
/// call [`AppAudioCapture::stop`] to also wait for a clean exit.
pub struct AppAudioCapture {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AppAudioCapture {
    /// Start capturing the audio of the PipeWire sink-input node `node_id`
    /// and deliver interleaved f32 frames (48 kHz stereo) to `on_frame` from
    /// a dedicated thread.
    pub fn start(node_id: u32, on_frame: AudioFrameCallback) -> Result<Self> {
        if node_id == 0 {
            bail!("node id 0 (ID_ANY) cannot be captured");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name(format!("rivulet-app-audio-pw{node_id}"))
            .spawn(move || {
                if let Err(err) = capture_thread(node_id, thread_stop, on_frame) {
                    tracing::warn!(node_id, error = %err, "PipeWire per-app audio capture ended with an error");
                }
            })
            .context("failed to spawn the PipeWire per-app audio capture thread")?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    /// Signal the capture loop to stop and wait for it to exit.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AppAudioCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Never join inside drop (the capture thread itself may own the last
        // handle during teardown); the flag alone stops the loop within one
        // iterate slice.
    }
}

// ---------------------------------------------------------------------------
// Pure classification helpers (unit-tested without a daemon)
// ---------------------------------------------------------------------------

/// `true` when the node's `media.class` marks it as a sink-input, i.e. the
/// node carrying what an application plays.
pub fn is_sink_input_media_class(media_class: Option<&str>) -> bool {
    media_class == Some(SINK_INPUT_MEDIA_CLASS)
}

/// Pick the node's display label: prefer `node.description`, then
/// `node.name`, then `app.name`; empty strings count as absent.
pub fn node_display_label(
    node_description: Option<&str>,
    node_name: Option<&str>,
    app_name: Option<&str>,
) -> Option<String> {
    [node_description, node_name, app_name]
        .into_iter()
        .flatten()
        .map(str::to_owned)
        .find(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Capture thread
// ---------------------------------------------------------------------------

fn capture_thread(
    node_id: u32,
    stop: Arc<AtomicBool>,
    mut on_frame: AudioFrameCallback,
) -> Result<()> {
    pipewire::init();

    // Private loop + context + connection: teardown of this capture never
    // disturbs any other PipeWire client (or capture) in the process.
    let mainloop = pipewire::main_loop::MainLoopRc::new(None)
        .context("failed to create the PipeWire main loop")?;
    let context = pipewire::context::ContextRc::new(&mainloop, None)
        .context("failed to create the PipeWire context")?;
    let core = context
        .connect_rc(None)
        .context("failed to connect to the PipeWire daemon")?;

    // Capture from the sink (what the app plays), targeted at its node.
    let props = pipewire::properties::properties! {
        *pipewire::keys::MEDIA_TYPE => "Audio",
        *pipewire::keys::MEDIA_CATEGORY => "Capture",
        *pipewire::keys::MEDIA_ROLE => "Music",
        *pipewire::keys::STREAM_CAPTURE_SINK => "true",
        *pipewire::keys::TARGET_OBJECT => node_id.to_string(),
    };

    let stream = pipewire::stream::StreamBox::new(&core, "rivulet-app-audio", props)
        .context("failed to create the PipeWire capture stream")?;

    // Fixed format matching the engine's routed appsrc caps exactly:
    // F32LE, 48 kHz, stereo, positioned FL/FR.
    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(PW_CAPTURE_RATE);
    audio_info.set_channels(u32::from(PW_CAPTURE_CHANNELS));
    let mut position = [0u32; spa::sys::SPA_AUDIO_MAX_CHANNELS as usize];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    audio_info.set_position(position);

    // SPA_PARAM_EnumFormat object holding the one supported format.
    // (libspa reaches us through pipewire's re-export: `pipewire::spa`.)
    use pipewire::spa;
    let format_object = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let pod_bytes: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(format_object),
    )
    .map(|(cursor, _)| cursor.into_inner())
    .map_err(|err| anyhow::anyhow!("failed to serialize the capture format pod: {err}"))?;

    // The listener must outlive the stream: declare it before `connect` and
    // keep the binding (never `let _ =`, which would drop it immediately).
    let listener = stream
        .add_local_listener::<()>()
        .state_changed(move |_stream, _ud, old, new| {
            if matches!(new, pipewire::stream::StreamState::Error(_)) {
                tracing::warn!(
                    ?old,
                    ?new,
                    node_id,
                    "PipeWire per-app capture stream entered error state"
                );
            }
        })
        .process(move |stream, _ud| {
            // `Buffer` auto-queues itself on drop, so dequeue/copy/let-drop
            // is the whole cycle.
            while let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    continue;
                }
                let data = &mut datas[0];
                if data
                    .chunk()
                    .flags()
                    .contains(pipewire::spa::buffer::ChunkFlags::CORRUPTED)
                {
                    continue;
                }
                let size = data.chunk().size() as usize;
                let Some(bytes) = data.data() else {
                    continue;
                };
                if size == 0 || size > bytes.len() {
                    continue;
                }
                let samples: Vec<f32> = bytes[..size]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect();
                on_frame(AudioFrame::new(
                    samples,
                    PW_CAPTURE_RATE,
                    PW_CAPTURE_CHANNELS,
                ));
            }
        })
        .register()
        .context("failed to register the capture stream listener")?;

    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pipewire::stream::StreamFlags::AUTOCONNECT
                | pipewire::stream::StreamFlags::MAP_BUFFERS
                | pipewire::stream::StreamFlags::RT_PROCESS,
            &mut [pipewire::spa::pod::Pod::from_bytes(&pod_bytes)
                .context("serialized format pod was too small")?],
        )
        .with_context(|| format!("failed to connect the capture stream to node {node_id}"))?;

    // Drive the stream from the private loop; the stop flag (set from the UI
    // thread) is checked between iterate slices, so teardown never waits
    // longer than one slice even when the target goes silent.
    while !stop.load(Ordering::SeqCst) {
        mainloop.loop_().iterate(pipewire::loop_::Timeout::Finite(
            std::time::Duration::from_millis(100),
        ));
    }

    // Explicit, ordered teardown: listener first (removes the callbacks),
    // then disconnect, then the normal drops.
    drop(listener);
    let _ = stream.disconnect();
    Ok(())
}

// ---------------------------------------------------------------------------
// Enumeration
// ---------------------------------------------------------------------------

/// List capturable application audio streams (PipeWire sink-input nodes).
///
/// Connects to the daemon, drains the registry for one roundtrip, and
/// returns the classified sink-inputs in ascending node-id order. If the
/// daemon is unreachable an empty list is returned (with a warning) so the
/// picker simply shows nothing instead of failing.
pub fn list_audio_processes() -> Vec<AppAudioProcess> {
    pipewire::init();

    let Ok(mainloop) = pipewire::main_loop::MainLoopRc::new(None) else {
        tracing::warn!("PipeWire enumeration: main loop unavailable");
        return Vec::new();
    };
    let Ok(context) = pipewire::context::ContextRc::new(&mainloop, None) else {
        tracing::warn!("PipeWire enumeration: context unavailable");
        return Vec::new();
    };
    let Ok(core) = context.connect_rc(None) else {
        tracing::warn!("PipeWire enumeration: daemon unreachable");
        return Vec::new();
    };
    let Ok(registry) = core.get_registry_rc() else {
        tracing::warn!("PipeWire enumeration: registry unavailable");
        return Vec::new();
    };

    let nodes: Arc<std::sync::Mutex<Vec<AppAudioProcess>>> = Default::default();
    let nodes_for_cb = nodes.clone();

    // Collect every sink-input node the registry announces.
    let registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            if !matches!(global.type_, pipewire::types::ObjectType::Node) {
                return;
            }
            let Some(props) = &global.props else {
                return;
            };
            if !is_sink_input_media_class(props.get(KEY_MEDIA_CLASS)) {
                return;
            }
            let Some(name) = node_display_label(
                props.get(KEY_NODE_DESCRIPTION),
                props.get(KEY_NODE_NAME),
                props.get(KEY_APP_NAME),
            ) else {
                return;
            };
            if let Ok(mut guard) = nodes_for_cb.lock() {
                guard.push(AppAudioProcess {
                    pid: global.id,
                    name,
                });
            }
        })
        .register();

    // Standard roundtrip: `sync` queues a core done event that is delivered
    // after every pending global, so quitting the loop on `done` guarantees
    // the registry snapshot is complete.
    let mainloop_for_done = mainloop.clone();
    let core_listener = core
        .add_listener_local()
        .done(move |_id, _seq| mainloop_for_done.quit())
        .register();
    if core.sync(0).is_err() {
        tracing::warn!("PipeWire enumeration: roundtrip sync failed");
    }
    mainloop.run();

    drop(core_listener);
    drop(registry_listener);

    let mut found = nodes
        .lock()
        .map(|mut guard| std::mem::take(&mut *guard))
        .unwrap_or_default();
    found.sort_by_key(|p| p.pid);
    found.dedup();
    found
}

// ---------------------------------------------------------------------------
// Tests (pure helpers only — no PipeWire daemon required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sink_input_classification() {
        assert!(is_sink_input_media_class(Some("Stream/Output/Audio")));
        assert!(!is_sink_input_media_class(Some("Stream/Input/Audio")));
        assert!(!is_sink_input_media_class(Some("Audio/Sink")));
        assert!(!is_sink_input_media_class(Some("Stream/Output/Video")));
        assert!(!is_sink_input_media_class(None));
    }

    #[test]
    fn display_label_prefers_description() {
        assert_eq!(
            node_display_label(Some("Spotify"), Some("node-name"), Some("app")),
            Some("Spotify".to_owned())
        );
        assert_eq!(
            node_display_label(None, Some("node-name"), Some("app")),
            Some("node-name".to_owned())
        );
        assert_eq!(
            node_display_label(None, None, Some("app")),
            Some("app".to_owned())
        );
        assert_eq!(node_display_label(None, None, None), None);
        // Empty strings are treated as absent.
        assert_eq!(node_display_label(Some(""), Some(""), Some("")), None);
    }

    #[test]
    fn capture_rejects_zero_node_id() {
        let (tx, _rx) = std::sync::mpsc::channel::<rivulet_core::AudioFrame>();
        let result = AppAudioCapture::start(
            0,
            Box::new(move |frame| {
                let _ = tx.send(frame);
            }),
        );
        // `is_err` instead of `unwrap_err`: AppAudioCapture holds a thread
        // handle and deliberately does not implement Debug.
        assert!(result.is_err(), "node id 0 (ID_ANY) is uncapturable");
    }

    #[test]
    fn capture_start_never_panics_on_bogus_node() {
        // On a machine with a daemon the connect may even succeed (node ids
        // are reusable); without one the thread logs the failure. Either way
        // `start` must return cleanly and `stop` must join.
        let (tx, _rx) = std::sync::mpsc::channel::<rivulet_core::AudioFrame>();
        let result = AppAudioCapture::start(
            u32::MAX,
            Box::new(move |frame| {
                let _ = tx.send(frame);
            }),
        );
        if let Ok(capture) = result {
            capture.stop();
        }
    }
}
