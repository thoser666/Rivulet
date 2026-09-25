//! PipeWire device enumeration and device-level capture (issue #231).
//!
//! The Windows slice (issue #229, `device_capture.rs`) made WASAPI endpoints
//! selectable as routed `InputDevice`/`OutputDevice` sources; this module is
//! the Linux mirror: selecting a PipeWire *source* node (microphone, virtual
//! device) as an `InputDevice` source, or a sink *monitor* (what that output
//! device plays — the analog of WASAPI render loopback) as an `OutputDevice`
//! source.
//!
//! Two primitives are exposed to the GUI:
//!
//! - [`list_audio_devices`] — enumerate capturable source nodes and sink
//!   monitors with friendly names and the session-default marker.
//! - [`AudioDeviceCapture`] — stream one node as interleaved f32 PCM at
//!   48 kHz stereo (the engine's routed appsrc caps; the same contract the
//!   per-app path in [`crate::app_audio_pw`] honors) from a dedicated thread
//!   with a private loop/context/connection, so teardown never disturbs any
//!   other capture.
//!
//! Capture targets one node via `target.object = <node id>` (the same
//! pattern the per-app backend uses for sink-inputs); monitor capture
//! additionally sets `stream.capture.sink = true`, which asks the graph for
//! the monitor ports of the targeted sink instead of routing it through the
//! usual loopback path. Node ids are session-scoped (PipeWire recycles them
//! after a daemon restart), so persisted configs that name a vanished node
//! degrade exactly like the Windows path: the picker clears the selection
//! and the legacy placeholders keep working.
//!
//! Device ids follow the `pw-src:<node id>` / `pw-mon:<node id>` convention
//! (see `rivulet_core::DeviceTarget`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context as _, Result};

use rivulet_core::{AudioFrame, DeviceTarget};

use crate::capture::AudioFrameCallback;

/// Format delivered to the engine: interleaved f32, stereo, 48 kHz — exactly
/// the routed appsrc caps (`audio/x-raw,format=F32LE,...,rate=48000`).
pub const PW_CAPTURE_RATE: u32 = 48_000;
pub const PW_CAPTURE_CHANNELS: u16 = 2;

/// media.class of the nodes that carry what an application plays into a
/// device (also used by virtual devices / loopback bridges).
pub const SOURCE_MEDIA_CLASS: &str = "Stream/Input/Audio";
/// media.class of sinks — their monitor ports carry what they play.
pub const SINK_MEDIA_CLASS: &str = "Audio/Sink";

/// PipeWire key holding the node's media class (`PW_KEY_MEDIA_CLASS`).
pub const KEY_MEDIA_CLASS: &str = "media.class";
/// PipeWire key holding the human label (`PW_KEY_NODE_DESCRIPTION`).
pub const KEY_NODE_DESCRIPTION: &str = "node.description";
/// PipeWire key holding the node name (`PW_KEY_NODE_NAME`).
pub const KEY_NODE_NAME: &str = "node.name";
/// PipeWire key holding the application name (`PW_KEY_APP_NAME`).
pub const KEY_APP_NAME: &str = "app.name";

/// Metadata name of the default-routes metadata object
/// (`sm-objects`/`default`), holding `default.audio.sink` /
/// `default.audio.source` as JSON `{"name": ...}`.
pub const KEY_DEFAULT_AUDIO_SINK: &str = "default.audio.sink";
pub const KEY_DEFAULT_AUDIO_SOURCE: &str = "default.audio.source";

/// One capturable device node offered by the device picker.
///
/// Field-compatible with the Windows `device_capture::AudioDeviceInfo` so
/// the GUI treats both backends identically: `endpoint_id` carries the
/// PipeWire node id (what the capture stream targets via `target.object`)
/// and `name` is the display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDeviceInfo {
    /// The PipeWire node id as a decimal string (becomes a `pw-src:` /
    /// `pw-mon:` device id).
    pub endpoint_id: String,
    /// Human-readable node label (`node.description` preferred).
    pub name: String,
    /// The node's `node.name` — the key the default metadata references
    /// (defaults are resolved against this, not the display label).
    pub node_name: String,
    /// Whether this is a sink monitor (output/loopback-like).
    pub is_output: bool,
    /// Whether this node is the session default for its flow.
    pub is_default: bool,
}

impl AudioDeviceInfo {
    /// The `rivulet_core::DeviceTarget`-style device id for this node
    /// (`pw-mon:<id>` for sinks, `pw-src:<id>` for source nodes).
    pub fn device_id(&self) -> String {
        if self.is_output {
            format!("pw-mon:{}", self.endpoint_id)
        } else {
            format!("pw-src:{}", self.endpoint_id)
        }
    }
}

// ---------------------------------------------------------------------------
// Pure classification helpers (unit-tested without a daemon)
// ---------------------------------------------------------------------------

/// `true` when the node's `media.class` marks it as a capturable *source*
/// node (microphones, virtual devices) — the Linux mirror of the per-app
/// backend's sink-input classification.
pub fn is_source_media_class(media_class: Option<&str>) -> bool {
    media_class == Some(SOURCE_MEDIA_CLASS)
}

/// `true` when the node's `media.class` marks it as a sink whose monitor
/// ports carry what it plays.
pub fn is_sink_media_class(media_class: Option<&str>) -> bool {
    media_class == Some(SINK_MEDIA_CLASS)
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

/// Extract the `"name"` value of the metadata default entry (a JSON object
/// like `{"name": "alsa_output.pci-0000_00_1f.3.analog-stereo"}`). Returns
/// [`None`] for absent values, non-JSON payloads, or objects without a
/// non-empty `name` — the default marker is best-effort decoration, never a
/// hard dependency.
pub fn default_name_from_metadata(value: Option<&str>) -> Option<String> {
    let value = value?;
    let value = serde_json::from_str::<serde_json::Value>(value).ok()?;
    let name = value.get("name")?.as_str()?;
    (!name.is_empty()).then(|| name.to_owned())
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

/// An active PipeWire device capture. Dropping it signals the capture loop;
/// call [`AudioDeviceCapture::stop`] to also wait for a clean exit.
pub struct AudioDeviceCapture {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioDeviceCapture {
    /// Start capturing the given PipeWire node (from a parsed
    /// [`DeviceTarget`]) and deliver interleaved f32 frames (48 kHz stereo)
    /// to `on_frame` from a dedicated thread.
    ///
    /// Monitor targets capture what the sink plays (monitor ports); source
    /// targets capture the node directly.
    pub fn start_for_target(target: &DeviceTarget, on_frame: AudioFrameCallback) -> Result<Self> {
        let (node_id, monitor) = match target {
            DeviceTarget::PwMonitor(id) => (*id, true),
            DeviceTarget::PwSource(id) => (*id, false),
            DeviceTarget::Output(_) | DeviceTarget::Input(_) => {
                bail!("target.object capture needs a pw-src:/pw-mon: node id")
            }
        };
        if node_id == 0 {
            bail!("node id 0 (ID_ANY) cannot be captured");
        }
        Self::start(node_id, monitor, on_frame)
    }

    /// Start capturing the node `node_id` (monitor mode when `monitor`)
    /// and deliver frames to `on_frame`.
    pub fn start(node_id: u32, monitor: bool, on_frame: AudioFrameCallback) -> Result<Self> {
        if node_id == 0 {
            bail!("node id 0 (ID_ANY) cannot be captured");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name(format!(
                "rivulet-device-audio-pw{}",
                if monitor { "-mon" } else { "-src" }
            ))
            .spawn(move || {
                if let Err(err) = capture_thread(node_id, monitor, thread_stop, on_frame) {
                    tracing::warn!(node_id, monitor, error = %err, "PipeWire device capture ended with an error");
                }
            })
            .context("failed to spawn the PipeWire device capture thread")?;
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

impl Drop for AudioDeviceCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Never join inside drop (the capture thread itself may own the last
        // handle during teardown); the flag alone stops the loop within one
        // iterate slice.
    }
}

// ---------------------------------------------------------------------------
// Capture thread
// ---------------------------------------------------------------------------

fn capture_thread(
    node_id: u32,
    monitor: bool,
    stop: Arc<AtomicBool>,
    mut on_frame: AudioFrameCallback,
) -> Result<()> {
    pipewire::init();

    // Private loop + context + connection: teardown of this capture never
    // disturbs any other PipeWire client (or capture) in the process —
    // same isolation as the per-app backend.
    let mainloop = pipewire::main_loop::MainLoopRc::new(None)
        .context("failed to create the PipeWire main loop")?;
    let context = pipewire::context::ContextRc::new(&mainloop, None)
        .context("failed to create the PipeWire context")?;
    let core = context
        .connect_rc(None)
        .context("failed to connect to the PipeWire daemon")?;

    let stream_name = if monitor {
        "rivulet-device-monitor"
    } else {
        "rivulet-device-source"
    };
    let mut props = pipewire::properties::properties! {
        *pipewire::keys::MEDIA_TYPE => "Audio",
        *pipewire::keys::MEDIA_CATEGORY => "Capture",
        *pipewire::keys::MEDIA_ROLE => "Music",
        *pipewire::keys::TARGET_OBJECT => node_id.to_string(),
    };
    if monitor {
        // Monitor capture: the graph links the stream to the targeted
        // sink's monitor ports (what it plays) instead of the usual
        // loopback path.
        props.insert(*pipewire::keys::STREAM_CAPTURE_SINK, "true");
    }

    let stream = pipewire::stream::StreamBox::new(&core, stream_name, props)
        .context("failed to create the PipeWire device capture stream")?;

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
                    monitor,
                    "PipeWire device capture stream entered error state"
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

/// List capturable device nodes for the device picker (issue #231): sink
/// monitors first (they map to `OutputDevice` sources), then source nodes;
/// within each flow the session default sorts first. Best-effort: an
/// unreadable node is skipped instead of failing the enumeration, and an
/// unreachable daemon yields an empty list (with a warning) so the picker
/// simply shows nothing.
pub fn list_audio_devices() -> Vec<AudioDeviceInfo> {
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

    let nodes: Arc<std::sync::Mutex<Vec<AudioDeviceInfo>>> = Default::default();
    let nodes_for_cb = nodes.clone();

    // Session defaults live in the `default` metadata object (managed by
    // wireplumber). Binding it is optional decoration: without it the picker
    // works, only without the default marker.
    let default_sink: Arc<std::sync::Mutex<Option<String>>> = Default::default();
    let default_source: Arc<std::sync::Mutex<Option<String>>> = Default::default();
    let metadata_listener: Arc<std::sync::Mutex<Vec<pipewire::metadata::MetadataListener>>> =
        Default::default();

    let registry_for_metadata = registry.clone();
    let default_sink_for_meta = default_sink.clone();
    let default_source_for_meta = default_source.clone();
    let metadata_listener_for_bind = metadata_listener.clone();

    // Collect every capturable node the registry announces, and bind the
    // default metadata object when it shows up.
    let registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            match global.type_ {
                pipewire::types::ObjectType::Node => {
                    let Some(props) = &global.props else {
                        return;
                    };
                    let (is_output, media_ok) = if is_sink_media_class(props.get(KEY_MEDIA_CLASS)) {
                        (true, true)
                    } else if is_source_media_class(props.get(KEY_MEDIA_CLASS)) {
                        (false, true)
                    } else {
                        (false, false)
                    };
                    if !media_ok {
                        return;
                    }
                    let Some(name) = node_display_label(
                        props.get(KEY_NODE_DESCRIPTION),
                        props.get(KEY_NODE_NAME),
                        props.get(KEY_APP_NAME),
                    ) else {
                        return;
                    };
                    let node_name = props.get(KEY_NODE_NAME).unwrap_or_default().to_owned();
                    if let Ok(mut guard) = nodes_for_cb.lock() {
                        guard.push(AudioDeviceInfo {
                            endpoint_id: global.id.to_string(),
                            name,
                            node_name,
                            is_output,
                            is_default: false,
                        });
                    }
                }
                pipewire::types::ObjectType::Metadata => {
                    // Bind the metadata global and install the property
                    // listener from inside the registry callback, like the
                    // pw-mon examples do. The listener is parked in
                    // `metadata_listener` so it survives the closure.
                    if let Ok(metadata) =
                        registry_for_metadata.bind::<pipewire::metadata::Metadata, _>(global)
                    {
                        let sink = default_sink_for_meta.clone();
                        let source = default_source_for_meta.clone();
                        let listener = metadata
                            .add_listener_local()
                            .property(move |_subject, key, _type, value| {
                                match key {
                                    Some(k) if k == KEY_DEFAULT_AUDIO_SINK => {
                                        if let Ok(mut guard) = sink.lock() {
                                            *guard = default_name_from_metadata(value);
                                        }
                                    }
                                    Some(k) if k == KEY_DEFAULT_AUDIO_SOURCE => {
                                        if let Ok(mut guard) = source.lock() {
                                            *guard = default_name_from_metadata(value);
                                        }
                                    }
                                    _ => {}
                                }
                                0
                            })
                            .register();
                        if let Ok(mut guard) = metadata_listener_for_bind.lock() {
                            guard.push(listener);
                        }
                    }
                }
                _ => {}
            }
        })
        .register();

    // Standard roundtrip: `sync` queues a core done event that is delivered
    // after every pending global, so quitting the loop on `done` guarantees
    // the registry snapshot (and the metadata default properties) is
    // complete.
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
    drop(metadata_listener);

    let sink_default = default_sink
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_default();
    let source_default = default_source
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_default();

    let mut found = nodes
        .lock()
        .map(|mut guard| std::mem::take(&mut *guard))
        .unwrap_or_default();
    // Resolve the default markers: the metadata names nodes by `node.name`,
    // matched against the node's own name per flow.
    for device in &mut found {
        let default_name = if device.is_output {
            &sink_default
        } else {
            &source_default
        };
        device.is_default = !default_name.is_empty() && device.node_name == *default_name;
    }
    found.sort_by(|a, b| {
        b.is_output
            .cmp(&a.is_output)
            .then_with(|| b.is_default.cmp(&a.is_default))
            .then_with(|| {
                a.endpoint_id
                    .parse::<u32>()
                    .unwrap_or(0)
                    .cmp(&b.endpoint_id.parse::<u32>().unwrap_or(0))
            })
    });
    found
}

// ---------------------------------------------------------------------------
// Tests (pure helpers only — no PipeWire daemon required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_class_classification() {
        assert!(is_source_media_class(Some("Stream/Input/Audio")));
        assert!(!is_source_media_class(Some("Stream/Output/Audio")));
        assert!(!is_source_media_class(Some("Audio/Sink")));
        assert!(!is_source_media_class(Some("Audio/Source")));
        assert!(!is_source_media_class(None));

        assert!(is_sink_media_class(Some("Audio/Sink")));
        assert!(!is_sink_media_class(Some("Audio/Source")));
        assert!(!is_sink_media_class(Some("Stream/Input/Audio")));
        assert!(!is_sink_media_class(None));
    }

    #[test]
    fn display_label_prefers_description() {
        assert_eq!(
            node_display_label(Some("Built-in Audio"), Some("node-name"), Some("app")),
            Some("Built-in Audio".to_owned())
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
    fn metadata_default_payload_is_parsed() {
        assert_eq!(
            default_name_from_metadata(Some(
                r#"{"name":"alsa_output.pci-0000_00_1f.3.analog-stereo"}"#
            )),
            Some("alsa_output.pci-0000_00_1f.3.analog-stereo".to_owned())
        );
        // Absent, non-JSON, wrong shape, and empty names are no default.
        assert_eq!(default_name_from_metadata(None), None);
        assert_eq!(default_name_from_metadata(Some("")), None);
        assert_eq!(default_name_from_metadata(Some("alsa_output.foo")), None);
        assert_eq!(default_name_from_metadata(Some(r#"{"nick":"x"}"#)), None);
        assert_eq!(default_name_from_metadata(Some(r#"{"name":""}"#)), None);
    }

    #[test]
    fn device_ids_follow_the_pw_convention() {
        let source = AudioDeviceInfo {
            endpoint_id: "42".to_owned(),
            name: "USB Mic".to_owned(),
            node_name: "alsa_input.usb-mic".to_owned(),
            is_output: false,
            is_default: false,
        };
        assert_eq!(source.device_id(), "pw-src:42");
        assert_eq!(
            DeviceTarget::parse(&source.device_id()),
            Some(DeviceTarget::PwSource(42)),
            "picker ids must round-trip through the core convention"
        );

        let sink = AudioDeviceInfo {
            endpoint_id: "7".to_owned(),
            name: "Built-in Speakers".to_owned(),
            node_name: "alsa_output.pci-0000_00_1f.3.analog-stereo".to_owned(),
            is_output: true,
            is_default: true,
        };
        assert_eq!(sink.device_id(), "pw-mon:7");
        assert_eq!(
            DeviceTarget::parse(&sink.device_id()),
            Some(DeviceTarget::PwMonitor(7))
        );
    }

    #[test]
    fn capture_rejects_zero_node_id() {
        let (tx, _rx) = std::sync::mpsc::channel::<rivulet_core::AudioFrame>();
        let result = AudioDeviceCapture::start(
            0,
            false,
            Box::new(move |frame| {
                let _ = tx.send(frame);
            }),
        );
        // `is_err` instead of `unwrap_err`: AudioDeviceCapture holds a thread
        // handle and deliberately does not implement Debug.
        assert!(result.is_err(), "node id 0 (ID_ANY) is uncapturable");
        assert!(AudioDeviceCapture::start(0, true, Box::new(|_| {})).is_err());
    }

    #[test]
    fn capture_rejects_wasapi_targets() {
        // The Windows conventions name endpoints, not nodes — the PipeWire
        // backend cannot resolve them.
        assert!(AudioDeviceCapture::start_for_target(
            &DeviceTarget::Output("{0.0.0.0}.{x}".to_owned()),
            Box::new(|_| {})
        )
        .is_err());
        assert!(AudioDeviceCapture::start_for_target(
            &DeviceTarget::Input("{0.0.1.0}.{y}".to_owned()),
            Box::new(|_| {})
        )
        .is_err());
    }

    #[test]
    fn capture_start_never_panics_on_bogus_node() {
        // On a machine with a daemon the connect may even succeed (node ids
        // are reusable); without one the thread logs the failure. Either way
        // `start` must return cleanly and `stop` must join.
        let (tx, _rx) = std::sync::mpsc::channel::<rivulet_core::AudioFrame>();
        let result = AudioDeviceCapture::start(
            u32::MAX,
            true,
            Box::new(move |frame| {
                let _ = tx.send(frame);
            }),
        );
        if let Ok(capture) = result {
            capture.stop();
        }
    }
}
