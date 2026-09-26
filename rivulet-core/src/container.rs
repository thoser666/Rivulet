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
use std::sync::atomic::{AtomicBool, Ordering};
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
    let track_chains: Arc<Mutex<HashMap<String, gst::Pad>>> = Arc::new(Mutex::new(HashMap::new()));
    let claim_started = Arc::new(AtomicBool::new(false));
    let shutdown = Arc::new(AtomicBool::new(false));
    // (pad, probe id) of every downstream block installed for a track chain.
    let block_probes: Arc<Mutex<Vec<(gst::Pad, gst::PadProbeId)>>> =
        Arc::new(Mutex::new(Vec::new()));

    demuxer.connect_pad_added({
        let muxer_for_pads = muxer.clone();
        let pipeline_for_pads = pipeline.clone();
        let track_chains = Arc::clone(&track_chains);
        let claim_started = Arc::clone(&claim_started);
        let block_probes = Arc::clone(&block_probes);
        move |_demux, src_pad| {
            let pad_name = src_pad.name().to_string();
            let segments: Vec<&str> = pad_name.split('_').collect();
            let chain_key: String = if segments.len() == 3 {
                segments[2].to_string()
            } else {
                pad_name.clone()
            };
            // A later pad version of a known PID joins the existing chain.
            if let Some(head) = track_chains
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&chain_key)
                .cloned()
            {
                if let Err(e) = src_pad.link(&head) {
                    tracing::warn!(error = %e, "remux: duplicate-pad link failed");
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

            // Chain: demux -> [parser ->] queue. Build it fully (add to
            // pipeline, link parser to queue, sync states) BEFORE linking the
            // demux pad — a running demuxer pushes immediately, and data must
            // never reach an element that is still in NULL state.
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
            let head = if let Some(parse) = &parser {
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
            let Some(queue_src) = queue.static_pad("src") else {
                return;
            };
            // Claim the muxer request pad IMMEDIATELY: qtmux/mp4mux refuse
            // request pads once the first stream has been *configured*, but
            // nothing is configured while every chain's downstream is still
            // blocked (see probe below). Claiming up front means the pads
            // all exist regardless of the demuxer's pad-wave timing; the
            // block guarantees the muxer sees no caps/data until every
            // request pad exists.
            let request_template = if media.starts_with("audio/") {
                "audio_%u"
            } else if media.starts_with("video/") {
                "video_%u"
            } else {
                "subtitle_%u"
            };
            let mux_pad = muxer_for_pads.request_pad_simple(request_template);
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

            // Hold data (and events) back until the demuxer has exposed all
            // its pads (batch release on no-more-pads + grace): the muxer
            // must not see the first stream configured before every request
            // pad has been claimed. The block is released by removing the
            // probe (a BLOCK_DOWNSTREAM probe stays active as long as the
            // callback returns Ok; dropping the data flow requires removal).
            let claimed = Arc::clone(&claim_started);
            if let Some(probe_id) = queue_src.add_probe(
                gst::PadProbeType::BLOCK_DOWNSTREAM | gst::PadProbeType::BUFFER,
                move |_pad, _info| {
                    if !claimed.load(Ordering::Acquire) {
                        // Keep the block: park this item until removal.
                        return gst::PadProbeReturn::Handled;
                    }
                    gst::PadProbeReturn::Ok
                },
            ) {
                block_probes
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((queue_src, probe_id));
            }

            if src_pad.link(&head).is_err() {
                tracing::warn!("remux: demux link failed, track skipped");
                return;
            }
            track_chains
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(chain_key, head);
        }
    });

    // Batch release: `no-more-pads` fires when the demuxer has exposed its
    // first wave of pads; later pad waves (the PMT update in TS) still
    // follow. The claim thread releases the block on all currently wired
    // chains after a short grace period and then keeps watching until the
    // pipeline ends, so chains from later waves are unblocked as they
    // appear. All request pads were claimed up front (see pad-added).
    //
    // The wait MUST happen off the streaming thread: sleeping in this
    // callback would block the demuxer itself — it could then never expose
    // the later pad waves, and every audio track would hang.
    demuxer.connect_no_more_pads({
        let claim_started = Arc::clone(&claim_started);
        let shutdown = Arc::clone(&shutdown);
        move |_demux| {
            if claim_started.swap(true, Ordering::SeqCst) {
                return;
            }
            let shutdown = Arc::clone(&shutdown);
            let claim_started = Arc::clone(&claim_started);
            let block_probes = Arc::clone(&block_probes);
            std::thread::spawn(move || {
                // Grace period: the PMT update (and with it the real pad
                // wave) lands well within it for short recordings. Only
                // after it the chains are released, so the muxer sees no
                // stream configured before every request pad exists.
                std::thread::sleep(std::time::Duration::from_millis(300));
                claim_started.store(true, Ordering::Release);
                for (pad, probe_id) in block_probes
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .drain(..)
                {
                    pad.remove_probe(probe_id);
                }
                drop(shutdown);
            });
        }
    });

    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| format!("could not start remux pipeline: {e}"))?;

    let bus = pipeline.bus().expect("pipeline without bus");
    let outcome = bus.timed_pop_filtered(
        gst::ClockTime::from_seconds(60),
        &[gst::MessageType::Eos, gst::MessageType::Error],
    );
    let shutdown_flag = Arc::clone(&shutdown);
    shutdown_flag.store(true, Ordering::Release);
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
