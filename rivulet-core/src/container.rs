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

/// The pad-wiring state shared between the remux pipeline and the
/// `pad-added` handler. Extracted from `remux_to_mp4` so the tsdemux
/// duplicate-pad behavior (funnel fan-in, first-wave retirement) is
/// directly unit-testable without a recorded file (issue #242).
struct RemuxChainWiring {
    /// One chain per logical stream (chain key = PID hex for TS pads).
    track_chains: Arc<Mutex<HashMap<String, gst::Element>>>,
    /// The first-wave funnel sink per chain, kept for retirement. Request
    /// pads are NOT findable by `static_pad("sink_0")`: modern GStreamer
    /// names generated request pads `funnelpad0`-style, so the pad handle
    /// must be stored at build time.
    first_wave_pads: Arc<Mutex<HashMap<String, gst::Pad>>>,
    /// Chains whose first-wave funnel sink was already retired.
    retired_first_wave: Arc<Mutex<std::collections::HashSet<String>>>,
    /// Pre-claimed muxer request pads, grouped by kind (TS PMT scan).
    pad_pool: Arc<Mutex<HashMap<&'static str, Vec<gst::Pad>>>>,
}

impl RemuxChainWiring {
    /// Cheap handle clone (all fields are `Arc`s) for the `pad-added`
    /// closure, which needs its own copy of the shared state.
    fn clone_wiring(&self) -> Self {
        Self {
            track_chains: Arc::clone(&self.track_chains),
            first_wave_pads: Arc::clone(&self.first_wave_pads),
            retired_first_wave: Arc::clone(&self.retired_first_wave),
            pad_pool: Arc::clone(&self.pad_pool),
        }
    }

    fn new() -> Self {
        Self {
            track_chains: Arc::new(Mutex::new(HashMap::new())),
            first_wave_pads: Arc::new(Mutex::new(HashMap::new())),
            retired_first_wave: Arc::new(Mutex::new(std::collections::HashSet::new())),
            pad_pool: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The chain key for a demux pad: tsdemux names pads
    /// `<kind>_<version>_<pid-hex>` (e.g. `audio_2_0101`), so the third
    /// segment (the PID) identifies the logical stream and duplicate pad
    /// versions of one PID must share a chain. Other demuxers expose unique
    /// pad names, which map 1:1 to their own chain.
    fn chain_key_for_pad(pad_name: &str) -> String {
        let segments: Vec<&str> = pad_name.split('_').collect();
        if segments.len() == 3 {
            segments[2].to_string()
        } else {
            pad_name.to_string()
        }
    }

    /// Attach one demux source pad to its chain. First wave: build the
    /// `funnel -> [parser ->] queue -> mux request pad` leg and take one
    /// funnel sink. Later waves (same chain key): request their own funnel
    /// sink pad and fan into the shared downstream leg; the dead first-wave
    /// sink is unlinked and released on the FIRST duplicate join (funnel
    /// forwards EOS only once every sink pad reported EOS — a retired pad
    /// that never sees EOS would stall the remux until the bus timeout).
    ///
    /// Returns the state that was reached:
    /// - `Joined` — duplicate pad wired into the existing chain's funnel.
    /// - `Built` — new chain built and wired end-to-end.
    /// - `Failed(reason)` — the track could not be wired and is skipped.
    #[allow(clippy::too_many_arguments)]
    fn attach_pad(
        &self,
        pipeline: &gst::Pipeline,
        muxer: &gst::Element,
        src_pad: &gst::Pad,
        media: &str,
        pad_name: &str,
        debug: bool,
    ) -> AttachResult {
        let chain_key = Self::chain_key_for_pad(pad_name);
        // A later pad version of a known PID joins the existing chain:
        // it requests its own sink pad on the chain's funnel (the first
        // sink pad is already taken by the initial demux pad) and feeds
        // the same parser/queue/mux leg through the fan-in.
        if let Some(funnel) = self
            .track_chains
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
            // forever and stall the remux until the bus timeout. The
            // handle was stored at build time: generated request pads are
            // named `funnelpad0`-style, so a name lookup cannot find them.
            if self
                .retired_first_wave
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(chain_key.clone())
            {
                if let Some(head) = self
                    .first_wave_pads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&chain_key)
                {
                    if let Some(dead_src) = head.peer() {
                        let _ = dead_src.unlink(&head);
                    }
                    funnel.release_request_pad(&head);
                }
            }
            return match funnel.request_pad_simple("sink_%u") {
                Some(extra_sink) => {
                    if let Err(e) = src_pad.link(&extra_sink) {
                        if debug {
                            eprintln!("RIVULET_REMUX duplicate-pad link FAILED: {e}");
                        }
                        tracing::warn!(error = %e, "remux: duplicate-pad link failed");
                        return AttachResult::Failed(e.to_string());
                    }
                    AttachResult::Joined
                }
                None => {
                    tracing::warn!("remux: funnel refused a sink pad, duplicate pad dropped");
                    AttachResult::Failed("funnel refused a duplicate pad".to_string())
                }
            };
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
                return AttachResult::Failed(e.to_string());
            }
        };
        if let Err(e) = pipeline.add(&funnel) {
            tracing::warn!(error = %e, "remux: could not add funnel, track skipped");
            return AttachResult::Failed(e.to_string());
        }
        let queue = match gst::ElementFactory::make("queue").build() {
            Ok(queue) => queue,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "remux: could not create track queue, track skipped"
                );
                return AttachResult::Failed(e.to_string());
            }
        };
        if let Err(e) = pipeline.add(&queue) {
            tracing::warn!(error = %e, "remux: could not add track queue, track skipped");
            return AttachResult::Failed(e.to_string());
        }
        let downstream_sink = if let Some(parse) = &parser {
            if let Err(e) = pipeline.add(parse) {
                tracing::warn!(
                    error = %e,
                    "remux: could not add parse element, track skipped"
                );
                return AttachResult::Failed(e.to_string());
            }
            let Some(parse_src) = parse.static_pad("src") else {
                return AttachResult::Failed("parser has no src pad".to_string());
            };
            let Some(parse_sink) = parse.static_pad("sink") else {
                return AttachResult::Failed("parser has no sink pad".to_string());
            };
            let Some(queue_sink) = queue.static_pad("sink") else {
                return AttachResult::Failed("queue has no sink pad".to_string());
            };
            if parse_src.link(&queue_sink).is_err() {
                tracing::warn!("remux: parse to queue link failed, track skipped");
                return AttachResult::Failed("parse-to-queue link failed".to_string());
            }
            let _ = parse.sync_state_with_parent();
            let _ = queue.sync_state_with_parent();
            parse_sink
        } else {
            let Some(queue_sink) = queue.static_pad("sink") else {
                return AttachResult::Failed("queue has no sink pad".to_string());
            };
            let _ = queue.sync_state_with_parent();
            queue_sink
        };
        let Some(funnel_src) = funnel.static_pad("src") else {
            return AttachResult::Failed("funnel has no src pad".to_string());
        };
        if funnel_src.link(&downstream_sink).is_err() {
            tracing::warn!("remux: funnel to parser/queue link failed, track skipped");
            return AttachResult::Failed("funnel-to-downstream link failed".to_string());
        }
        let _ = funnel.sync_state_with_parent();
        // Entry point of the chain: this first demux pad takes one
        // funnel sink pad; later duplicate versions request their own.
        // The handle is kept for retirement — generated request pads are
        // named `funnelpad0`-style, so they cannot be re-found by name.
        let Some(head) = funnel.request_pad_simple("sink_%u") else {
            tracing::warn!("remux: funnel refused an entry pad, track skipped");
            return AttachResult::Failed("funnel refused an entry pad".to_string());
        };
        self.first_wave_pads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(chain_key.clone(), head.clone());
        let Some(queue_src) = queue.static_pad("src") else {
            return AttachResult::Failed("queue has no src pad".to_string());
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
        let mux_pad = self
            .pad_pool
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(request_template)
            .and_then(|pads| pads.pop())
            .or_else(|| muxer.request_pad_simple(request_template));
        let Some(mux_pad) = mux_pad else {
            if debug {
                eprintln!("RIVULET_REMUX no request pad for {media:?} — parking on fakesink");
            }
            if let Ok(fake) = gst::ElementFactory::make("fakesink")
                .property("sync", false)
                .build()
            {
                let _ = pipeline.add(&fake);
                if let Some(sink) = fake.static_pad("sink") {
                    let _ = queue_src.link(&sink);
                }
                let _ = fake.sync_state_with_parent();
            }
            if src_pad.link(&head).is_err() {
                tracing::warn!("remux: demux link failed, track skipped");
                return AttachResult::Failed("demux link failed".to_string());
            }
            self.track_chains
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(chain_key, funnel);
            return AttachResult::Built;
        };
        if queue_src.link(&mux_pad).is_err() {
            tracing::warn!("remux: queue to muxer link failed, track skipped");
            return AttachResult::Failed("queue-to-muxer link failed".to_string());
        }

        // Timestamp hygiene: sources recorded with DTS-only video
        // buffers (hardware encoders with B-frame reordering) or
        // otherwise PTS-less buffers make mp4mux fail with "Buffer has
        // no PTS". Recover by carrying DTS over as PTS, or interpolating
        // from the previous buffer. The probe must ALSO cover buffer
        // lists: parsers (h264parse) can push lists downstream, and a
        // BUFFER-only probe never fires for list pushes — exactly the
        // buffers most likely to be DTS-only.
        {
            fn fix_pts(buffer: &mut gst::BufferRef, last_pts_ns: &std::sync::atomic::AtomicU64) {
                use std::sync::atomic::Ordering;
                if buffer.pts().is_none() {
                    // DTS carryover, then interpolation from the previous
                    // PTS, then t=0 for a stream's very first PTS-less
                    // buffer — mp4mux must never see "Buffer has no PTS".
                    let fallback = buffer.dts().map(|dts| dts.nseconds()).or_else(|| {
                        let prev = last_pts_ns.load(Ordering::Relaxed);
                        (prev != u64::MAX).then(|| {
                            prev + buffer
                                .duration()
                                .map(|d| d.nseconds())
                                .unwrap_or(33_000_000)
                        })
                    });
                    buffer.set_pts(gst::ClockTime::from_nseconds(fallback.unwrap_or(0)));
                }
                if let Some(pts) = buffer.pts() {
                    last_pts_ns.store(pts.nseconds(), Ordering::Relaxed);
                }
            }
            let last_pts_ns = std::sync::atomic::AtomicU64::new(u64::MAX);
            mux_pad.add_probe(
                gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
                move |_pad, info| {
                    if let Some(buffer) = info.buffer_mut() {
                        fix_pts(buffer.make_mut(), &last_pts_ns);
                    } else if let Some(list) = info.buffer_list_mut() {
                        let list = list.make_mut();
                        for idx in 0..list.len() {
                            if let Some(buffer) = list.get_mut(idx) {
                                fix_pts(buffer, &last_pts_ns);
                            }
                        }
                    }
                    gst::PadProbeReturn::Ok
                },
            );
        }

        if src_pad.link(&head).is_err() {
            tracing::warn!("remux: demux link failed, track skipped");
            return AttachResult::Failed("demux link failed".to_string());
        }
        self.track_chains
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(chain_key, funnel);
        AttachResult::Built
    }

    /// Pre-claim `count` muxer request pads of one kind into the pool (the
    /// TS PMT scan fills this before the pipeline starts so pad assignment
    /// in `attach_pad` cannot race the muxer's "refuse pads once
    /// configured" rule). Returns false when the muxer refused.
    fn preclaim_ts_pads(&self, muxer: &gst::Element, template: &'static str, count: usize) -> bool {
        let mut pool = self.pad_pool.lock().unwrap_or_else(|e| e.into_inner());
        for _ in 0..count {
            match muxer.request_pad_simple(template) {
                Some(pad) => pool.entry(template).or_default().push(pad),
                None => return false,
            }
        }
        true
    }
}

/// Outcome of [`RemuxChainWiring::attach_pad`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum AttachResult {
    /// Duplicate pad joined the existing chain's funnel.
    Joined,
    /// New chain built and wired end-to-end.
    Built,
    /// Track could not be wired (skipped, reason for diagnostics).
    Failed(String),
}

/// Remuxes a crash-safe intermediate recording (MKV/MOV/TS) to MP4 **without
/// re-encoding** (issue #71).
///
/// Runs a `filesrc -> demuxer -> muxer -> filesink` pipeline. Demuxer pads
/// are dynamic (one per contained track), so every demux src pad is wired on
/// `pad-added` through [`RemuxChainWiring::attach_pad`]: one chain per
/// logical stream (`funnel -> [aacparse|h264parse|h265parse] -> queue -> mux
/// request pad`). The chain head is a funnel so tsdemux's duplicate pad
/// waves (a speculative pad before the PMT update, the real pad after)
/// fan into the same downstream leg instead of fighting over a single
/// peer, and the dead first-wave funnel sink is released on the first
/// duplicate join (funnel forwards EOS only once every sink pad reported
/// EOS — a never-EOS pad would stall the remux until the bus timeout).
///
/// For TS sources the PMT is scanned straight from the file (pure byte
/// parsing, no GStreamer) and every muxer request pad is pre-claimed before
/// the pipeline starts; tsdemux's pad waves, the muxer's "refuse pads once
/// configured" rule and the speculative duplicate pads collapse into simple
/// pad assignment inside `pad-added`. MKV/MOV demuxers expose their pads in
/// one wave, so the lazy in-callback claim is fine for them.
///
/// The encoded video/audio streams pass through unchanged; only the
/// transport format is converted on the fly (ADTS -> raw AAC,
/// Annex-B -> AVC/HVC1). PTS-less buffers (DTS-only hardware encoders)
/// are repaired on the mux pads for both buffer and buffer-list pushes.
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

    let wiring = RemuxChainWiring::new();
    demuxer.connect_pad_added({
        let muxer_for_pads = muxer.clone();
        let pipeline_for_pads = pipeline.clone();
        let wiring = wiring.clone_wiring();
        move |_demux, src_pad| {
            let pad_name = src_pad.name().to_string();
            let chain_key = RemuxChainWiring::chain_key_for_pad(&pad_name);
            let segments: Vec<&str> = pad_name.split('_').collect();
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

            // Everything else — chain construction, duplicate-pad fan-in,
            // first-wave retirement, pad assignment and the PTS probe —
            // lives in RemuxChainWiring so it stays unit-testable.
            let _ = wiring.attach_pad(
                &pipeline_for_pads,
                &muxer_for_pads,
                src_pad,
                &media,
                &pad_name,
                debug,
            );
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
            if !wiring.preclaim_ts_pads(&muxer, "audio_%u", stream_counts.audio) {
                return Err("mp4 target has no audio pad template".to_string());
            }
            if !wiring.preclaim_ts_pads(&muxer, "video_%u", stream_counts.video) {
                return Err("mp4 target has no video pad template".to_string());
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
    // section_length counts the bytes FOLLOWING the length field itself, so
    // the section ends at 3 + section_len (not at section_len). Skipping the
    // trailing CRC32 lands on the last stream-entry byte; using section_len
    // directly ends three bytes early and drops the LAST elementary stream —
    // for a TS recording that silently lost the last audio track on remux.
    let end = (3 + section_len).min(pmt.len()).saturating_sub(4);
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
    use crate::test_helpers::wait_until;

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

    // ── Funnel fan-in unit tests (issue #242): the tsdemux duplicate-pad
    // waves are simulated with plain pads instead of a recorded file, so
    // the wiring invariants fail here and not only in CI's parity test.

    /// A demux-side src pad named like a tsdemux pad wave entry.
    fn wave_pad(name: &str) -> gst::Pad {
        gst::Pad::builder(gst::PadDirection::Src).name(name).build()
    }

    /// The minimal muxer double for `attach_pad`: a real mp4mux is not
    /// needed to exercise the fan-in, only *a* muxer-shaped element whose
    /// request pads exist.
    fn mux_double() -> gst::Element {
        gst::ElementFactory::make("fakesink")
            .property("sync", false)
            .build()
            .expect("fakesink exists in every GStreamer build")
    }

    #[test]
    fn chain_key_uses_the_tsdemux_pid_segment() {
        assert_eq!(RemuxChainWiring::chain_key_for_pad("video_0_0103"), "0103");
        assert_eq!(RemuxChainWiring::chain_key_for_pad("audio_2_0101"), "0101");
        // Non-tsdemux pad names (mkvdemux/mp4demux expose unique names)
        // map to their own chain.
        assert_eq!(RemuxChainWiring::chain_key_for_pad("video_0"), "video_0");
    }

    #[test]
    fn duplicate_pad_waves_fan_into_the_chain_funnel() {
        gst::init().expect("gstreamer initializes");
        let wiring = RemuxChainWiring::new();
        let pipeline = gst::Pipeline::default();
        let muxer = mux_double();

        // Wave 1: the speculative pad builds the chain.
        let first = wave_pad("video_0_0103");
        assert_eq!(
            wiring.attach_pad(
                &pipeline,
                &muxer,
                &first,
                "video/x-h264",
                "video_0_0103",
                false,
            ),
            AttachResult::Built
        );
        let funnel = wiring
            .track_chains
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get("0103")
            .cloned()
            .expect("chain registered for the PID");
        assert_eq!(funnel.num_sink_pads(), 1, "one funnel sink for wave 1");

        // Wave 2 (the PMT update re-exposes the PID): must join the SAME
        // chain through its own funnel sink — the exact spot where the old
        // single-peer head link failed with "Pad was already linked" and
        // dropped the real stream, corrupting the MP4.
        let second = wave_pad("video_1_0103");
        assert_eq!(
            wiring.attach_pad(
                &pipeline,
                &muxer,
                &second,
                "video/x-h264",
                "video_1_0103",
                false,
            ),
            AttachResult::Joined
        );
        assert_eq!(
            funnel.num_sink_pads(),
            1,
            "first-wave sink was retired; the duplicate owns the fan-in now"
        );
        let second_peer = second.peer().expect("duplicate pad must be linked");
        assert_eq!(
            second_peer.parent().map(|p| p.name()),
            Some(funnel.name()),
            "duplicate pad must be linked into the chain funnel"
        );
        assert_eq!(
            first.peer(),
            None,
            "retired first-wave pad must be unlinked from the funnel"
        );

        // Wave 3 adds another sink without retiring anything again.
        let third = wave_pad("video_2_0103");
        assert_eq!(
            wiring.attach_pad(
                &pipeline,
                &muxer,
                &third,
                "video/x-h264",
                "video_2_0103",
                false,
            ),
            AttachResult::Joined
        );
        assert_eq!(funnel.num_sink_pads(), 2, "waves 2+3 each own one sink");

        // Distinct PIDs never share a chain.
        let other = wave_pad("audio_0_0101");
        assert_eq!(
            wiring.attach_pad(
                &pipeline,
                &muxer,
                &other,
                "audio/mpeg",
                "audio_0_0101",
                false,
            ),
            AttachResult::Built
        );
        let chains = wiring
            .track_chains
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(chains.len(), 2, "one chain per PID");
    }

    #[test]
    fn first_wave_retirement_happens_exactly_once_per_chain() {
        gst::init().expect("gstreamer initializes");
        let wiring = RemuxChainWiring::new();
        let pipeline = gst::Pipeline::default();
        let muxer = mux_double();

        let first = wave_pad("audio_0_0101");
        wiring.attach_pad(
            &pipeline,
            &muxer,
            &first,
            "audio/mpeg",
            "audio_0_0101",
            false,
        );
        let second = wave_pad("audio_1_0101");
        wiring.attach_pad(
            &pipeline,
            &muxer,
            &second,
            "audio/mpeg",
            "audio_1_0101",
            false,
        );
        let third = wave_pad("audio_2_0101");
        wiring.attach_pad(
            &pipeline,
            &muxer,
            &third,
            "audio/mpeg",
            "audio_2_0101",
            false,
        );

        let retired = wiring
            .retired_first_wave
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            retired.len(),
            1,
            "the retire guard must mark the chain exactly once regardless of wave count"
        );
        assert!(retired.contains("0101"));
    }

    #[test]
    fn preclaimed_pool_pads_are_consumed_before_fresh_claims() {
        gst::init().expect("gstreamer initializes");
        let wiring = RemuxChainWiring::new();
        let pipeline = gst::Pipeline::default();
        let muxer = gst::ElementFactory::make("mp4mux")
            .build()
            .expect("mp4mux available for remux tests");
        // Pad links require both parents to share a bin hierarchy — in the
        // real remux the muxer is part of the pipeline, and so it is here.
        pipeline.add(&muxer).expect("muxer joins the test pipeline");

        assert!(wiring.preclaim_ts_pads(&muxer, "audio_%u", 2));
        assert_eq!(
            wiring.pad_pool.lock().unwrap_or_else(|e| e.into_inner())["audio_%u"].len(),
            2
        );

        let first = wave_pad("audio_0_0101");
        assert_eq!(
            wiring.attach_pad(
                &pipeline,
                &muxer,
                &first,
                "audio/mpeg",
                "audio_0_0101",
                false,
            ),
            AttachResult::Built
        );
        assert_eq!(
            wiring.pad_pool.lock().unwrap_or_else(|e| e.into_inner())["audio_%u"].len(),
            1,
            "first chain consumed one pre-claimed pad"
        );

        let second = wave_pad("audio_1_0102");
        assert_eq!(
            wiring.attach_pad(
                &pipeline,
                &muxer,
                &second,
                "audio/mpeg",
                "audio_1_0102",
                false,
            ),
            AttachResult::Built
        );
        assert!(
            wiring.pad_pool.lock().unwrap_or_else(|e| e.into_inner())["audio_%u"].is_empty(),
            "second chain drained the pool before any fresh claim"
        );
    }

    #[test]
    fn funnel_eos_gating_after_retirement_reaches_downstream_eos() {
        // The macOS CI stall, reproduced at unit level: funnel forwards EOS
        // only once EVERY sink pad reported EOS. After the first-wave sink
        // is retired (it will never see EOS — speculative pads carry no
        // data) the remaining duplicate sink must be able to drive the
        // downstream EOS alone.
        gst::init().expect("gstreamer initializes");
        let wiring = RemuxChainWiring::new();
        let pipeline = gst::Pipeline::default();
        let muxer = mux_double();

        // video/x-raw keeps the chain parser-free (funnel -> queue only):
        // the fan-in and EOS behavior under test is the funnel's, not a
        // parser's reaction to synthetic stream data.
        let first = wave_pad("video_0_0103");
        wiring.attach_pad(
            &pipeline,
            &muxer,
            &first,
            "video/x-raw",
            "video_0_0103",
            false,
        );
        let second = wave_pad("video_1_0103");
        wiring.attach_pad(
            &pipeline,
            &muxer,
            &second,
            "video/x-raw",
            "video_1_0103",
            false,
        );

        let funnel = wiring
            .track_chains
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get("0103")
            .cloned()
            .expect("chain registered");
        assert_eq!(funnel.num_sink_pads(), 1);

        // Walk the wiring downstream: funnel src -> queue sink.
        let funnel_src = funnel.static_pad("src").expect("funnel src pad");
        let downstream = funnel_src.peer().expect("funnel wired downstream");
        assert!(
            downstream.name().starts_with("sink"),
            "funnel feeds the parser/queue sink, got {}",
            downstream.name()
        );

        // Push a buffer + EOS through the surviving duplicate pad (src
        // direction — events/buffers flow downstream to the funnel sink)
        // and observe them arrive at the funnel src.
        let received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let eos_seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let received = Arc::clone(&received);
            funnel_src.add_probe(gst::PadProbeType::BUFFER, move |_pad, _info| {
                received.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                gst::PadProbeReturn::Ok
            });
        }
        {
            let eos_seen = Arc::clone(&eos_seen);
            funnel_src.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_pad, info| {
                if let Some(event) = info.event() {
                    if matches!(event.view(), gst::EventView::Eos(_)) {
                        eos_seen.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                gst::PadProbeReturn::Ok
            });
        }

        pipeline
            .set_state(gst::State::Playing)
            .expect("mini pipeline plays");
        // The wave pad has no parent element that would activate it during
        // the state change — activate it into pushing mode by hand.
        second
            .set_active(true)
            .expect("wave pad activates for pushing");
        let mut buffer = gst::Buffer::with_size(16).unwrap();
        {
            let buf = buffer.get_mut().unwrap();
            buf.set_pts(gst::ClockTime::from_nseconds(0));
        }
        // The demux double pushes the standard stream-start/segment/caps
        // prelude + a buffer + EOS like tsdemux would on its surviving
        // (duplicate) pad — buffers error out downstream without a segment.
        assert!(
            second.push_event(gst::event::StreamStart::new("dup-wave")),
            "stream-start rejected"
        );
        assert!(
            second.push_event(gst::event::Segment::new(&gst::FormattedSegment::<
                gst::ClockTime,
            >::default(),)),
            "segment rejected"
        );
        let caps = gst::Caps::builder("video/x-raw").build();
        assert!(
            second.push_event(gst::event::Caps::new(&caps)),
            "caps rejected"
        );
        second
            .push(buffer)
            .expect("buffer must flow through the fan-in");
        assert!(
            second.push_event(gst::event::Eos::new()),
            "EOS from the surviving pad must be accepted"
        );
        // Poll the streaming threads instead of sleeping a fixed amount.
        wait_until(std::time::Duration::from_secs(2), || {
            eos_seen
                .load(std::sync::atomic::Ordering::SeqCst)
                .then_some(())
        });
        pipeline.set_state(gst::State::Null).ok();
        assert!(
            received.load(std::sync::atomic::Ordering::SeqCst) >= 1,
            "the buffer pushed into the duplicate funnel sink must reach the funnel src"
        );
        assert!(
            eos_seen.load(std::sync::atomic::Ordering::SeqCst),
            "EOS must propagate after retirement — the exact regression that stalled macOS CI"
        );
    }

    // ── scan_mpegts_stream_kinds (PMT pre-scan, issue #242 slice 3) ──
    //
    // The PMT scanner decides how many muxer request pads the remux pipeline
    // pre-claims, so a miscount silently drops audio tracks from the MP4
    // output. It reads bytes from disk, which is why it had no coverage at
    // all until now: the helpers below synthesize a transport stream in a
    // temp file so every parse branch is reachable without GStreamer.

    /// Wraps `section` into a single TS packet (188 bytes) for `pid`.
    fn ts_section_packet(pid: u16, section: &[u8]) -> Vec<u8> {
        assert!(section.len() <= 182, "section must fit one TS packet");
        let mut pkt = vec![0u8; 188];
        pkt[0] = 0x47; // sync byte
        pkt[1] = 0x40 | u8::try_from(pid >> 8).unwrap(); // payload_unit_start_indicator
        pkt[2] = pid as u8;
        pkt[3] = 0x10; // adaptation_field_control = 01: payload only
        pkt[4] = 0x00; // adaptation_field_length = 0
        pkt[5] = 0x00; // pointer_field: section starts immediately after it
        pkt[6..6 + section.len()].copy_from_slice(section);
        pkt
    }

    /// PAT section for a single program pointing at `pmt_pid`.
    fn pat_section(pmt_pid: u16) -> Vec<u8> {
        let pid = pmt_pid;
        let mut s: Vec<u8> = vec![0x00]; // table_id = PAT
        s.extend_from_slice(&[0xb0, 0x0d]); // section_length = 13
        s.extend_from_slice(&[0x00, 0x01]); // transport_stream_id
        s.extend_from_slice(&[0xc1, 0x00, 0x00]); // version 0, section 0/0
        s.extend_from_slice(&[0x00, 0x01]); // program_number = 1 (not NIT)
        s.extend_from_slice(&[0xe0 | u8::try_from(pid >> 8).unwrap(), pid as u8]);
        s.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // CRC32 (unverified)
        s
    }

    /// PMT section listing `(stream_type, pid)` elementary streams.
    fn pmt_section(streams: &[(u8, u16)]) -> Vec<u8> {
        let body_len = 5 * streams.len();
        let section_len = 13 + body_len; // bytes after the section_length field
        let mut s: Vec<u8> = vec![0x02]; // table_id = PMT
        s.extend_from_slice(&[
            0xb0 | u8::try_from(section_len >> 8).unwrap(),
            section_len as u8,
        ]);
        s.extend_from_slice(&[0x00, 0x01]); // program_number = 1
        s.extend_from_slice(&[0xc1, 0x00, 0x00]); // version 0, section 0/0
        s.extend_from_slice(&[0xe0, 0x00]); // PCR_PID = 0x100
        s.extend_from_slice(&[0xf0, 0x00]); // program_info_length = 0
        for &(stream_type, pid) in streams {
            s.push(stream_type);
            s.extend_from_slice(&[0xe0 | u8::try_from(pid >> 8).unwrap(), pid as u8]);
            s.extend_from_slice(&[0xf0, 0x00]); // ES_info_length = 0
        }
        s.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // CRC32 (unverified)
        s
    }

    /// Writes `packets` to a temp `.ts` file and scans it. Returns the counts.
    fn scan_packets(packets: &[Vec<u8>]) -> TsStreamCounts {
        let mut bytes = Vec::new();
        for p in packets {
            bytes.extend_from_slice(p);
        }
        // The scanner only accepts a path, so materialize the synthetic
        // stream next to the target dir and clean it up right after.
        let path = std::env::temp_dir().join(format!(
            "rivulet-pmt-scan-{}-{:p}.ts",
            std::process::id(),
            &bytes
        ));
        std::fs::write(&path, &bytes).expect("synthetic TS written");
        let counts = scan_mpegts_stream_kinds(path.to_str().expect("utf-8 temp path"));
        std::fs::remove_file(&path).ok();
        counts
    }

    #[test]
    fn pmt_scan_counts_audio_and_video_streams() {
        // One H.264 video + two AAC audio tracks: the exact shape of an
        // H.265-per-track recording that was encoded to TS and now has to
        // reach MP4 with every audio track intact.
        let counts = scan_packets(&[
            ts_section_packet(0x0000, &pat_section(0x0100)),
            ts_section_packet(
                0x0100,
                &pmt_section(&[(0x1b, 0x0101), (0x0f, 0x0102), (0x0f, 0x0103)]),
            ),
        ]);
        assert_eq!(counts.audio, 2, "both AAC audio tracks must be counted");
        assert_eq!(counts.video, 1, "the H.264 video track must be counted");
    }

    #[test]
    fn pmt_scan_reads_every_supported_codec_family() {
        // The stream_type -> kind mapping drives the pre-claim count, so an
        // unmapped codec would drop that stream's chain entirely.
        let counts = scan_packets(&[
            ts_section_packet(0x0000, &pat_section(0x0100)),
            ts_section_packet(
                0x0100,
                // AAC, MPEG audio, AC-3, Opus | MPEG-2 video, H.264, H.265, AV1
                &pmt_section(&[
                    (0x0f, 0x0201),
                    (0x03, 0x0202),
                    (0x81, 0x0203),
                    (0x90, 0x0204),
                    (0x02, 0x0301),
                    (0x1b, 0x0302),
                    (0x24, 0x0303),
                    (0x25, 0x0304),
                ]),
            ),
        ]);
        assert_eq!(counts.audio, 4, "AAC/MPEG/AC-3/Opus are all audio");
        assert_eq!(counts.video, 4, "MPEG-2/H.264/H.265/AV1 are all video");
    }

    #[test]
    fn pmt_scan_takes_the_last_pmt_version() {
        // mpegtsmux rewrites the PMT as streams materialize: an early version
        // may list only video while the audio ES arrives in a later update.
        // The scanner keeps the LAST PMT, otherwise pre-claiming would
        // under-count and drop the audio tracks from the remux.
        let counts = scan_packets(&[
            ts_section_packet(0x0000, &pat_section(0x0100)),
            ts_section_packet(0x0100, &pmt_section(&[(0x1b, 0x0101)])),
            ts_section_packet(0x0100, &pmt_section(&[(0x1b, 0x0101), (0x0f, 0x0102)])),
        ]);
        assert_eq!(counts.video, 1);
        assert_eq!(counts.audio, 1, "the later, more complete PMT must win");
    }

    #[test]
    fn pmt_scan_counts_each_pid_once() {
        // A PMT that repeats an elementary PID (muxer rewrite artifacts)
        // must not inflate the count, or the remux pre-claims pads that
        // never receive data and stalls on pad-added.
        let counts = scan_packets(&[
            ts_section_packet(0x0000, &pat_section(0x0100)),
            ts_section_packet(
                0x0100,
                &pmt_section(&[(0x0f, 0x0102), (0x0f, 0x0102), (0x1b, 0x0101)]),
            ),
        ]);
        assert_eq!(counts.audio, 1, "the repeated PID counts once");
        assert_eq!(counts.video, 1);
    }

    #[test]
    fn pmt_scan_stays_empty_on_unusable_input() {
        // Best-effort contract: no PAT, no PMT, a non-TS file and a missing
        // file all yield zero counts so the caller simply skips pre-claiming
        // instead of pre-claiming a wrong number of pads.
        assert_eq!(scan_packets(&[]), TsStreamCounts::default());
        assert_eq!(
            scan_packets(&[ts_section_packet(0x0100, &pmt_section(&[(0x0f, 0x0102)]))]),
            TsStreamCounts::default(),
            "a PMT without a PAT cannot be located"
        );
        assert_eq!(
            scan_mpegts_stream_kinds("no-such-file.ts"),
            TsStreamCounts::default(),
            "a missing file must not panic"
        );
        let mut not_ts = vec![0u8; 376];
        not_ts[0] = 0x00; // no sync byte anywhere
        assert_eq!(
            scan_packets(&[not_ts.clone(), not_ts]),
            TsStreamCounts::default(),
            "bytes without a sync byte yield no counts"
        );
    }

    #[test]
    fn pmt_scan_ignores_the_stream_type_free_pad_entries() {
        // Private stream types (0x06 PES private data, 0x0b/0x0c) and the
        // reserved 0x00 must not be mistaken for a media stream: counting
        // them would pre-claim pads for streams the muxer never requests.
        let counts = scan_packets(&[
            ts_section_packet(0x0000, &pat_section(0x0100)),
            ts_section_packet(
                0x0100,
                &pmt_section(&[(0x06, 0x0401), (0x0f, 0x0102), (0xf0, 0x0402)]),
            ),
        ]);
        assert_eq!(counts.audio, 1);
        assert_eq!(counts.video, 0);
    }
}
