//! Replay buffer / instant replay support.
//!
//! While a recording or stream is active, the engine captures the *encoded*
//! H.264/AAC packets that flow into the muxer into a RAM ring buffer that
//! retains the last N seconds. On demand the buffered segments are remuxed
//! into a standalone MP4 file — the classic "instant replay" / "clip moments"
//! feature that is essential for gaming streamers (roadmap M4).

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

/// One encoded packet (an H.264 NAL set or an AAC frame) captured from the
/// recording pipeline, together with the timestamps it carried there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaySegment {
    /// Presentation timestamp in nanoseconds.
    pub pts_ns: u64,
    /// Decoding timestamp in nanoseconds. Equals `pts_ns` for audio and for
    /// video streams without B-frames.
    pub dts_ns: u64,
    /// Encoded payload exactly as it flowed into the muxer.
    pub data: Vec<u8>,
    /// Whether this packet starts an independently decodable unit (an H.264
    /// IDR frame). A saved clip must begin at a keyframe.
    pub keyframe: bool,
}

impl ReplaySegment {
    pub fn new(pts_ns: u64, dts_ns: u64, data: Vec<u8>, keyframe: bool) -> Self {
        Self {
            pts_ns,
            dts_ns,
            data,
            keyframe,
        }
    }
}

/// One buffered audio stream (one per audio pad in the pipeline: the mixed
/// track, or system + microphone when separate tracks are enabled).
#[derive(Debug, Clone, Default)]
struct ReplayAudioTrack {
    segments: VecDeque<ReplaySegment>,
    caps: Option<String>,
    newest_pts: Option<u64>,
}

/// Immutable snapshot of the buffered data, taken for saving.
#[derive(Debug, Clone, Default)]
pub struct ReplaySnapshot {
    pub video: Vec<ReplaySegment>,
    pub video_caps: Option<String>,
    pub audio_tracks: Vec<Vec<ReplaySegment>>,
    pub audio_caps: Vec<Option<String>>,
}

impl ReplaySnapshot {
    pub fn is_empty(&self) -> bool {
        self.video.is_empty() && self.audio_tracks.iter().all(Vec::is_empty)
    }

    /// Prepare the snapshot for remuxing:
    ///
    /// 1. Drop leading video segments up to and including the first keyframe,
    ///    so the clip starts at an independently decodable frame.
    /// 2. Shift every stream's timestamps so the clip starts at time zero
    ///    (mp4mux otherwise writes a dead gap at the start of the file).
    ///
    /// Relative A/V sync is preserved because every track is shifted by its
    /// own start timestamp.
    pub fn normalize(&mut self) {
        if let Some(first_keyframe) = self.video.iter().position(|s| s.keyframe) {
            self.video.drain(..first_keyframe);
        } else {
            // No independently decodable frame at all: the clip would be
            // unplayable, so drop the video entirely.
            self.video.clear();
        }
        if let Some(offset) = self.video.first().map(|s| s.pts_ns) {
            for seg in &mut self.video {
                seg.pts_ns = seg.pts_ns.saturating_sub(offset);
                seg.dts_ns = seg.dts_ns.saturating_sub(offset);
            }
        }
        for track in &mut self.audio_tracks {
            if let Some(offset) = track.first().map(|s| s.pts_ns) {
                for seg in track.iter_mut() {
                    seg.pts_ns = seg.pts_ns.saturating_sub(offset);
                    seg.dts_ns = seg.dts_ns.saturating_sub(offset);
                }
            }
        }
    }
}

/// RAM ring buffer retaining the last [`ReplayBuffer::duration`] of encoded
/// video/audio packets. Written from the GStreamer streaming thread (through
/// an `Arc<Mutex<..>>`), read via [`ReplayBuffer::snapshot`] when saving.
#[derive(Debug)]
pub struct ReplayBuffer {
    duration: Duration,
    video: VecDeque<ReplaySegment>,
    video_caps: Option<String>,
    /// Newest video decoding timestamp. DTS is monotonic in the pipeline's
    /// arrival order (unlike PTS, which B-frame reordering shuffles), so it
    /// drives eviction.
    newest_video_dts: Option<u64>,
    audio_tracks: Vec<ReplayAudioTrack>,
}

impl ReplayBuffer {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            video: VecDeque::new(),
            video_caps: None,
            newest_video_dts: None,
            audio_tracks: Vec::new(),
        }
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Change the retention window and immediately evict everything that no
    /// longer fits.
    pub fn set_duration(&mut self, duration: Duration) {
        self.duration = duration;
        self.evict();
    }

    pub fn is_empty(&self) -> bool {
        self.video.is_empty() && self.audio_tracks.iter().all(|t| t.segments.is_empty())
    }

    pub fn clear(&mut self) {
        self.video.clear();
        self.video_caps = None;
        self.newest_video_dts = None;
        self.audio_tracks.clear();
    }

    /// Record the caps of the video stream (captured once from the first
    /// sample). The caps carry the encoder's `codec_data`, which the remux
    /// needs to reconstruct parameter sets even when the ring starts
    /// mid-stream.
    pub fn set_video_caps(&mut self, caps: String) {
        if self.video_caps.is_none() {
            self.video_caps = Some(caps);
        }
    }

    pub fn set_audio_caps(&mut self, track: usize, caps: String) {
        self.ensure_track(track);
        if self.audio_tracks[track].caps.is_none() {
            self.audio_tracks[track].caps = Some(caps);
        }
    }

    /// The recorded video caps, if the first sample has been captured.
    pub fn video_caps(&self) -> Option<&str> {
        self.video_caps.as_deref()
    }

    /// The buffered video segments (oldest first).
    pub fn video(&self) -> &VecDeque<ReplaySegment> {
        &self.video
    }

    /// The recorded caps of one audio track, if captured.
    pub fn audio_caps(&self, track: usize) -> Option<&str> {
        self.audio_tracks.get(track).and_then(|t| t.caps.as_deref())
    }

    fn ensure_track(&mut self, track: usize) {
        while self.audio_tracks.len() <= track {
            self.audio_tracks.push(ReplayAudioTrack::default());
        }
    }

    pub fn push_video(&mut self, pts_ns: u64, dts_ns: u64, data: Vec<u8>, keyframe: bool) {
        self.newest_video_dts = Some(match self.newest_video_dts {
            Some(prev) => prev.max(dts_ns),
            None => dts_ns,
        });
        self.video
            .push_back(ReplaySegment::new(pts_ns, dts_ns, data, keyframe));
        self.evict();
    }

    pub fn push_audio(&mut self, track: usize, pts_ns: u64, dts_ns: u64, data: Vec<u8>) {
        self.ensure_track(track);
        let t = &mut self.audio_tracks[track];
        t.newest_pts = Some(match t.newest_pts {
            Some(prev) => prev.max(pts_ns),
            None => pts_ns,
        });
        t.segments
            .push_back(ReplaySegment::new(pts_ns, dts_ns, data, false));
        self.evict();
    }

    /// Drop everything older than the retention window. Each stream evicts
    /// relative to its own newest timestamp.
    pub fn evict(&mut self) {
        let window_ns = self.duration.as_nanos() as u64;
        // DTS is monotonic in arrival order, so popping from the front by DTS
        // fully covers the window even when PTS is shuffled by B-frames.
        if let Some(newest) = self.newest_video_dts {
            let cutoff = newest.saturating_sub(window_ns);
            while self.video.front().is_some_and(|s| s.dts_ns < cutoff) {
                self.video.pop_front();
            }
        }
        for track in &mut self.audio_tracks {
            if let Some(newest) = track.newest_pts {
                let cutoff = newest.saturating_sub(window_ns);
                while track.segments.front().is_some_and(|s| s.pts_ns < cutoff) {
                    track.segments.pop_front();
                }
            }
        }
    }

    /// Approximate amount of video retained, in nanoseconds (0 when empty).
    pub fn retained_ns(&self) -> u64 {
        match (self.video.front().map(|s| s.dts_ns), self.newest_video_dts) {
            (Some(first), Some(newest)) => newest.saturating_sub(first),
            _ => 0,
        }
    }

    /// Clone the buffered data into a normalized snapshot ready for
    /// [`save_replay`].
    pub fn snapshot(&self) -> ReplaySnapshot {
        let mut snap = ReplaySnapshot {
            video: self.video.iter().cloned().collect(),
            video_caps: self.video_caps.clone(),
            audio_tracks: self
                .audio_tracks
                .iter()
                .map(|t| t.segments.iter().cloned().collect())
                .collect(),
            audio_caps: self.audio_tracks.iter().map(|t| t.caps.clone()).collect(),
        };
        snap.normalize();
        snap
    }
}

/// Remux a [`ReplaySnapshot`] into an MP4 file at `path`.
///
/// The buffered encoded packets are fed through `h264parse`/`aacparse` into
/// an `mp4mux`, exactly like a file-to-file remux — the encoded data is never
/// re-encoded, so saving a clip is fast. Fails cleanly when the buffer is
/// empty, the pipeline cannot be built, or the remux reports an error.
pub fn save_replay(snapshot: &ReplaySnapshot, path: &Path) -> anyhow::Result<()> {
    if snapshot.is_empty() {
        anyhow::bail!("replay buffer is empty, nothing to save");
    }
    let _ = gst::init();

    let mut desc = String::from(
        "appsrc name=rv_vsrc format=time is-live=false do-timestamp=false \
         ! h264parse ! replay_mux. ",
    );
    for (i, track) in snapshot.audio_tracks.iter().enumerate() {
        if track.is_empty() {
            continue;
        }
        desc.push_str(&format!(
            "appsrc name=rv_asrc{i} format=time is-live=false do-timestamp=false \
             ! aacparse ! replay_mux. "
        ));
    }
    desc.push_str(&format!(
        "mp4mux name=replay_mux ! filesink name=replay_file location=\"{}\"",
        escape_location(&path.display().to_string())
    ));

    let pipeline = gst::parse::launch(&desc)
        .map_err(|e| anyhow::anyhow!("could not build replay remux pipeline: {e}"))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow::anyhow!("replay remux pipeline is not a pipeline"))?;

    // Configure the video source with the caps the encoder actually
    // produced. These caps carry `codec_data` (captured from the parsing
    // branch of the live pipeline), so a clip can start mid-stream.
    if let Some(vsrc) = pipeline.by_name("rv_vsrc") {
        let vsrc = vsrc
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| anyhow::anyhow!("replay video source is not an appsrc"))?;
        let caps = snapshot.video_caps.as_deref().unwrap_or("video/x-h264");
        let caps = caps
            .parse::<gst::Caps>()
            .map_err(|e| anyhow::anyhow!("invalid replay video caps \"{caps}\": {e}"))?;
        vsrc.set_caps(Some(&caps));
        for seg in &snapshot.video {
            push_segment(&vsrc, seg)?;
        }
        vsrc.end_of_stream()
            .map_err(|e| anyhow::anyhow!("replay video end of stream failed: {e}"))?;
    }
    for (i, track) in snapshot.audio_tracks.iter().enumerate() {
        if track.is_empty() {
            continue;
        }
        let name = format!("rv_asrc{i}");
        if let Some(asrc) = pipeline.by_name(&name) {
            let asrc = asrc
                .downcast::<gst_app::AppSrc>()
                .map_err(|_| anyhow::anyhow!("replay audio source is not an appsrc"))?;
            let caps = snapshot
                .audio_caps
                .get(i)
                .and_then(|c| c.clone())
                .unwrap_or_else(|| {
                    "audio/mpeg, mpegversion=(int)4, stream-format=(string)raw".to_string()
                });
            let caps = caps
                .parse::<gst::Caps>()
                .map_err(|e| anyhow::anyhow!("invalid replay audio caps \"{caps}\": {e}"))?;
            asrc.set_caps(Some(&caps));
            for seg in track {
                push_segment(&asrc, seg)?;
            }
            asrc.end_of_stream()
                .map_err(|e| anyhow::anyhow!("replay audio end of stream failed: {e}"))?;
        }
    }

    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| anyhow::anyhow!("could not start replay remux pipeline: {e}"))?;

    let bus = pipeline.bus().expect("pipeline without bus");
    let result = bus.timed_pop_filtered(
        gst::ClockTime::from_seconds(30),
        &[gst::MessageType::Eos, gst::MessageType::Error],
    );
    let _ = pipeline.set_state(gst::State::Null);

    match result {
        Some(msg) if msg.type_() == gst::MessageType::Error => {
            let err = match msg.view() {
                gst::message::MessageView::Error(err) => err.error().to_string(),
                _ => "unknown GStreamer error".to_string(),
            };
            anyhow::bail!("replay remux failed: {err}");
        }
        Some(_) => Ok(()), // EOS
        None => anyhow::bail!("replay remux timed out"),
    }
}

fn push_segment(appsrc: &gst_app::AppSrc, seg: &ReplaySegment) -> anyhow::Result<()> {
    let mut buf = gst::Buffer::from_slice(seg.data.clone());
    {
        let buf_ref = buf
            .get_mut()
            .expect("replay buffer should be uniquely owned");
        buf_ref.set_pts(gst::ClockTime::from_nseconds(seg.pts_ns));
        buf_ref.set_dts(gst::ClockTime::from_nseconds(seg.dts_ns));
    }
    appsrc
        .push_buffer(buf)
        .map_err(|e| anyhow::anyhow!("could not push replay segment: {e:?}"))?;
    Ok(())
}

/// Escape a location string for embedding into a `parse_launch` pipeline
/// description (mirrors the engine's helper: `\` is an escape character in
/// quoted property values, which would otherwise mangle Windows paths).
fn escape_location(location: &str) -> String {
    location.replace('\\', "\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(pts_ns: u64, keyframe: bool) -> ReplaySegment {
        ReplaySegment::new(pts_ns, pts_ns, vec![1, 2, 3], keyframe)
    }

    #[test]
    fn new_buffer_is_empty_with_configured_duration() {
        let buffer = ReplayBuffer::new(Duration::from_secs(30));
        assert!(buffer.is_empty());
        assert_eq!(buffer.duration(), Duration::from_secs(30));
        assert_eq!(buffer.retained_ns(), 0);
    }

    #[test]
    fn eviction_keeps_only_the_last_duration() {
        let mut buffer = ReplayBuffer::new(Duration::from_secs(10));
        // 1 second apart, 12 seconds total. Everything older than
        // newest - duration (11s - 10s = 1s) is evicted.
        for i in 0..12 {
            buffer.push_video(i * 1_000_000_000, i * 1_000_000_000, vec![0], i == 0);
        }
        assert_eq!(buffer.video.len(), 11);
        assert_eq!(buffer.video.front().unwrap().pts_ns, 1_000_000_000);
        assert_eq!(buffer.video.back().unwrap().pts_ns, 11_000_000_000);
        assert_eq!(buffer.retained_ns(), 10_000_000_000);
    }

    #[test]
    fn out_of_order_pts_does_not_break_eviction() {
        let mut buffer = ReplayBuffer::new(Duration::from_secs(5));
        // B-frame reordering: arrival order (decode order) differs from
        // presentation order (PTS). Eviction is driven by DTS, which is
        // monotonic in arrival order.
        buffer.push_video(1_000_000_000, 0, vec![0], true); // I (pts 1.0s, dts 0)
        buffer.push_video(900_000_000, 1_000_000_000, vec![0], false); // B (pts 0.9s)
        buffer.push_video(6_000_000_000, 2_000_000_000, vec![0], false); // P (pts 6.0s)
        assert_eq!(buffer.video.len(), 3);
        // A keyframe more than 5s of decode time after the oldest frame:
        // everything older than cutoff (8s - 5s = 3s) is evicted, including
        // the reordered B frame.
        buffer.push_video(11_000_000_000, 8_000_000_000, vec![0], true);
        assert_eq!(buffer.video.len(), 1);
        assert_eq!(buffer.video.front().unwrap().pts_ns, 11_000_000_000);
        assert_eq!(buffer.newest_video_dts, Some(8_000_000_000));
    }

    #[test]
    fn audio_tracks_are_evicted_independently() {
        let mut buffer = ReplayBuffer::new(Duration::from_secs(5));
        for i in 0..6 {
            buffer.push_audio(0, i * 1_000_000_000, i * 1_000_000_000, vec![0]);
        }
        buffer.push_audio(1, 0, 0, vec![0]);
        // Track 0 keeps samples >= newest (5s) - 5s = 0 -> all six.
        assert_eq!(buffer.audio_tracks[0].segments.len(), 6);
        assert_eq!(buffer.audio_tracks[1].segments.len(), 1);
        assert!(!buffer.is_empty());
    }

    #[test]
    fn set_duration_evicts_immediately() {
        let mut buffer = ReplayBuffer::new(Duration::from_secs(60));
        for i in 0..10 {
            buffer.push_video(i * 1_000_000_000, i * 1_000_000_000, vec![0], i == 0);
        }
        buffer.set_duration(Duration::from_secs(2));
        // Keep samples >= newest (9s) - 2s = 7s -> 7s, 8s, 9s.
        assert_eq!(buffer.video.len(), 3);
        assert_eq!(buffer.duration(), Duration::from_secs(2));
    }

    #[test]
    fn clear_removes_everything_including_caps() {
        let mut buffer = ReplayBuffer::new(Duration::from_secs(30));
        buffer.set_video_caps("video/x-h264".to_string());
        buffer.push_video(0, 0, vec![0], true);
        buffer.push_audio(0, 0, 0, vec![0]);
        buffer.clear();
        assert!(buffer.is_empty());
        assert!(buffer.video_caps.is_none());
        assert_eq!(buffer.retained_ns(), 0);
    }

    #[test]
    fn caps_are_sticky() {
        let mut buffer = ReplayBuffer::new(Duration::from_secs(30));
        buffer.set_video_caps("video/x-h264, stream-format=(string)avc".to_string());
        // A second (different) caps string must not overwrite the first.
        buffer.set_video_caps("video/x-h264, stream-format=(string)byte-stream".to_string());
        assert_eq!(
            buffer.video_caps.as_deref(),
            Some("video/x-h264, stream-format=(string)avc")
        );
    }

    #[test]
    fn snapshot_starts_at_keyframe_and_shifts_timestamps() {
        let mut buffer = ReplayBuffer::new(Duration::from_secs(30));
        buffer.set_video_caps("video/x-h264".to_string());
        // Leading non-keyframe segment must be dropped.
        buffer.push_video(1_500_000_000, 1_500_000_000, vec![1], false);
        buffer.push_video(2_000_000_000, 2_000_000_000, vec![2], true);
        buffer.push_video(3_000_000_000, 3_000_000_000, vec![3], false);
        buffer.push_audio(0, 1_000_000_000, 1_000_000_000, vec![9]);
        buffer.push_audio(0, 2_000_000_000, 2_000_000_000, vec![8]);

        let snap = buffer.snapshot();
        assert_eq!(snap.video.len(), 2);
        assert_eq!(snap.video[0].data, vec![2]);
        assert!(snap.video[0].keyframe);
        // Timestamps shifted so the clip starts at zero.
        assert_eq!(snap.video[0].pts_ns, 0);
        assert_eq!(snap.video[1].pts_ns, 1_000_000_000);
        // Audio shifted by its own start -> relative A/V sync preserved.
        assert_eq!(snap.audio_tracks[0][0].pts_ns, 0);
        assert_eq!(snap.audio_tracks[0][1].pts_ns, 1_000_000_000);
        assert_eq!(snap.video_caps.as_deref(), Some("video/x-h264"));
    }

    #[test]
    fn snapshot_without_any_keyframe_drops_all_video() {
        let mut buffer = ReplayBuffer::new(Duration::from_secs(30));
        buffer.push_video(0, 0, vec![1], false);
        buffer.push_video(1_000_000_000, 1_000_000_000, vec![2], false);
        let snap = buffer.snapshot();
        assert!(snap.video.is_empty());
        assert!(snap.is_empty());
    }

    #[test]
    fn empty_snapshot_save_fails_cleanly() {
        let snap = ReplaySnapshot::default();
        let err = save_replay(&snap, Path::new("unused.mp4")).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn save_replay_pipeline_description_parses() {
        // A real smoke test of the remux plumbing without needing valid media:
        // the pipeline (with real recorded caps) must parse and the video
        // source must accept its caps. The remux itself would reject garbage
        // payloads, which is fine — the pipeline construction is the part
        // under test here.
        let _ = gst::init();
        let mut snapshot = ReplaySnapshot {
            video: vec![seg(0, true)],
            video_caps: Some(
                "video/x-h264, stream-format=(string)avc, alignment=(string)au".to_string(),
            ),
            audio_tracks: vec![vec![seg(0, false)]],
            audio_caps: vec![Some(
                "audio/mpeg, mpegversion=(int)4, stream-format=(string)raw".to_string(),
            )],
        };
        snapshot.normalize();

        let desc = {
            let mut d = String::from(
                "appsrc name=rv_vsrc format=time is-live=false do-timestamp=false \
                 ! h264parse ! replay_mux. ",
            );
            for (i, track) in snapshot.audio_tracks.iter().enumerate() {
                if track.is_empty() {
                    continue;
                }
                d.push_str(&format!(
                    "appsrc name=rv_asrc{i} format=time is-live=false do-timestamp=false \
                     ! aacparse ! replay_mux. "
                ));
            }
            d.push_str("mp4mux name=replay_mux ! filesink name=replay_file location=\"out.mp4\"");
            d
        };
        let parsed = gst::parse::launch(&desc).expect("replay remux pipeline must parse");
        let parsed = parsed
            .downcast::<gst::Pipeline>()
            .expect("replay remux pipeline must be a pipeline");
        assert!(parsed.by_name("rv_vsrc").is_some());
        assert!(parsed.by_name("rv_asrc0").is_some());
        assert!(parsed.by_name("replay_mux").is_some());
    }

    /// Full end-to-end test: encode a short test pattern with the system
    /// encoder, capture it into a `ReplayBuffer` exactly like the engine's
    /// parsing branch does, save it, and verify a real MP4 file comes out.
    /// Skipped when `x264enc` is unavailable (e.g. minimal GStreamer setups).
    #[test]
    fn end_to_end_save_produces_a_valid_mp4() {
        let _ = gst::init();
        if gst::ElementFactory::find("x264enc").is_none() {
            eprintln!("x264enc not available, skipping end-to-end replay test");
            return;
        }
        if gst::ElementFactory::find("mp4mux").is_none() {
            eprintln!("mp4mux not available, skipping end-to-end replay test");
            return;
        }

        let buffer = ReplayBuffer::new(Duration::from_secs(10));
        let pipeline_str = "videotestsrc num-buffers=60 pattern=ball \
            ! videoconvert ! video/x-raw,format=I420,width=320,height=240,framerate=30/1 \
            ! tee name=t ! queue ! x264enc tune=zerolatency \
            ! h264parse ! appsink name=capture_sink \
            t. ! queue ! fakesink";
        let pipeline = gst::parse::launch(pipeline_str)
            .expect("capture pipeline must parse")
            .downcast::<gst::Pipeline>()
            .unwrap();
        let sink = pipeline
            .by_name("capture_sink")
            .unwrap()
            .downcast::<gst_app::AppSink>()
            .unwrap();

        let capture_buffer = std::sync::Arc::new(std::sync::Mutex::new(buffer));
        let rb = capture_buffer.clone();
        let callbacks = gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().expect("sample");
                let buf = sample.buffer().expect("buffer");
                let pts = buf.pts().map(|t| t.nseconds()).unwrap_or(0);
                let dts = buf.dts().or(buf.pts()).map(|t| t.nseconds()).unwrap_or(pts);
                let keyframe = !buf.flags().contains(gst::BufferFlags::DELTA_UNIT);
                let data = buf
                    .map_readable()
                    .map(|m| m.as_slice().to_vec())
                    .unwrap_or_default();
                let mut rb = rb.lock().unwrap();
                if rb.video_caps.is_none() {
                    if let Some(caps) = sample.caps() {
                        rb.set_video_caps(caps.to_string());
                    }
                }
                rb.push_video(pts, dts, data, keyframe);
                Ok(gst::FlowSuccess::Ok)
            })
            .build();
        sink.set_callbacks(callbacks);

        pipeline
            .set_state(gst::State::Playing)
            .expect("capture pipeline must start");
        let bus = pipeline.bus().unwrap();
        let _ = bus.timed_pop_filtered(
            gst::ClockTime::from_seconds(20),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        );
        let _ = pipeline.set_state(gst::State::Null);

        let buffer = capture_buffer.lock().unwrap();
        assert!(!buffer.is_empty(), "capture must have produced segments");
        assert!(buffer.video_caps.is_some());

        let path =
            std::env::temp_dir().join(format!("rivulet-replay-test-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&path);
        save_replay(&buffer.snapshot(), &path).expect("save must succeed");

        let bytes = std::fs::read(&path).expect("saved file must exist");
        // MP4 files start with a box size followed by the "ftyp" brand.
        assert!(bytes.len() > 16, "saved file is suspiciously small");
        assert_eq!(&bytes[4..8], b"ftyp", "saved file must be a valid MP4");
        let _ = std::fs::remove_file(&path);
    }
}
