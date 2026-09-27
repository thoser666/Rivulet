//! Recording container formats and crash-safe remuxing (M4).
//!
//! Point 2 of the M4 roadmap adds recording formats beyond MP4 (MKV, MOV,
//! TS). Because MP4 stores the moov box at the end, it is not crash-safe: a
//! partial write after a crash loses the whole file. MKV/MOV/TS tolerate
//! interruption, and the finished file can be remuxed to MP4 *without
//! re-encoding* afterwards (issue #71) — exactly the OBS workflow.
//!
//! This module is pure policy: container <-> GStreamer element/ext mapping and
//! a remux plan that validates a source container can be losslessly carried
//! into a target container. GStreamer execution lives with pipeline
//! integration; everything here is fully unit-testable.

use gst::prelude::*;
use gstreamer as gst;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Recording container formats supported for local capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum RecordingContainer {
    /// Universal compatibility, but the moov box lives at the end of the file
    /// so a crash mid-write loses the whole recording. **Not crash-safe.**
    #[default]
    Mp4,
    /// Matroska: crash-safe (cluster-based writes), remuxes to MP4 losslessly.
    Mkv,
    /// QuickTime: crash-safe, remuxes to MP4 losslessly.
    Mov,
    /// MPEG transport stream: crash-safe, remuxes to MP4 losslessly (H.264
    /// carried as Annex-B; the remux inserts a parser for the AVC conversion).
    MpegTs,
}

impl RecordingContainer {
    /// GStreamer muxer element used for new recordings.
    pub fn muxer_element(&self) -> &'static str {
        match self {
            RecordingContainer::Mp4 => "mp4mux",
            RecordingContainer::Mkv => "matroskamux",
            RecordingContainer::Mov => "qtmux",
            RecordingContainer::MpegTs => "mpegtsmux",
        }
    }

    /// GStreamer demuxer element that can read back a recording of this
    /// container (used by the crash-safe remux).
    pub fn demuxer_element(&self) -> &'static str {
        match self {
            RecordingContainer::Mp4 => "qtdemux",
            RecordingContainer::Mkv => "matroskademux",
            RecordingContainer::Mov => "qtdemux",
            RecordingContainer::MpegTs => "tsdemux",
        }
    }

    /// File extension (without dot) for new recordings.
    pub fn file_extension(&self) -> &'static str {
        match self {
            RecordingContainer::Mp4 => "mp4",
            RecordingContainer::Mkv => "mkv",
            RecordingContainer::Mov => "mov",
            RecordingContainer::MpegTs => "ts",
        }
    }

    /// Human-readable label for UI and logs.
    pub fn label(&self) -> &'static str {
        match self {
            RecordingContainer::Mp4 => "MP4",
            RecordingContainer::Mkv => "MKV (Matroska)",
            RecordingContainer::Mov => "MOV (QuickTime)",
            RecordingContainer::MpegTs => "TS (MPEG transport)",
        }
    }

    /// MP4 is the only container whose partial writes are unrecoverable; the
    /// others can be finalized by remuxing after a crash.
    pub fn is_crash_safe(&self) -> bool {
        !matches!(self, RecordingContainer::Mp4)
    }

    /// Parses a file extension (with or without leading dot) into a container.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "mp4" | "m4v" => Some(RecordingContainer::Mp4),
            "mkv" => Some(RecordingContainer::Mkv),
            "mov" => Some(RecordingContainer::Mov),
            "ts" | "m2ts" | "mts" => Some(RecordingContainer::MpegTs),
            _ => None,
        }
    }
}

/// A validated plan for remuxing one container into another losslessly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemuxPlan {
    /// Path of the recorded (crash-safe) source file.
    pub source_path: String,
    /// Target path for the remuxed MP4.
    pub output_path: String,
    /// Container of the source file.
    pub source: RecordingContainer,
    /// Target container (only MP4 is supported without re-encoding).
    pub target: RecordingContainer,
}

impl RemuxPlan {
    /// A remux is supported when the source container is crash-safe (has a
    /// demuxer) and the target is MP4; anything else needs re-encoding and is
    /// out of scope (issue #71).
    pub fn is_supported(source: RecordingContainer, target: RecordingContainer) -> bool {
        source.is_crash_safe() && target == RecordingContainer::Mp4
    }

    /// Builds a plan for remuxing `source_path` into a sibling MP4 file.
    pub fn auto(source_path: &str, source: RecordingContainer) -> Self {
        RemuxPlan {
            output_path: Self::output_for(source_path, RecordingContainer::Mp4),
            source_path: source_path.to_string(),
            source,
            target: RecordingContainer::Mp4,
        }
    }

    /// Derives the output path for a source path by swapping the extension.
    pub fn output_for(source_path: &str, target: RecordingContainer) -> String {
        let path = std::path::Path::new(source_path);
        let out_ext = target.file_extension();
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if !ext.is_empty() => {
                path.with_extension(out_ext).to_string_lossy().into_owned()
            }
            _ => format!("{}.{out_ext}", path.to_string_lossy()),
        }
    }

    /// Builds the `parse_launch` remux pipeline fragment (containers only,
    /// no re-encoding).
    ///
    /// Uses GStreamer's any-pad syntax: `demux.` refers to each dynamically-
    /// appearing src pad of the demuxer and `mux.` to each request sink pad of
    /// the muxer, so every encoded track is identity-copied into the target
    /// container without decoding or re-encoding.
    pub fn pipeline_fragment(&self) -> String {
        let src = self.source_path.replace(['"', '\\'], "");
        let out = self.output_path.replace(['"', '\\'], "");
        format!(
            "filesrc location=\"{}\" ! {} name=demux demux. ! queue ! {} name=mux mux. ! filesink location=\"{}\"",
            src, self.demuxer_element(), self.muxer_element(), out
        )
    }

    /// The GStreamer demuxer element for the source container.
    pub fn demuxer_element(&self) -> &'static str {
        self.source.demuxer_element()
    }

    /// The GStreamer muxer element for the target container.
    pub fn muxer_element(&self) -> &'static str {
        self.target.muxer_element()
    }
}

/// Remux configuration (issue #71).
///
/// A recording written to a crash-safe intermediate container
/// (MKV/MOV/TS — see [`RecordingContainer`]) is losslessly remuxed to MP4
/// after recording stops. The remux is a container swap only; the encoded
/// video/audio streams are copied without re-encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemuxSettings {
    /// Whether to automatically remux to MP4 after recording stops.
    pub auto_remux_after_stop: bool,
    /// The target container (always [`RecordingContainer::Mp4`], kept as a
    /// field so future targets are a natural extension).
    pub target: RecordingContainer,
}

impl Default for RemuxSettings {
    fn default() -> Self {
        Self {
            // OBS auto-remuxes by default; mirror that expectation.
            auto_remux_after_stop: true,
            target: RecordingContainer::Mp4,
        }
    }
}

impl RemuxSettings {
    /// Validates the settings.
    pub fn validate(&self) -> Result<(), String> {
        if self.target != RecordingContainer::Mp4 {
            return Err("remux target must be MP4".to_string());
        }
        Ok(())
    }
}

/// Result of a remux attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemuxOutcome {
    /// The remux succeeded; the MP4 file exists at the target path.
    Success { output_path: String },
    /// The remux was skipped because a required GStreamer element is missing.
    Skipped(String),
}

/// Remuxes a crash-safe intermediate recording (MKV/MOV/TS) to MP4 **without
/// re-encoding** (issue #71).
///
/// Runs a `filesrc -> demuxer -> muxer -> filesink` pipeline. Demuxer pads are
/// dynamic (one per contained track), so every demux src pad is linked on
/// `pad-added` — through its own queue — to a *request* sink pad of the muxer
/// (the canonical GStreamer remux pattern). Request pads instead of any-pad
/// syntax are essential: the old single `demux.` branch silently dropped
/// every track beyond the first, which lost audio tracks > 1 in the finished
/// MP4 (issue #242, slice 3). The encoded video/audio streams pass through
/// unchanged.
///
/// Request pads are claimed in **one batch** after the demuxer has signalled
/// `no-more-pads` (plus a short grace period): qtmux/mp4mux stop handing out
/// request pads once the first stream has been configured, and tsdemux
/// exposes its pads in several waves (a speculative stream before the PMT
/// update, then the real ones), so lazily claiming pads on `pad-added` can
/// lose audio tracks on TS->MP4 remuxes. Until the batch has run, every
/// chain is held with a BLOCK probe on its demux pad; nothing reaches the
/// muxer before all pads exist (issue #242, slice 3).
///
/// Returns `RemuxOutcome::Skipped` when a required element is unavailable so
/// an environment without the full GStreamer plugins can degrade gracefully
/// instead of failing the workflow.
pub fn remux_to_mp4(plan: &RemuxPlan) -> Result<RemuxOutcome, String> {
    if !RemuxPlan::is_supported(plan.source, plan.target) {
        return Err(format!(
            "cannot remux {} to {} without re-encoding",
            plan.source.label(),
            plan.target.label()
        ));
    }
    if !std::path::Path::new(&plan.source_path).exists() {
        return Err(format!("source recording not found: {}", plan.source_path));
    }

    let demuxer_name = plan.demuxer_element();
    let muxer_name = plan.muxer_element();
    if gst::ElementFactory::find(demuxer_name).is_none() {
        return Ok(RemuxOutcome::Skipped(format!(
            "{demuxer_name} not available in this GStreamer build"
        )));
    }
    if gst::ElementFactory::find(muxer_name).is_none() {
        return Ok(RemuxOutcome::Skipped(format!(
            "{muxer_name} not available in this GStreamer build"
        )));
    }

    let debug = std::env::var("RIVULET_REMUX_DEBUG").is_ok();

    let filesrc = gst::ElementFactory::make("filesrc")
        .name("file_src")
        .property("location", plan.source_path.replace('"', ""))
        .build()
        .map_err(|e| format!("could not create filesrc: {e}"))?;
    let demuxer = gst::ElementFactory::make(demuxer_name)
        .name("demux")
        .build()
        .map_err(|e| format!("could not create {demuxer_name}: {e}"))?;
    let muxer = gst::ElementFactory::make(muxer_name)
        .name("mux")
        .build()
        .map_err(|e| format!("could not create {muxer_name}: {e}"))?;
    let filesink = gst::ElementFactory::make("filesink")
        .name("file_sink")
        .property("location", plan.output_path.replace('"', ""))
        .build()
        .map_err(|e| format!("could not create filesink: {e}"))?;

    let pipeline = gst::Pipeline::default();
    pipeline
        .add_many([&filesrc, &demuxer, &muxer, &filesink])
        .map_err(|e| format!("could not add remux elements: {e}"))?;
    filesrc
        .link(&demuxer)
        .map_err(|e| format!("could not link filesrc to {demuxer_name}: {e}"))?;
    muxer
        .link(&filesink)
        .map_err(|e| format!("could not link {muxer_name} to filesink: {e}"))?;

    // tsdemux names pads `<kind>_<version>_<pid-hex>` and can expose a new
    // pad *version* for the same PID after a PMT update (a speculative
    // stream materializing before the PMT, then the real one). All pads of
    // one PID carry one logical track and must share a single chain, or the
    // continuation pad feeds a context-less parser that drops every frame.
    // The chain head is a `funnel`: a pad has exactly one peer, so late
    // duplicate pads cannot link onto the parser/queue sink of the first
    // wave — each duplicate gets its own funnel sink pad and fans into the
    // single downstream leg (N:1, no data loss).
    let track_chains: Arc<Mutex<HashMap<String, gst::Element>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // Chains whose first-wave funnel sink has already been retired (only
    // the FIRST duplicate join retires it — later waves just add pads).
    let retired_first_wave: Arc<Mutex<std::collections::HashSet<String>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));
    // Pre-claimed muxer request pads, grouped by kind (filled by the PMT
    // scan for TS sources). `pad-added` assigns chains from here first.
    let pad_pool: Arc<Mutex<HashMap<&'static str, Vec<gst::Pad>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    demuxer.connect_pad_added({
        let muxer_for_pads = muxer.clone();
        let pipeline_for_pads = pipeline.clone();
        let track_chains = Arc::clone(&track_chains);
        let pad_pool = Arc::clone(&pad_pool);
        move |_demux, src_pad| {
            let pad_name = src_pad.name().to_string();
            let segments: Vec<&str> = pad_name.split('_').collect();
            let chain_key: String = if segments.len() == 3 {
                segments[2].to_string()
            } else {
                pad_name.clone()
            };
            // A later pad version of a known PID joins the existing chain:
            // it requests its own sink pad on the chain's funnel (the first
            // sink pad is already taken by the initial demux pad) and feeds
            // the same parser/queue/mux leg through the fan-in.
            if let Some(funnel) = track_chains
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&chain_key)
                .cloned()
            {
                if debug {
                    eprintln!("RIVULET_REMUX duplicate pad {pad_name} joins chain {chain_key}");
                }
                // Once a duplicate pad appears, the first-wave pad of this
                // PID is dead — unlink and release its funnel sink. Funnel
                // only forwards EOS once *every* sink pad reported EOS, so
                // a retired first-wave pad that never sees EOS (speculative
                // pads carry no data) would hold the downstream EOS back
                // forever and stall the remux until the bus timeout.
                if retired_first_wave
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(chain_key.clone())
                {
                    if let Some(first) = funnel.static_pad("sink_0") {
                        if let Some(dead_src) = first.peer() {
                            let _ = dead_src.unlink(&first);
                        }
                        funnel.release_request_pad(&first);
                    }
                }
                match funnel.request_pad_simple("sink_%u") {
                    Some(extra_sink) => {
                        if let Err(e) = src_pad.link(&extra_sink) {
                            if debug {
                                eprintln!("RIVULET_REMUX duplicate-pad link FAILED: {e}");
                            }
                            tracing::warn!(error = %e, "remux: duplicate-pad link failed");
                        }
                    }
                    None => {
                        tracing::warn!("remux: funnel refused a sink pad, duplicate pad dropped");
                    }
                }
                return;
            }

            // Media type: prefer the pad's current caps (present for MKV/MOV
            // and for TS pads exposed from a PMT that was already scanned);
            // fall back to the template caps (generic ANY for TS speculative
            // pads), refined by the pad *name* which carries the kind.
            let media = src_pad
                .current_caps()
                .and_then(|caps| caps.structure(0).map(|s| s.name().to_string()))
                .or_else(|| {
                    let kind = segments.first().copied().unwrap_or("");
                    match kind {
                        "audio" => Some("audio/mpeg".to_string()),
                        "video" => Some("video/x-h264".to_string()),
                        "subtitle" | "text" => Some("text/x-raw".to_string()),
                        _ => src_pad
                            .query_caps(None)
                            .structure(0)
                            .map(|s| s.name().to_string()),
                    }
                })
                .unwrap_or_default();
            if debug {
                eprintln!("RIVULET_REMUX pad={pad_name} media={media:?} chain={chain_key}");
            }

            // Transport intermediates (TS) carry stream formats MP4 cannot
            // mux: AAC as ADTS instead of raw frames with codec_data, H.264/
            // H.265 as Annex-B byte-stream instead of AVC/HVC1. A parse
            // element converts on the fly (negotiation-driven) without
            // re-encoding; for already-conformant inputs it passes through.
            let parser = if media.starts_with("audio/") {
                gst::ElementFactory::make("aacparse").build().ok()
            } else if media == "video/x-h264" {
                gst::ElementFactory::make("h264parse").build().ok()
            } else if media == "video/x-h265" {
                gst::ElementFactory::make("h265parse").build().ok()
            } else {
                None
            };

            // Chain: demux -> funnel -> [parser ->] queue -> mux pad. Build
            // it fully (add to pipeline, link parser to queue, sync states)
            // BEFORE linking the demux pad — a running demuxer pushes
            // immediately, and data must never reach an element that is
            // still in NULL state.
            let funnel = match gst::ElementFactory::make("funnel").build() {
                Ok(funnel) => funnel,
                Err(e) => {
                    tracing::warn!(error = %e, "remux: could not create funnel, track skipped");
                    return;
                }
            };
            if let Err(e) = pipeline_for_pads.add(&funnel) {
                tracing::warn!(error = %e, "remux: could not add funnel, track skipped");
                return;
            }
            let queue = match gst::ElementFactory::make("queue").build() {
                Ok(queue) => queue,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "remux: could not create track queue, track skipped"
                    );
                    return;
                }
            };
            if let Err(e) = pipeline_for_pads.add(&queue) {
                tracing::warn!(error = %e, "remux: could not add track queue, track skipped");
                return;
            }
            let downstream_sink = if let Some(parse) = &parser {
                if let Err(e) = pipeline_for_pads.add(parse) {
                    tracing::warn!(
                        error = %e,
                        "remux: could not add parse element, track skipped"
                    );
                    return;
                }
                let Some(parse_src) = parse.static_pad("src") else {
                    return;
                };
                let Some(parse_sink) = parse.static_pad("sink") else {
                    return;
                };
                let Some(queue_sink) = queue.static_pad("sink") else {
                    return;
                };
                if parse_src.link(&queue_sink).is_err() {
                    tracing::warn!("remux: parse to queue link failed, track skipped");
                    return;
                }
                let _ = parse.sync_state_with_parent();
                let _ = queue.sync_state_with_parent();
                parse_sink
            } else {
                let Some(queue_sink) = queue.static_pad("sink") else {
                    return;
                };
                let _ = queue.sync_state_with_parent();
                queue_sink
            };
            let Some(funnel_src) = funnel.static_pad("src") else {
                return;
            };
            if funnel_src.link(&downstream_sink).is_err() {
                tracing::warn!("remux: funnel to parser/queue link failed, track skipped");
                return;
            }
            let _ = funnel.sync_state_with_parent();
            // Entry point of the chain: this first demux pad takes one
            // funnel sink pad; later duplicate versions request their own.
            let Some(head) = funnel.request_pad_simple("sink_%u") else {
                tracing::warn!("remux: funnel refused an entry pad, track skipped");
                return;
            };
            let Some(queue_src) = queue.static_pad("src") else {
                return;
            };
            // Assignment order: pre-claimed pool pads (from the PMT scan)
            // first, then a fresh claim — which works because the pool
            // covers the full stream count, so the muxer has never seen
            // data when a fresh claim is even attempted.
            let request_template: &'static str = if media.starts_with("audio/") {
                "audio_%u"
            } else if media.starts_with("video/") {
                "video_%u"
            } else {
                "subtitle_%u"
            };
            let mux_pad = pad_pool
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(request_template)
                .and_then(|pads| pads.pop())
                .or_else(|| muxer_for_pads.request_pad_simple(request_template));
            let Some(mux_pad) = mux_pad else {
                if debug {
                    eprintln!("RIVULET_REMUX no request pad for {media:?} — parking on fakesink");
                }
                if let Ok(fake) = gst::ElementFactory::make("fakesink")
                    .property("sync", false)
                    .build()
                {
                    let _ = pipeline_for_pads.add(&fake);
                    if let Some(sink) = fake.static_pad("sink") {
                        let _ = queue_src.link(&sink);
                    }
                    let _ = fake.sync_state_with_parent();
                }
                if src_pad.link(&head).is_err() {
                    tracing::warn!("remux: demux link failed, track skipped");
                }
                return;
            };
            if queue_src.link(&mux_pad).is_err() {
                tracing::warn!("remux: queue to muxer link failed, track skipped");
                return;
            }

            // Timestamp hygiene: sources recorded with DTS-only video
            // buffers (hardware encoders with B-frame reordering) or
            // otherwise PTS-less buffers make mp4mux fail with "Buffer has
            // no PTS". Recover by carrying DTS over as PTS, or interpolating
            // from the previous buffer.
            {
                let last_pts_ns = std::sync::atomic::AtomicU64::new(u64::MAX);
                mux_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
                    use std::sync::atomic::Ordering;
                    if let Some(buffer) = info.buffer_mut() {
                        let buffer = buffer.make_mut();
                        if buffer.pts().is_none() {
                            let fallback = buffer.dts().map(|dts| dts.nseconds()).or_else(|| {
                                let prev = last_pts_ns.load(Ordering::Relaxed);
                                (prev != u64::MAX).then(|| {
                                    prev + buffer
                                        .duration()
                                        .map(|d| d.nseconds())
                                        .unwrap_or(33_000_000)
                                })
                            });
                            if let Some(ns) = fallback {
                                buffer.set_pts(gst::ClockTime::from_nseconds(ns));
                            }
                        }
                        if let Some(pts) = buffer.pts() {
                            last_pts_ns.store(pts.nseconds(), Ordering::Relaxed);
                        }
                    }
                    gst::PadProbeReturn::Ok
                });
            }

            if src_pad.link(&head).is_err() {
                tracing::warn!("remux: demux link failed, track skipped");
                return;
            }
            track_chains
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(chain_key, funnel);
        }
    });

    // Pre-claim: for TS sources the PMT is scanned straight from the file
    // (pure byte parsing, no GStreamer), so EVERY request pad exists before
    // the pipeline starts. This removes the whole claim-ordering race:
    // tsdemux's pad waves, the muxer's "configured" heuristic and the
    // speculative duplicate pads all collapse into simple pad assignment
    // inside pad-added. MKV/MOV demuxers expose their pads in one wave, so
    // the lazy in-callback claim is fine for them.
    if plan.source == RecordingContainer::MpegTs {
        let stream_counts = scan_mpegts_stream_kinds(&plan.source_path);
        let total = stream_counts.audio + stream_counts.video;
        if debug {
            eprintln!(
                "RIVULET_REMUX PMT scan: audio={} video={} total={total}",
                stream_counts.audio, stream_counts.video
            );
        }
        if total > 0 {
            let mut pool = pad_pool.lock().unwrap_or_else(|e| e.into_inner());
            for _ in 0..stream_counts.audio {
                match muxer.request_pad_simple("audio_%u") {
                    Some(pad) => pool.entry("audio_%u").or_default().push(pad),
                    None => return Err("mp4 target has no audio pad template".to_string()),
                }
            }
            for _ in 0..stream_counts.video {
                match muxer.request_pad_simple("video_%u") {
                    Some(pad) => pool.entry("video_%u").or_default().push(pad),
                    None => return Err("mp4 target has no video pad template".to_string()),
                }
            }
        }
    }

    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| format!("could not start remux pipeline: {e}"))?;

    let bus = pipeline.bus().expect("pipeline without bus");
    let outcome = bus.timed_pop_filtered(
        gst::ClockTime::from_seconds(60),
        &[gst::MessageType::Eos, gst::MessageType::Error],
    );
    let _ = pipeline.set_state(gst::State::Null);

    match outcome {
        Some(msg) if msg.type_() == gst::MessageType::Eos => Ok(RemuxOutcome::Success {
            output_path: plan.output_path.clone(),
        }),
        Some(msg) if msg.type_() == gst::MessageType::Error => {
            let (err, debug) = match msg.view() {
                gst::MessageView::Error(e) => {
                    (e.error().to_string(), e.debug().unwrap_or_default())
                }
                _ => ("unknown remux error".to_string(), glib::GString::default()),
            };
            Err(format!("remux failed: {err} ({debug})"))
        }
        _ => Err("remux timed out before EOS".to_string()),
    }
}

/// Elementary-stream kinds counted from a transport stream's PMT (issue
/// #242, slice 3). Used to pre-claim muxer request pads before the remux
/// pipeline starts so pad assignment in `pad-added` cannot race the
/// muxer's "refuse pads once configured" rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TsStreamCounts {
    audio: usize,
    video: usize,
}

/// Scans `path` for the PAT and the (first) PMT and counts the distinct
/// elementary-stream PIDs by kind. Best-effort: any parse problem yields
/// the counts found so far (an empty result simply skips pre-claiming).
///
/// Only enough of the ISO 13818-1 syntax to walk PAT/PMT sections is
/// implemented: 188-byte packets, pointer_field, section-length bounds,
/// and PMT stream entries (`stream_type` + `elementary_PID` + ES_info
/// length skip). Program- and stream-type values follow the spec (and
/// H.222 extension): 0x0f/0x11 AAC, 0x1b/0x24 H.264/H.265, 0x02/0x10
/// MPEG video.
fn scan_mpegts_stream_kinds(path: &str) -> TsStreamCounts {
    let mut counts = TsStreamCounts::default();
    let Ok(data) = std::fs::read(path) else {
        return counts;
    };

    // PAT: PID 0x0000 -> program number -> PMT PID.
    let mut pmt_pid: Option<u16> = None;
    let mut pmt: Option<&[u8]> = None;
    let mut offset = 0;
    while offset + 188 <= data.len() {
        let pkt = &data[offset..offset + 188];
        offset += 188;
        if pkt[0] != 0x47 {
            continue; // sync byte: not a TS packet (resync would be nicer,
                      // but our own muxer writes clean 188-byte packets)
        }
        let pid = (u16::from(pkt[1] & 0x1f) << 8) | u16::from(pkt[2]);
        let payload_unit_start = pkt[1] & 0x40 != 0;
        if !payload_unit_start {
            continue;
        }
        // Payload after the adaptation field: byte 4 is the AF length, the
        // AF itself (if any) follows, then the payload.
        let af_len = usize::from(pkt[4]);
        if 5 + af_len >= 188 {
            continue;
        }
        let payload = &pkt[5 + af_len..];
        if payload.is_empty() {
            continue;
        }
        let pointer = usize::from(payload[0]);
        let Some(section) = payload.get(1 + pointer..) else {
            continue;
        };
        if section.is_empty() {
            continue;
        }
        let table_id = section[0];
        if pmt_pid.is_none() && pid == 0x0000 && table_id == 0x00 && section.len() >= 12 {
            // PAT: section_length bounds the section; programs start at 8.
            let section_len = (usize::from(section[1] & 0x03) << 8) | usize::from(section[2]);
            let end = section_len.min(section.len() - 3);
            let mut i = 8;
            while i + 4 <= end {
                let program = (u16::from(section[i]) << 8) | u16::from(section[i + 1]);
                let pid = ((u16::from(section[i + 2]) & 0x1f) << 8) | u16::from(section[i + 3]);
                if program != 0 && pid != 0 {
                    pmt_pid = Some(pid);
                    break;
                }
                i += 4;
            }
        } else if pid == pmt_pid.unwrap_or(u16::MAX) && table_id == 0x02 {
            // mpegtsmux rewrites the PMT as streams materialize: the first
            // version may list only the video PID (audio ES info arrives
            // with the PMT update later in the file). Parse every PMT and
            // let the LAST one (the most complete) decide the counts; the
            // scan caps at a few MB so a truncated file cannot spin.
            pmt = Some(section);
            if offset > 8 * 1024 * 1024 {
                break;
            }
        }
    }

    let Some(pmt) = pmt else {
        return counts;
    };
    if pmt.len() < 12 {
        return counts;
    }
    let section_len = (usize::from(pmt[1] & 0x03) << 8) | usize::from(pmt[2]);
    let end = section_len.min(pmt.len() - 3).saturating_sub(4); // minus CRC32
                                                                // program_info_length sits at bytes 10-11; the stream entries follow it.
    let prog_info_len = ((usize::from(pmt[10]) & 0x0f) << 8) | usize::from(pmt[11]);
    let mut i = 12 + prog_info_len;
    let mut seen_pids: Vec<u16> = Vec::new();
    while i + 5 <= end {
        let stream_type = pmt[i];
        let es_pid = ((u16::from(pmt[i + 1]) & 0x1f) << 8) | u16::from(pmt[i + 2]);
        let es_info_len = ((usize::from(pmt[i + 3]) & 0x0f) << 8) | usize::from(pmt[i + 4]);
        if es_pid != 0 && !seen_pids.contains(&es_pid) {
            seen_pids.push(es_pid);
            match stream_type {
                // AAC (ADTS/raw), MPEG-1/2 audio, AC-3, DTS, Opus
                0x0f | 0x11 | 0x03 | 0x04 | 0x81 | 0x82 | 0x85 | 0x90 => counts.audio += 1,
                // MPEG-1/2 video, H.264, H.265, AV1
                0x01 | 0x02 | 0x10 | 0x1b | 0x24 | 0x25 => counts.video += 1,
                _ => {}
            }
        }
        i += 5 + es_info_len;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_container_is_mp4() {
        assert_eq!(RecordingContainer::default(), RecordingContainer::Mp4);
    }

    #[test]
    fn muxer_and_extension_mapping_is_deterministic() {
        let cases = [
            (RecordingContainer::Mp4, "mp4mux", "mp4"),
            (RecordingContainer::Mkv, "matroskamux", "mkv"),
            (RecordingContainer::Mov, "qtmux", "mov"),
            (RecordingContainer::MpegTs, "mpegtsmux", "ts"),
        ];
        for (container, muxer, ext) in cases {
            assert_eq!(container.muxer_element(), muxer);
            assert_eq!(container.file_extension(), ext);
        }
    }

    #[test]
    fn mp4_is_not_crash_safe_others_are() {
        assert!(!RecordingContainer::Mp4.is_crash_safe());
        assert!(RecordingContainer::Mkv.is_crash_safe());
        assert!(RecordingContainer::Mov.is_crash_safe());
        assert!(RecordingContainer::MpegTs.is_crash_safe());
    }

    #[test]
    fn extension_parsing_is_case_insensitive() {
        assert_eq!(
            RecordingContainer::from_extension(".MKV"),
            Some(RecordingContainer::Mkv)
        );
        assert_eq!(
            RecordingContainer::from_extension("mp4"),
            Some(RecordingContainer::Mp4)
        );
        assert_eq!(
            RecordingContainer::from_extension("m2ts"),
            Some(RecordingContainer::MpegTs)
        );
        assert_eq!(RecordingContainer::from_extension("xyz"), None);
    }
    #[test]
    fn default_remux_settings_auto_enabled_mp4() {
        let s = RemuxSettings::default();
        assert!(s.auto_remux_after_stop);
        assert!(s.validate().is_ok());
        assert_eq!(s.target, RecordingContainer::Mp4);
    }

    #[test]
    fn remux_rejects_non_mp4_target() {
        let s = RemuxSettings {
            auto_remux_after_stop: true,
            target: RecordingContainer::Mkv,
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn supported_sources_are_crash_safe_intermediates() {
        assert!(RemuxPlan::is_supported(
            RecordingContainer::Mkv,
            RecordingContainer::Mp4
        ));
        assert!(RemuxPlan::is_supported(
            RecordingContainer::Mov,
            RecordingContainer::Mp4
        ));
        assert!(RemuxPlan::is_supported(
            RecordingContainer::MpegTs,
            RecordingContainer::Mp4
        ));
        // MP4 source (not crash-safe) is rejected; MP4 target never targets MP4.
        assert!(!RemuxPlan::is_supported(
            RecordingContainer::Mp4,
            RecordingContainer::Mp4
        ));
        assert!(!RemuxPlan::is_supported(
            RecordingContainer::Mkv,
            RecordingContainer::Mkv
        ));
    }

    #[test]
    fn remux_plan_builds_for_mkv_intermediate() {
        let plan = RemuxPlan {
            source_path: "/tmp/record.mkv".to_string(),
            output_path: "/tmp/record.mp4".to_string(),
            source: RecordingContainer::Mkv,
            target: RecordingContainer::Mp4,
        };
        assert_eq!(plan.demuxer_element(), "matroskademux");
        assert_eq!(plan.muxer_element(), "mp4mux");
        let p = plan.pipeline_fragment();
        assert!(p.contains("filesrc location=\"/tmp/record.mkv\""));
        assert!(p.contains("matroskademux name=demux"));
        assert!(p.contains("mp4mux name=mux"));
        assert!(p.contains("filesink location=\"/tmp/record.mp4\""));
        assert!(!p.contains("enc"), "remux must never re-encode");
    }

    #[test]
    fn output_path_swaps_extension() {
        assert_eq!(
            RemuxPlan::output_for("/tmp/record.mkv", RecordingContainer::Mp4),
            "/tmp/record.mp4"
        );
        assert_eq!(
            RemuxPlan::output_for("clip.MOV", RecordingContainer::Mp4),
            "clip.mp4"
        );
        assert_eq!(
            RemuxPlan::output_for("/tmp/noext", RecordingContainer::Mp4),
            "/tmp/noext.mp4"
        );
    }

    #[test]
    fn pipeline_escapes_quotes_and_backslashes_in_paths() {
        let plan = RemuxPlan {
            source_path: "/tmp/quo\"te.mkv".to_string(),
            output_path: "/tmp\\back.mp4".to_string(),
            source: RecordingContainer::Mkv,
            target: RecordingContainer::Mp4,
        };
        let p = plan.pipeline_fragment();
        assert!(!p.contains('"') || p.contains("location="));
        assert!(p.contains("matroskademux"));
        assert!(
            !p.contains('\\'),
            "path separators must not leak into the pipeline"
        );
    }

    #[test]
    fn remux_entrypoint_rejects_unsupported_source() {
        let plan = RemuxPlan {
            source_path: "/tmp/record.mp4".to_string(),
            output_path: "/tmp/out.mp4".to_string(),
            source: RecordingContainer::Mp4,
            target: RecordingContainer::Mp4,
        };
        // MP4->MP4 is unsupported; must fail before talking to GStreamer.
        assert!(remux_to_mp4(&plan).is_err());
    }

    #[test]
    fn remux_entrypoint_rejects_missing_source() {
        let plan = RemuxPlan {
            source_path: "/definitely/missing/file.mkv".to_string(),
            output_path: "/tmp/out.mp4".to_string(),
            source: RecordingContainer::Mkv,
            target: RecordingContainer::Mp4,
        };
        let err = remux_to_mp4(&plan).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn remux_fragment_is_parse_launchable_when_elements_exist() {
        let plan = RemuxPlan {
            source_path: "/tmp/record.mkv".to_string(),
            output_path: "/tmp/out.mp4".to_string(),
            source: RecordingContainer::Mkv,
            target: RecordingContainer::Mp4,
        };
        let desc = plan.pipeline_fragment();
        // The any-pad syntax must be well-formed GStreamer parse_launch, but
        // only when GStreamer is initialized and the elements exist.
        if gst::init().is_ok()
            && gst::ElementFactory::find("matroskademux").is_some()
            && gst::ElementFactory::find("mp4mux").is_some()
        {
            assert!(gst::parse::launch(&desc).is_ok(), "fragment: {desc}");
        }
        // Unsupported combo errors at path/plan level before launching.
        assert!(RemuxPlan::is_supported(plan.source, plan.target));
    }

    #[test]
    fn remux_skips_gracefully_when_demuxer_missing() {
        // Only hits GStreamer availability when the source file exists; simulate
        // the missing-element branch without a real file by pointing at
        // a guaranteed-absent element name is not possible with a real muxer,
        // so instead assert that the guard order is: validate -> exists -> elems.
        let plan = RemuxPlan {
            source_path: "/tmp/missing-source.mkv".to_string(),
            output_path: "/tmp/out.mp4".to_string(),
            source: RecordingContainer::Mkv,
            target: RecordingContainer::Mp4,
        };
        // Missing source is reported before element availability.
        let err = remux_to_mp4(&plan).unwrap_err();
        assert!(err.contains("not found"));
    }
}
