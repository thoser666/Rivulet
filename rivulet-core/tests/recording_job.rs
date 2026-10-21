//! Behavior tests for the GUI-free session entry point (#294), written in
//! the same style as the `capture_backend` suite: fixtures first, then the
//! contracts a foreign consumer relies on — exact field names, exact
//! messages, exact lifecycle answers.
//!
//! The configuration-gate and pipeline-description tests never touch
//! GStreamer, so they run deterministically on every machine. The lifecycle
//! tests use the real pipeline; when a required plugin is missing they pin
//! the typed `MissingElements` refusal instead of skipping, so both
//! environment shapes stay covered.

use gstreamer as gst;
use rivulet_core::{
    AudioSettings, JobError, JobState, RecordingConfig, RecordingContainer, RecordingJob,
    RecordingJobConfig, SessionEvent, StreamSettings, VideoCodec,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Fixtures (mirrors the capture_backend suite's dummy_window/dummy_config)
// ---------------------------------------------------------------------------

/// Small even geometry: fast to encode, large enough for every codec here.
const W: u32 = 320;
const H: u32 = 240;
const FPS: u32 = 30;

/// A valid file-target configuration; override fields per test.
fn file_config() -> RecordingJobConfig {
    let recording = RecordingConfig {
        width: W,
        height: H,
        fps: FPS,
        path: Some(temp_mp4("fixture").display().to_string()),
        ..Default::default()
    };
    RecordingJobConfig::file(recording)
}

/// Deterministic stream target; the key must never survive redaction.
fn stream_settings() -> StreamSettings {
    StreamSettings::twitch("live_testkey12345")
}

/// Unique temp path per test so parallel tests never share an output file.
fn temp_mp4(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rivulet_recording_job_{tag}_{}.mp4",
        std::process::id()
    ))
}

fn rgba_frame() -> Vec<u8> {
    vec![0u8; (W * H * 4) as usize]
}

/// Poll a broadcast receiver without a Tokio runtime (broadcast's `recv` is
/// async; `try_recv` polling keeps these tests plain `#[test]`).
fn next_event(
    rx: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
    budget: Duration,
) -> Option<SessionEvent> {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        match rx.try_recv() {
            Ok(event) => return Some(event),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => return None,
        }
    }
    None
}

/// True when this GStreamer installation carries every element the session
/// pipeline needs (init first — `make` without it is meaningless).
///
/// `make`, not `find`: a stale registry entry whose plugin DLL no longer
/// loads survives `find` but cannot build a pipeline — `start` would then
/// fail with a parse error instead of the typed refusal. Only creating the
/// element proves availability.
fn session_elements_present() -> bool {
    let _ = gst::init();
    [
        "appsrc",
        "videoconvert",
        "x264enc",
        "queue",
        "mp4mux",
        "filesink",
    ]
    .into_iter()
    .all(|name| gst::ElementFactory::make(name).build().is_ok())
}

/// True when the audio branch's extra elements are available as well
/// (same `make` rule as [`session_elements_present`]).
fn audio_elements_present() -> bool {
    let _ = gst::init();
    ["audioconvert", "audioresample", "avenc_aac"]
        .into_iter()
        .all(|name| gst::ElementFactory::make(name).build().is_ok())
}

/// Shared lifecycle guard: when the host GStreamer lacks the elements a
/// session needs, the start must refuse typed — pin that refusal instead of
/// silently skipping, so a missing-plugin environment still proves the
/// contract (the suite's documented policy).
fn refuse_or_continue(config: RecordingJobConfig) {
    let err = match RecordingJob::start(config) {
        Err(err) => err,
        Ok(_) => panic!("a session with missing plugins must refuse typed, not start"),
    };
    assert!(
        matches!(err, JobError::MissingElements { .. }),
        "expected MissingElements, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate layer 1: the typed video-config contract (no GStreamer involved)
// ---------------------------------------------------------------------------

#[test]
fn zero_width_is_typed_names_its_field_and_states_the_fix() {
    let mut config = file_config();
    config.recording.width = 0;
    let err = config.validate().expect_err("width 0 must be rejected");
    assert!(
        matches!(err, JobError::Config(ref inner) if inner.field() == "width"),
        "field must name width, got {:?}",
        err.field()
    );
    assert!(
        err.to_string().contains("width"),
        "message must name the field: {err}"
    );
}

#[test]
fn odd_width_is_rejected_because_codecs_encode_macroblocks() {
    let mut config = file_config();
    config.recording.width = W + 1;
    let err = config.validate().expect_err("odd width must be rejected");
    assert!(
        matches!(err, JobError::Config(ref inner) if inner.field() == "width"),
        "field must name width, got {:?}",
        err.field()
    );
    assert!(
        err.to_string().contains("even"),
        "message must state the even-size fix: {err}"
    );
}

#[test]
fn fps_zero_and_fps_above_240_are_both_rejected_with_the_bound() {
    let mut config = file_config();
    config.recording.fps = 0;
    let err = config.validate().expect_err("fps 0 must be rejected");
    assert!(matches!(err, JobError::Config(ref inner) if inner.field() == "fps"));

    config.recording.fps = 241;
    let err = config.validate().expect_err("fps 241 must be rejected");
    assert!(
        err.to_string().contains("240"),
        "message must name the cap: {err}"
    );
}

#[test]
fn bitrate_zero_too_low_and_too_high_each_name_their_bound() {
    let mut config = file_config();

    config.recording.bitrate_kbps = 0;
    let err = config.validate().expect_err("bitrate 0 must be rejected");
    assert!(matches!(err, JobError::Config(ref inner) if inner.field() == "bitrate_kbps"));

    config.recording.bitrate_kbps = 50;
    let err = config
        .validate()
        .expect_err("50 kbps is below the usable floor");
    assert!(
        err.to_string().contains("100"),
        "message must state the minimum: {err}"
    );

    config.recording.bitrate_kbps = 500_001;
    let err = config.validate().expect_err("500 Mbps must be rejected");
    assert!(
        err.to_string().contains("500000"),
        "message must state the maximum: {err}"
    );
}

#[test]
fn codec_container_incompatibility_names_the_working_container() {
    let mut config = file_config();
    config.recording.codec = VideoCodec::VP9;
    config.recording.container = RecordingContainer::Mp4;
    let err = config.validate().expect_err("VP9 cannot ride MP4 here");
    assert!(
        matches!(err, JobError::Config(ref inner) if inner.field() == "codec" || inner.field() == "container")
    );
    assert!(
        err.to_string().contains("MKV"),
        "message must name MKV as the fix: {err}"
    );
}

#[test]
fn path_extension_mismatch_names_the_expected_extension() {
    let mut config = file_config();
    config.recording.path = Some(
        temp_mp4("mismatch")
            .display()
            .to_string()
            .replace(".mp4", ".mkv"),
    );
    let err = config
        .validate()
        .expect_err(".mkv path with MP4 container must be rejected");
    assert!(matches!(err, JobError::Config(ref inner) if inner.field() == "path"));
    assert!(
        err.to_string().contains(".mp4"),
        "message must name the expected extension: {err}"
    );
}

// ---------------------------------------------------------------------------
// Gate layer 2: the session-level parameters this entry point adds
// ---------------------------------------------------------------------------

#[test]
fn audio_rate_zero_and_out_of_range_name_the_field_and_the_bounds() {
    let mut config = file_config().with_audio(AudioSettings {
        sample_rate: 0,
        channels: 2,
    });
    let err = config.validate().expect_err("rate 0 must be rejected");
    assert_eq!(err.field(), "audio.sample_rate");
    assert!(
        matches!(err, JobError::AudioRateZero),
        "expected the dedicated zero variant, got {err:?}"
    );

    config.audio = Some(AudioSettings {
        sample_rate: 4_000,
        channels: 2,
    });
    let err = config.validate().expect_err("4 kHz is below the floor");
    assert_eq!(err.field(), "audio.sample_rate");
    assert!(
        err.to_string().contains("between 8000 and 384000"),
        "message must state the accepted range: {err}"
    );

    config.audio = Some(AudioSettings {
        sample_rate: 500_000,
        channels: 2,
    });
    let err = config.validate().expect_err("500 kHz is above the cap");
    assert!(
        err.to_string().contains("between 8000 and 384000"),
        "message must state the accepted range: {err}"
    );
}

#[test]
fn audio_channels_zero_and_above_eight_are_rejected() {
    let mut config = file_config().with_audio(AudioSettings {
        sample_rate: 48_000,
        channels: 0,
    });
    let err = config.validate().expect_err("0 channels must be rejected");
    assert_eq!(err.field(), "audio.channels");
    assert!(matches!(err, JobError::AudioChannelsZero));

    config.audio = Some(AudioSettings {
        sample_rate: 48_000,
        channels: 9,
    });
    let err = config.validate().expect_err("9 channels must be rejected");
    assert!(
        err.to_string().contains("at most 8"),
        "message must state the channel cap: {err}"
    );
}

#[test]
fn a_session_without_file_and_without_stream_is_refused_with_a_hint() {
    let mut config = file_config();
    config.recording.path = None;
    let err = config.validate().expect_err("no output must be rejected");
    assert_eq!(err.field(), "output");
    assert!(
        err.to_string().contains("with_stream"),
        "message must name how to add an output: {err}"
    );
}

#[test]
fn empty_stream_key_surfaces_the_platform_reason_verbatim() {
    let config = file_config().with_stream(StreamSettings::twitch("   "));
    let err = config.validate().expect_err("blank key must be rejected");
    assert_eq!(err.field(), "stream");
    match err {
        JobError::StreamSettings(reason) => assert_eq!(
            reason, "stream key is empty",
            "the platform's exact reason must pass through"
        ),
        other => panic!("expected StreamSettings, got {other:?}"),
    }
}

#[test]
fn non_rtmp_ingest_url_is_rejected_with_the_scheme_rule() {
    let settings = StreamSettings::new(
        rivulet_core::stream::StreamPlatform::Custom,
        "https://example.invalid/live",
        "key123",
    );
    let config = file_config().with_stream(settings);
    let err = config
        .validate()
        .expect_err("https ingest must be rejected");
    match err {
        JobError::StreamSettings(reason) => assert_eq!(
            reason, "ingest URL must use rtmp:// or rtmps://",
            "the platform's exact reason must pass through"
        ),
        other => panic!("expected StreamSettings, got {other:?}"),
    }
}

#[test]
fn streaming_with_a_non_h264_codec_is_refused_before_any_pipeline_work() {
    let mut config = file_config();
    config.recording.codec = VideoCodec::H265;
    config.recording.container = RecordingContainer::Mp4;
    let config = config.with_stream(stream_settings());
    let err = config
        .validate()
        .expect_err("H.265 cannot ride the FLV leg");
    assert_eq!(err.field(), "recording.codec");
    assert!(
        err.to_string().contains("H.264"),
        "message must name H.264 as the streaming codec: {err}"
    );
}

// ---------------------------------------------------------------------------
// Pipeline description contract (pure strings — no GStreamer required)
// ---------------------------------------------------------------------------

#[test]
fn file_pipeline_carries_the_configured_geometry_and_location() {
    let description = file_config()
        .pipeline_description()
        .expect("valid config must describe its pipeline");
    assert!(
        description.contains("appsrc name=video_src"),
        "{description}"
    );
    assert!(
        description.contains(&format!("width={W},height={H},framerate={FPS}/1")),
        "caps must carry the configured geometry: {description}"
    );
    assert!(description.contains("mp4mux"), "{description}");
    assert!(
        description.contains("filesink name=file_sink"),
        "{description}"
    );
    // The location must round-trip the configured path verbatim.
    assert!(
        description.contains("location=\"") && description.contains(".mp4"),
        "filesink must carry the output path: {description}"
    );
}

#[test]
fn h265_file_pipeline_inserts_the_parser_h264_does_not() {
    let mut config = file_config();
    config.recording.codec = VideoCodec::H265;
    config.recording.container = RecordingContainer::Mp4;
    let h265 = config
        .pipeline_description()
        .expect("H.265 into MP4 is valid");
    assert!(h265.contains("h265parse"), "H.265 needs its parser: {h265}");

    let h264 = file_config()
        .pipeline_description()
        .expect("H.264 into MP4 is valid");
    assert!(
        !h264.contains("h265parse"),
        "H.264 must not drag the H.265 parser: {h264}"
    );
}

#[test]
fn dual_output_tees_video_once_into_both_muxers() {
    let description = file_config()
        .with_stream(stream_settings())
        .pipeline_description()
        .expect("file+stream is a valid combination");
    // Encode once, split afterwards — the whole point of the dual shape.
    assert!(description.contains("tee name=video_tee"), "{description}");
    assert!(description.contains("mux_rec"), "{description}");
    assert!(description.contains("flvmux"), "{description}");
    assert!(description.contains("rtmp2sink"), "{description}");
    assert!(
        description.contains("live_testkey12345"),
        "the sink location embeds the configured key: {description}"
    );
}

#[test]
fn audio_legs_leave_a_separator_before_the_muxer_line() {
    // Regression (CI, all three platforms): the audio legs end ON the
    // muxer reference — `mux_rec.` / `mux_stream.audio` — and appending
    // the muxer line directly glued them together (`mux_rec.mp4mux`,
    // `mux_stream.audiomp4mux`, `mux_stream.flvmux`). Every GStreamer
    // parse rejects that with a bare "syntax error", so all three
    // audio-bearing shapes are pinned here as pure strings.
    let file = file_config()
        .with_default_audio()
        .pipeline_description()
        .expect("file+audio is valid");
    assert!(
        file.contains("mux_rec. mp4mux name=mux_rec"),
        "file+audio needs a separator before the muxer: {file}"
    );
    assert!(!file.contains("mux_rec.mp4mux"), "glued: {file}");

    let dual = file_config()
        .with_default_audio()
        .with_stream(stream_settings())
        .pipeline_description()
        .expect("file+audio+stream is valid");
    assert!(
        dual.contains("mux_stream.audio mp4mux name=mux_rec"),
        "dual output needs a separator after the audio tee: {dual}"
    );
    assert!(!dual.contains("mux_stream.audiomp4mux"), "glued: {dual}");

    let mut stream_only = file_config()
        .with_default_audio()
        .with_stream(stream_settings());
    stream_only.recording.path = None;
    let stream = stream_only
        .pipeline_description()
        .expect("audio+stream without a file is valid");
    assert!(
        stream.contains("mux_stream. flvmux name=mux_stream"),
        "stream-only audio needs a separator before flvmux: {stream}"
    );
    assert!(!stream.contains("mux_stream.flvmux"), "glued: {stream}");
}

#[test]
fn windows_backslashes_are_escaped_for_parse_launch() {
    let mut config = file_config();
    config.recording.path = Some(r"C:\tmp\rivulet_job_test.mp4".to_string());
    let description = config.pipeline_description().expect("valid path");
    assert!(
        description.contains(r"C:\\tmp\\rivulet_job_test.mp4"),
        "each backslash must be doubled to survive parse_launch: {description}"
    );
}

#[test]
fn the_stream_key_is_redactable_before_any_logging() {
    let description = file_config()
        .with_stream(stream_settings())
        .pipeline_description()
        .expect("valid config");
    let redacted = rivulet_core::redact_pipeline_for_log(&description);
    assert!(
        !redacted.contains("live_testkey12345"),
        "redaction must remove the stream key: {redacted}"
    );
    assert!(
        redacted.contains("rtmp"),
        "the rest of the pipeline must stay inspectable: {redacted}"
    );
}

// ---------------------------------------------------------------------------
// Lifecycle: state machine, events, Drop semantics (real pipeline)
// ---------------------------------------------------------------------------

#[test]
fn start_emits_started_with_the_path_and_reports_the_recording_state() {
    let path = temp_mp4("started");
    let mut config = file_config();
    config.recording.path = Some(path.display().to_string());

    if !session_elements_present() {
        refuse_or_continue(config);
        return;
    }

    let (job, mut events) = RecordingJob::start(config).expect("start must succeed");
    assert_eq!(job.state(), JobState::Recording);
    assert_eq!(job.state().to_string(), "recording");
    match next_event(&mut events, Duration::from_secs(2)) {
        Some(SessionEvent::Started { path: Some(p) }) => assert_eq!(p, path),
        other => panic!("expected Started with the path, got {other:?}"),
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_wrong_sized_frame_is_rejected_with_expected_and_got() {
    let mut config = file_config();
    config.recording.path = Some(temp_mp4("framesize").display().to_string());
    if !session_elements_present() {
        refuse_or_continue(config);
        return;
    }
    let (mut job, _events) = RecordingJob::start(config.clone()).expect("start");
    let err = job
        .push_frame(&[0u8; 16])
        .expect_err("wrong frame size must be rejected");
    match err {
        JobError::FrameSizeMismatch { expected, got } => {
            assert_eq!(expected, (W * H * 4) as usize);
            assert_eq!(got, 16);
        }
        other => panic!("expected FrameSizeMismatch, got {other:?}"),
    }
    // The typed message must state the fix, not just the numbers.
    assert!(err.to_string().contains("bytes per RGBA frame"));
    drop(job);
    let _ = std::fs::remove_file(temp_mp4("framesize"));
}

#[test]
fn progress_events_count_the_frames_the_embedder_pushed() {
    let mut config = file_config();
    config.recording.path = Some(temp_mp4("progress").display().to_string());
    if !session_elements_present() {
        refuse_or_continue(config);
        return;
    }
    let (mut job, mut events) = RecordingJob::start(config).expect("start");
    // Drain the Started event first so the assertion below is unambiguous.
    let _ = next_event(&mut events, Duration::from_secs(2));
    for _ in 0..5 {
        job.push_frame(&rgba_frame())
            .expect("frame must be accepted");
    }
    let progress = loop {
        match next_event(&mut events, Duration::from_secs(3)) {
            Some(SessionEvent::Progress { frames_written, .. }) if frames_written > 0 => {
                break frames_written
            }
            Some(_) => continue,
            None => panic!("no progress tick arrived"),
        }
    };
    assert!(
        progress >= 5,
        "progress must count pushed frames, got {progress}"
    );
    drop(job);
    let _ = std::fs::remove_file(temp_mp4("progress"));
}

#[test]
fn pause_rejects_pushes_with_a_typed_state_and_resume_accepts_again() {
    let mut config = file_config();
    config.recording.path = Some(temp_mp4("pause").display().to_string());
    if !session_elements_present() {
        refuse_or_continue(config);
        return;
    }
    let (mut job, mut events) = RecordingJob::start(config).expect("start");
    let _ = next_event(&mut events, Duration::from_secs(2));

    job.pause().expect("pause must work from Recording");
    assert_eq!(job.state(), JobState::Paused);
    assert_eq!(job.state().to_string(), "paused");
    match next_event(&mut events, Duration::from_secs(2)) {
        Some(SessionEvent::Paused) => {}
        other => panic!("expected Paused, got {other:?}"),
    }

    let err = job
        .push_frame(&rgba_frame())
        .expect_err("pushes must be rejected while paused");
    assert!(
        matches!(
            err,
            JobError::WrongState {
                operation: "push_frame",
                state: JobState::Paused,
            }
        ),
        "expected typed WrongState, got {err:?}"
    );
    assert!(
        err.to_string().contains("resume()"),
        "message must name the way out: {err}"
    );

    job.resume().expect("resume must work from Paused");
    assert_eq!(job.state(), JobState::Recording);
    job.push_frame(&rgba_frame())
        .expect("pushes must be accepted again");
    match next_event(&mut events, Duration::from_secs(2)) {
        Some(SessionEvent::Resumed) => {}
        other => panic!("expected Resumed, got {other:?}"),
    }
    drop(job);
    let _ = std::fs::remove_file(temp_mp4("pause"));
}

#[test]
fn stopping_a_paused_session_still_finalizes_the_output() {
    let path = temp_mp4("paused_stop");
    let mut config = file_config();
    config.recording.path = Some(path.display().to_string());
    if !session_elements_present() {
        refuse_or_continue(config);
        return;
    }
    let (mut job, _events) = RecordingJob::start(config).expect("start");
    job.push_frame(&rgba_frame()).expect("frame");
    job.pause().expect("pause from Recording");
    assert_eq!(job.state(), JobState::Paused);
    // stop() from Paused must still drive the EOS to mp4mux — otherwise
    // the file stays a crash-truncated stub without the moov atom.
    let report = job.stop().expect("stop from Paused must work");
    assert_eq!(report.frames_written, 1, "the pushed frame must be counted");
    let bytes = std::fs::read(&path).expect("a paused session must leave a file behind");
    assert!(
        bytes.windows(4).any(|w| w == b"moov"),
        "the muxer must finalize even when the session was paused at stop"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn stop_consumes_the_handle_and_reports_the_session_counters() {
    let path = temp_mp4("stop");
    let mut config = file_config();
    config.recording.path = Some(path.display().to_string());
    if !session_elements_present() {
        refuse_or_continue(config);
        return;
    }
    let (mut job, mut events) = RecordingJob::start(config).expect("start");
    let _ = next_event(&mut events, Duration::from_secs(2));
    for _ in 0..3 {
        job.push_frame(&rgba_frame()).expect("frame");
    }
    let report = job.stop().expect("clean stop");
    assert_eq!(
        report.frames_written, 3,
        "report must count the pushed frames"
    );
    assert_eq!(report.path.as_deref(), Some(path.as_path()));
    match next_event(&mut events, Duration::from_secs(2)) {
        Some(SessionEvent::Stopped(stopped)) => assert_eq!(stopped.frames_written, 3),
        other => panic!("expected Stopped event, got {other:?}"),
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn drop_finalizes_the_output_file_the_same_way_stop_does() {
    let path = temp_mp4("drop");
    let mut config = file_config();
    config.recording.path = Some(path.display().to_string());
    if !session_elements_present() {
        refuse_or_continue(config);
        return;
    }
    {
        let (mut job, _events) = RecordingJob::start(config).expect("start");
        for _ in 0..3 {
            job.push_frame(&rgba_frame()).expect("frame");
        }
        // No stop(): the documented decision is that dropping the handle
        // stops and finalizes the recording.
        drop(job);
    }
    let bytes = std::fs::read(&path).expect("drop must leave a file behind");
    assert!(!bytes.is_empty(), "the finalized file must not be empty");
    assert!(
        bytes.windows(4).any(|w| w == b"moov"),
        "the muxer must have written the moov atom (finalized MP4), \
         otherwise the file is a crash-truncated stub"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn subscribers_see_the_events_sent_after_they_subscribe() {
    let mut config = file_config();
    config.recording.path = Some(temp_mp4("subscribe").display().to_string());
    if !session_elements_present() {
        refuse_or_continue(config);
        return;
    }
    let (mut job, mut first) = RecordingJob::start(config).expect("start");
    let _ = next_event(&mut first, Duration::from_secs(2)); // drain Started

    let mut second = job.subscribe();
    job.push_frame(&rgba_frame()).expect("frame");
    // The late subscriber must not see the past Started event, but live ones.
    match next_event(&mut second, Duration::from_secs(3)) {
        Some(SessionEvent::Progress { .. }) => {}
        other => expect_progress_or_fail(other),
    }
    drop(job);
    let _ = std::fs::remove_file(temp_mp4("subscribe"));
}

fn expect_progress_or_fail(event: Option<SessionEvent>) -> ! {
    panic!("late subscriber must see a live event, got {event:?}");
}

#[test]
fn audio_and_video_are_pushed_and_reported_in_one_session() {
    let mut config = file_config().with_default_audio();
    config.recording.path = Some(temp_mp4("av").display().to_string());
    if !session_elements_present() || !audio_elements_present() {
        refuse_or_continue(config);
        return;
    }
    let (mut job, _events) = RecordingJob::start(config).expect("start");
    job.push_frame(&rgba_frame()).expect("video frame");
    // One frame's worth of interleaved stereo F32 at the configured rate.
    job.push_audio(&vec![0.0f32; (48_000 / FPS * 2) as usize])
        .expect("audio must be accepted alongside video");
    let report = job.stop().expect("stop");
    assert_eq!(report.frames_written, 1, "one video frame was pushed");
    assert_eq!(
        report.audio_samples,
        (48_000 / FPS * 2) as u64,
        "report must count interleaved audio samples"
    );
    let _ = std::fs::remove_file(temp_mp4("av"));
}

#[test]
fn pushing_audio_without_an_audio_branch_is_a_typed_wrong_state() {
    let mut config = file_config(); // video only — no audio branch
    config.recording.path = Some(temp_mp4("noaudio").display().to_string());
    if !session_elements_present() {
        refuse_or_continue(config);
        return;
    }
    let (mut job, _events) = RecordingJob::start(config).expect("start");
    let err = job
        .push_audio(&[0.0f32; 320])
        .expect_err("audio without a branch must be rejected");
    assert!(
        matches!(
            err,
            JobError::WrongState {
                operation: "push_audio",
                ..
            }
        ),
        "expected typed WrongState, got {err:?}"
    );
    drop(job);
    let _ = std::fs::remove_file(temp_mp4("noaudio"));
}
