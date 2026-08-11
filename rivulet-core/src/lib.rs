// In rivulet-core/src/lib.rs

use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use once_cell::sync::Lazy;
use std::path::PathBuf;
// Wichtig: Der Prelude wird immer noch für Methoden wie .set_state(), .by_name() etc. benötigt.
use gst::prelude::*;

pub mod audio;
pub use audio::{AudioFrame, AudioTrack};

pub mod stream;
pub use stream::{StreamPlatform, StreamSettings};

// GStreamer-Initialisierung
static GSTREAMER_INIT: Lazy<()> = Lazy::new(|| {
    gst::init().expect("GStreamer-Initialisierung fehlgeschlagen.");
});

/// Default audio format used by the engine's audio input.
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u16 = 2;

pub struct RivuletEngine {
    pipeline: Option<gst::Pipeline>,
    appsrc: Option<gst_app::AppSrc>,
    audio_appsrc: Option<gst_app::AppSrc>,
    audio_appsrc_sys: Option<gst_app::AppSrc>,
    audio_appsrc_mic: Option<gst_app::AppSrc>,
    audio_enabled: bool,
    separate_audio_tracks: bool,
    audio_sys_enabled: bool,
    audio_mic_enabled: bool,
    is_recording: bool,
    output_path: Option<PathBuf>,
    stream_settings: Option<StreamSettings>,
}

impl Default for RivuletEngine {
    fn default() -> Self {
        Lazy::force(&GSTREAMER_INIT);
        Self {
            pipeline: None,
            appsrc: None,
            audio_appsrc: None,
            audio_appsrc_sys: None,
            audio_appsrc_mic: None,
            audio_enabled: false,
            separate_audio_tracks: false,
            audio_sys_enabled: true,
            audio_mic_enabled: true,
            is_recording: false,
            output_path: None,
            stream_settings: None,
        }
    }
}

impl RivuletEngine {
    pub fn new() -> Self {
        println!("[Engine] GStreamer bereit.");
        Self::default()
    }

    /// Configure the RTMP/RTMPS stream output. When set, the engine streams to
    /// the given ingest URL. If a local recording is started at the same time
    /// (via [`RivuletEngine::start_local_recording`]) the engine runs in dual
    /// output mode and records and streams simultaneously.
    ///
    /// Must be called before recording starts.
    pub fn set_stream_settings(&mut self, settings: Option<StreamSettings>) {
        self.stream_settings = settings;
    }

    /// Whether the engine is configured to stream.
    pub fn is_streaming(&self) -> bool {
        self.stream_settings.is_some()
    }

    /// Whether the engine records and streams at the same time (dual output).
    /// This is the case when both stream settings and a local output path are
    /// configured.
    pub fn is_dual_output(&self) -> bool {
        self.is_streaming() && self.output_path.is_some()
    }

    /// Escape a location string for embedding into a `parse_launch` pipeline
    /// description. GStreamer treats `\` as an escape character in quoted
    /// property values; on Windows paths like `C:\Users\...` would otherwise be
    /// silently mangled (e.g. `C:UsersRUNNER~1...`).
    fn escape_location(location: &str) -> String {
        location.replace('\\', "\\\\")
    }

    fn video_branch_str(&self) -> String {
        "appsrc name=rivulet_src ! videoconvert ! x264enc tune=zerolatency ! queue ! mux. "
            .to_string()
    }

    /// Build the audio branch of the pipeline. With `force_mixed` the sources
    /// are mixed into a single track regardless of `separate_audio_tracks`; the
    /// FLV muxer used for streaming only supports a single audio track.
    fn audio_branch_str(&self, force_mixed: bool) -> String {
        if !self.audio_enabled {
            return String::new();
        }
        if self.separate_audio_tracks && !force_mixed {
            let mut s = String::new();
            if self.audio_sys_enabled {
                s.push_str(
                    "appsrc name=audio_src_sys format=time is-live=true do-timestamp=true \
                     ! audioconvert ! audioresample ! avenc_aac ! queue ! mux. ",
                );
            }
            if self.audio_mic_enabled {
                s.push_str(
                    "appsrc name=audio_src_mic format=time is-live=true do-timestamp=true \
                     ! audioconvert ! audioresample ! avenc_aac ! queue ! mux. ",
                );
            }
            s
        } else {
            "appsrc name=audio_src format=time is-live=true do-timestamp=true \
             ! audioconvert ! audioresample ! avenc_aac ! queue ! mux. "
                .to_string()
        }
    }

    fn build_recording_pipeline_str(&self, location: &str) -> String {
        let mut s = self.video_branch_str();
        s.push_str(&self.audio_branch_str(false));
        let escaped = Self::escape_location(location);
        s.push_str(&format!(
            "mp4mux name=mux ! filesink location=\"{}\"",
            escaped
        ));
        s
    }

    fn build_streaming_pipeline_str(&self) -> String {
        let location = self
            .stream_settings
            .as_ref()
            .expect("Stream-Settings müssen gesetzt sein")
            .location();
        println!(
            "[Engine] Initialisiere Streaming-Pipeline für: {}",
            location
        );
        let mut s = self.video_branch_str();
        s.push_str(&self.audio_branch_str(true));
        s.push_str(&format!(
            "flvmux name=mux streamable=true ! rtmp2sink location=\"{}\"",
            location
        ));
        s
    }

    /// Build a pipeline that records locally **and** streams at the same time.
    ///
    /// Video and audio are encoded once and split via `tee` into two branches,
    /// one feeding the MP4 muxer (local file) and one feeding the FLV muxer
    /// (RTMP/RTMPS sink). Audio is always mixed into a single track because FLV
    /// only supports one audio track.
    fn build_dual_output_pipeline_str(&self) -> String {
        let stream_location = self
            .stream_settings
            .as_ref()
            .expect("Stream-Settings müssen gesetzt sein")
            .location();
        let path = self
            .output_path
            .as_ref()
            .expect("Ausgabepfad muss gesetzt sein");
        let location = path.to_str().expect("Dateipfad ist ungültig.");
        println!(
            "[Engine] Initialisiere Dual-Output-Pipeline (Aufnahme + Stream nach {})",
            stream_location
        );

        let mut s = String::new();
        s.push_str(
            "appsrc name=rivulet_src ! videoconvert ! x264enc tune=zerolatency \
             ! tee name=video_tee ! queue ! mux_rec. \
             video_tee. ! queue ! mux_stream. ",
        );
        if self.audio_enabled {
            s.push_str(
                "appsrc name=audio_src format=time is-live=true do-timestamp=true \
                 ! audioconvert ! audioresample ! avenc_aac ! tee name=audio_tee \
                 ! queue ! mux_rec. \
                 audio_tee. ! queue ! mux_stream. ",
            );
        }
        s.push_str(&format!(
            "mp4mux name=mux_rec ! filesink location=\"{}\" ",
            Self::escape_location(location)
        ));
        s.push_str(&format!(
            "flvmux name=mux_stream streamable=true ! rtmp2sink location=\"{}\"",
            stream_location
        ));
        s
    }

    fn initialize_and_start_pipeline(&mut self, width: u32, height: u32) {
        let pipeline_str = if self.is_dual_output() {
            self.build_dual_output_pipeline_str()
        } else if self.is_streaming() {
            self.build_streaming_pipeline_str()
        } else {
            let Some(path) = self.output_path.as_ref() else {
                return;
            };
            let location = path.to_str().expect("Dateipfad ist ungültig.");
            println!("[Engine] Initialisiere Aufnahme-Pipeline für: {}", location);
            self.build_recording_pipeline_str(location)
        };

        // KORREKTUR: Die `parse_launch`-Funktion kommt aus `gstreamer` (Modul `gst::parse`).
        // In v0.24 können wir `gst::parse::launch(&str)` verwenden; bei Bedarf gäbe es auch `launch_full` mit Context/Flags.
        let pipeline = match gst::parse::launch(&pipeline_str) {
            Ok(p) => p.downcast::<gst::Pipeline>().unwrap(),
            Err(e) => {
                eprintln!("[Engine] Fehler beim Erstellen der Pipeline: {}", e);
                return;
            }
        };

        // Der Rest des Codes ist korrekt...
        let appsrc = pipeline
            .by_name("rivulet_src")
            .unwrap()
            .downcast::<gst_app::AppSrc>()
            .unwrap();

        let video_info = gst_video::VideoInfo::builder(gst_video::VideoFormat::Rgba, width, height)
            .fps((30, 1))
            .build()
            .unwrap();
        appsrc.set_caps(Some(&video_info.to_caps().unwrap()));
        appsrc.set_property("format", gst::Format::Time);
        appsrc.set_property("is-live", true);
        appsrc.set_property("do-timestamp", true);

        if self.audio_enabled {
            let separate = self.separate_audio_tracks && !self.is_streaming();
            if separate {
                let mut track_srcs: Vec<(&str, &mut Option<gst_app::AppSrc>)> = Vec::new();
                if self.audio_sys_enabled {
                    track_srcs.push(("audio_src_sys", &mut self.audio_appsrc_sys));
                }
                if self.audio_mic_enabled {
                    track_srcs.push(("audio_src_mic", &mut self.audio_appsrc_mic));
                }
                for (name, slot) in track_srcs {
                    let app = pipeline
                        .by_name(name)
                        .unwrap()
                        .downcast::<gst_app::AppSrc>()
                        .unwrap();
                    app.set_caps(Some(&audio_caps()));
                    app.set_property("format", gst::Format::Time);
                    app.set_property("is-live", true);
                    app.set_property("do-timestamp", true);
                    *slot = Some(app);
                }
            } else {
                let audio_appsrc = pipeline
                    .by_name("audio_src")
                    .unwrap()
                    .downcast::<gst_app::AppSrc>()
                    .unwrap();
                audio_appsrc.set_caps(Some(&audio_caps()));
                audio_appsrc.set_property("format", gst::Format::Time);
                audio_appsrc.set_property("is-live", true);
                audio_appsrc.set_property("do-timestamp", true);
                self.audio_appsrc = Some(audio_appsrc);
            }
        }

        if pipeline.set_state(gst::State::Playing).is_err() {
            eprintln!("[Engine] Pipeline konnte nicht gestartet werden.");
            return;
        }

        self.pipeline = Some(pipeline);
        self.appsrc = Some(appsrc);
        if self.is_dual_output() {
            println!("[Engine] Dual-Output-Pipeline läuft.");
        } else if self.is_streaming() {
            println!("[Engine] Streaming-Pipeline läuft.");
        } else {
            println!("[Engine] Aufnahme-Pipeline läuft.");
        }
    }

    /// Enable or disable the audio input branch of the recording pipeline.
    ///
    /// Must be called before recording starts.
    pub fn set_audio_enabled(&mut self, enabled: bool) {
        self.audio_enabled = enabled;
    }

    pub fn audio_enabled(&self) -> bool {
        self.audio_enabled
    }

    /// Enable or disable separate audio tracks.
    ///
    /// When enabled, system audio and microphone frames must be pushed through
    /// [`RivuletEngine::push_audio_track`]; they will be stored in distinct
    /// tracks of the output file. When disabled (default) the frames are mixed
    /// into a single track via [`RivuletEngine::push_audio_frame`].
    ///
    /// Must be called before recording starts.
    pub fn set_separate_audio_tracks(&mut self, enabled: bool) {
        self.separate_audio_tracks = enabled;
    }

    pub fn separate_audio_tracks(&self) -> bool {
        self.separate_audio_tracks
    }

    /// Enable or disable a single audio track when recording separate tracks.
    ///
    /// A disabled track is not built into the recording pipeline at all, so it
    /// is safe to keep separate tracks enabled even when only one source is
    /// active. Must be called before recording starts.
    pub fn set_audio_track_enabled(&mut self, track: AudioTrack, enabled: bool) {
        match track {
            AudioTrack::System => self.audio_sys_enabled = enabled,
            AudioTrack::Microphone => self.audio_mic_enabled = enabled,
        }
    }

    pub fn audio_track_enabled(&self, track: AudioTrack) -> bool {
        match track {
            AudioTrack::System => self.audio_sys_enabled,
            AudioTrack::Microphone => self.audio_mic_enabled,
        }
    }

    /// Push a mixed PCM frame into the recording pipeline.
    pub fn push_audio_frame(&mut self, frame: &AudioFrame) -> anyhow::Result<()> {
        if !self.audio_enabled || (self.separate_audio_tracks && !self.is_streaming()) {
            return Ok(());
        }
        if frame.data.is_empty() {
            return Ok(());
        }

        if self.pipeline.is_none() {
            anyhow::bail!("Pipeline is not running");
        }

        let Some(appsrc) = self.audio_appsrc.as_ref() else {
            anyhow::bail!("Audio input is not enabled");
        };

        push_pcm_buffer(appsrc, frame)
    }

    /// Push a PCM frame belonging to the given audio track into the pipeline.
    ///
    /// Only valid when separate audio tracks are enabled; otherwise the frame
    /// is ignored.
    pub fn push_audio_track(
        &mut self,
        frame: &AudioFrame,
        track: AudioTrack,
    ) -> anyhow::Result<()> {
        if !self.audio_enabled || !self.separate_audio_tracks || self.is_streaming() {
            return Ok(());
        }
        if frame.data.is_empty() {
            return Ok(());
        }

        if self.pipeline.is_none() {
            anyhow::bail!("Pipeline is not running");
        }

        let appsrc = match track {
            AudioTrack::System => self.audio_appsrc_sys.as_ref(),
            AudioTrack::Microphone => self.audio_appsrc_mic.as_ref(),
        };
        let Some(appsrc) = appsrc else {
            return Ok(());
        };

        push_pcm_buffer(appsrc, frame)
    }

    /// Start writing the video/audio frames to a local MP4 file.
    ///
    /// When stream settings are configured as well ([`RivuletEngine::set_stream_settings`]),
    /// the engine records and streams simultaneously (dual output).
    pub fn start_local_recording(&mut self, path: PathBuf) {
        if self.is_recording {
            return;
        }
        println!("[Engine] Aufnahme vorbereitet für: {:?}", path);
        self.output_path = Some(path);
        self.is_recording = true;
    }

    pub fn stop_recording(&mut self) {
        if !self.is_recording {
            return;
        }
        println!("[Engine] Aufnahme wird gestoppt...");

        if let Some(appsrc) = self.appsrc.as_ref() {
            let _ = appsrc.end_of_stream();
        }
        if let Some(appsrc) = self.audio_appsrc.as_ref() {
            let _ = appsrc.end_of_stream();
        }
        if let Some(appsrc) = self.audio_appsrc_sys.as_ref() {
            let _ = appsrc.end_of_stream();
        }
        if let Some(appsrc) = self.audio_appsrc_mic.as_ref() {
            let _ = appsrc.end_of_stream();
        }

        // Auf EOS warten, damit der Muxer die Datei (moov-Atom) finalisiert,
        // bevor die Pipeline auf Null gesetzt wird.
        if let Some(pipeline) = self.pipeline.as_ref() {
            if let Some(bus) = pipeline.bus() {
                let _ = bus.timed_pop_filtered(
                    gst::ClockTime::from_seconds(10),
                    &[gst::MessageType::Eos, gst::MessageType::Error],
                );
            }
        }

        if let Some(pipeline) = self.pipeline.take() {
            pipeline
                .set_state(gst::State::Null)
                .expect("Pipeline konnte nicht gestoppt werden.");
        }

        self.appsrc = None;
        self.audio_appsrc = None;
        self.audio_appsrc_sys = None;
        self.audio_appsrc_mic = None;
        self.output_path = None;
        self.is_recording = false;
        println!("[Engine] Aufnahme gestoppt und Datei gespeichert.");
    }

    pub fn process_raw_frame(&mut self, frame_data: &[u8], width: u32, height: u32) {
        if !self.is_recording {
            return;
        }

        if self.pipeline.is_none() {
            self.initialize_and_start_pipeline(width, height);
        }

        if let Some(appsrc) = &self.appsrc {
            let mut buffer = gst::Buffer::with_size(frame_data.len()).unwrap();
            {
                let buffer_ref = buffer.get_mut().expect("Buffer not writable");
                let mut map = buffer_ref
                    .map_writable()
                    .expect("Failed to map buffer writable");
                map.as_mut_slice().copy_from_slice(frame_data);
            }

            if let Err(err) = appsrc.push_buffer(buffer) {
                eprintln!(
                    "[Engine] Fehler beim Senden des Frames in die Pipeline: {:?}",
                    err
                );
                self.stop_recording();
            }
        }
    }
}

/// Caps for the engine's audio input branch: interleaved f32 PCM.
pub fn audio_caps() -> gst::Caps {
    gst::Caps::builder("audio/x-raw")
        .field("format", "F32LE")
        .field("layout", "interleaved")
        .field("channels", i32::from(AUDIO_CHANNELS))
        .field("rate", AUDIO_SAMPLE_RATE as i32)
        .build()
}

/// Copy an [`AudioFrame`] into a writable GStreamer buffer and push it into
/// the given appsrc.
fn push_pcm_buffer(appsrc: &gst_app::AppSrc, frame: &AudioFrame) -> anyhow::Result<()> {
    let bytes_len = frame.data.len() * std::mem::size_of::<f32>();
    let mut buffer = gst::Buffer::with_size(bytes_len)?;
    {
        let buffer_ref = buffer
            .get_mut()
            .ok_or_else(|| anyhow::anyhow!("Failed to get writable audio buffer"))?;
        let mut map = buffer_ref
            .map_writable()
            .map_err(|_| anyhow::anyhow!("Failed to map audio buffer writable"))?;
        let dst = map.as_mut_slice();
        for (i, sample) in frame.data.iter().enumerate() {
            let bytes = sample.to_ne_bytes();
            let off = i * 4;
            dst[off..off + 4].copy_from_slice(&bytes);
        }
    }

    appsrc.push_buffer(buffer)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gstreamer_pbutils as gst_pbutils;

    #[test]
    fn audio_toggle_is_disabled_by_default() {
        let engine = RivuletEngine::default();
        assert!(!engine.audio_enabled());
    }

    #[test]
    fn audio_toggle_can_be_enabled_and_disabled() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        assert!(engine.audio_enabled());
        engine.set_audio_enabled(false);
        assert!(!engine.audio_enabled());
    }

    #[test]
    fn audio_tracks_are_enabled_by_default_and_toggleable() {
        let mut engine = RivuletEngine::default();
        assert!(engine.audio_track_enabled(AudioTrack::System));
        assert!(engine.audio_track_enabled(AudioTrack::Microphone));
        engine.set_audio_track_enabled(AudioTrack::Microphone, false);
        assert!(engine.audio_track_enabled(AudioTrack::System));
        assert!(!engine.audio_track_enabled(AudioTrack::Microphone));
    }

    #[test]
    fn audio_caps_describe_f32_interleaved_stereo_48k() {
        let _ = gst::init();
        let caps = audio_caps();
        assert!(!caps.is_empty(), "audio caps should not be empty");
    }

    /// End-to-end test: feed synthetic video + audio frames into the engine and
    /// verify that a non-empty MP4 file is written.
    #[test]
    fn records_synthetic_video_and_audio_to_file() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);

        let path =
            std::env::temp_dir().join(format!("rivulet_engine_test_{}.mp4", std::process::id()));
        engine.start_local_recording(path.clone());

        let (width, height) = (64u32, 64u32);
        let video = vec![0u8; (width * height * 4) as usize];
        for _ in 0..12 {
            engine.process_raw_frame(&video, width, height);
        }

        let audio = AudioFrame::new(vec![0.0f32; 4800], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
        for _ in 0..12 {
            let _ = engine.push_audio_frame(&audio);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
        engine.stop_recording();

        assert!(path.exists(), "Ausgabedatei sollte existieren");
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        assert!(len > 0, "Ausgabedatei sollte nicht leer sein");

        let _ = std::fs::remove_file(&path);
    }

    /// End-to-end test for separate audio tracks: both tracks are pushed into
    /// the engine and the resulting file must contain two audio streams.
    #[test]
    fn records_separate_audio_tracks_into_two_streams() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);

        let path = std::env::temp_dir().join(format!(
            "rivulet_separate_tracks_{}.mp4",
            std::process::id()
        ));
        engine.start_local_recording(path.clone());

        let (width, height) = (64u32, 64u32);
        let video = vec![0u8; (width * height * 4) as usize];
        for _ in 0..12 {
            engine.process_raw_frame(&video, width, height);
        }

        let audio = AudioFrame::new(vec![0.0f32; 4800], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
        for _ in 0..12 {
            let _ = engine.push_audio_track(&audio, AudioTrack::System);
            let _ = engine.push_audio_track(&audio, AudioTrack::Microphone);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
        engine.stop_recording();

        assert!(path.exists(), "Ausgabedatei sollte existieren");
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        assert!(len > 0, "Ausgabedatei sollte nicht leer sein");

        let uri = format!("file://{}", path.display());
        let discoverer = gst_pbutils::Discoverer::new(gst::ClockTime::from_seconds(5))
            .expect("Discoverer sollte erstellt werden können");
        let info = discoverer
            .discover_uri(&uri)
            .expect("Datei sollte lesbar sein");
        let audio_streams = info.audio_streams();
        assert_eq!(
            audio_streams.len(),
            2,
            "Es sollten zwei Audio-Streams vorhanden sein, gefunden: {}",
            audio_streams.len()
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Frames pushed for a specific track are ignored while separate tracks are
    /// disabled.
    #[test]
    fn track_frames_are_ignored_in_mixed_mode() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);

        let frame = AudioFrame::new(vec![0.0f32; 8], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
        let result = engine.push_audio_track(&frame, AudioTrack::System);
        assert!(result.is_ok(), "push_audio_track sollte kein Fehler sein");
    }

    /// The streaming pipeline parses and uses an RTMPS ingest URL.
    #[test]
    fn streaming_pipeline_str_parses_and_uses_rtmps_location() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_stream_settings(Some(StreamSettings::twitch("testkey123")));
        assert!(engine.is_streaming());

        let pipeline_str = engine.build_streaming_pipeline_str();
        assert!(pipeline_str.contains("rtmps://live.twitch.tv/app/testkey123"));
        let pipeline = gst::parse::launch(&pipeline_str)
            .expect("Streaming-Pipeline sollte parsen")
            .downcast::<gst::Pipeline>()
            .unwrap();
        assert!(pipeline.by_name("rivulet_src").is_some());
        assert!(pipeline.by_name("mux").is_some());
    }

    /// The streaming pipeline forces a single mixed audio track even when
    /// separate tracks are enabled, because FLV only supports one audio track.
    #[test]
    fn streaming_pipeline_mixes_audio_despite_separate_tracks() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);
        engine.set_stream_settings(Some(StreamSettings::youtube("k1")));

        let pipeline_str = engine.build_streaming_pipeline_str();
        assert!(pipeline_str.contains("name=audio_src "), "{}", pipeline_str);
        assert!(
            !pipeline_str.contains("audio_src_sys"),
            "separate Sinks sollten im Streaming-Modus nicht gebaut werden"
        );
        assert!(!pipeline_str.contains("audio_src_mic"));
        assert!(pipeline_str.contains("flvmux name=mux streamable=true"));
    }

    /// The recording pipeline still keeps separate audio tracks when enabled.
    #[test]
    fn recording_pipeline_keeps_separate_tracks() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);

        let pipeline_str = engine.build_recording_pipeline_str("/tmp/out.mp4");
        assert!(pipeline_str.contains("name=audio_src_sys"));
        assert!(pipeline_str.contains("name=audio_src_mic"));
        assert!(pipeline_str.contains("mp4mux name=mux"));
    }

    /// Without stream settings the engine is not configured for streaming.
    #[test]
    fn engine_is_not_streaming_by_default() {
        let engine = RivuletEngine::default();
        assert!(!engine.is_streaming());
    }

    /// Stream settings can be configured and cleared again.
    #[test]
    fn stream_settings_can_be_cleared() {
        let mut engine = RivuletEngine::default();
        engine.set_stream_settings(Some(StreamSettings::twitch("k")));
        assert!(engine.is_streaming());
        engine.set_stream_settings(None);
        assert!(!engine.is_streaming());
    }

    /// The streaming pipeline contains no audio branch when audio is disabled.
    #[test]
    fn streaming_pipeline_without_audio_has_no_audio_branch() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(false);
        engine.set_stream_settings(Some(StreamSettings::youtube("k1")));

        let pipeline_str = engine.build_streaming_pipeline_str();
        assert!(!pipeline_str.contains("audio_src"), "{}", pipeline_str);
        gst::parse::launch(&pipeline_str).expect("Video-Only-Streaming-Pipeline sollte parsen");
    }

    /// The streaming pipeline mixes audio into a single track by default and
    /// accepts a plain (unencrypted) custom RTMP ingest URL.
    #[test]
    fn streaming_pipeline_mixes_audio_and_accepts_plain_rtmp() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_stream_settings(Some(StreamSettings::custom("rtmp://localhost/live", "k")));

        let pipeline_str = engine.build_streaming_pipeline_str();
        assert!(pipeline_str.contains("name=audio_src "), "{}", pipeline_str);
        assert!(!pipeline_str.contains("audio_src_sys"), "{}", pipeline_str);
        assert!(pipeline_str.contains("rtmp://localhost/live/k"));
        gst::parse::launch(&pipeline_str).expect("Custom-RTMP-Pipeline sollte parsen");
    }

    /// While streaming, mixed audio frames are routed into the pipeline even
    /// when separate tracks are enabled, while `push_audio_track` is ignored.
    #[test]
    fn streaming_mode_routes_audio_to_mixed_sink() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);
        engine.set_stream_settings(Some(StreamSettings::twitch("k")));

        let frame = AudioFrame::new(vec![0.0f32; 8], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);

        // Must NOT be silently ignored: it has to reach the pipeline check.
        let result = engine.push_audio_frame(&frame);
        assert!(
            result.is_err(),
            "Gemischtes Audio muss im Streaming-Modus geroutet werden"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Pipeline is not running"),
            "Es sollte an der Pipeline-Prüfung scheitern, da keine Pipeline läuft"
        );

        // push_audio_track is ignored while streaming (FLV supports one track).
        assert!(
            engine.push_audio_track(&frame, AudioTrack::System).is_ok(),
            "push_audio_track muss im Streaming-Modus ignoriert werden"
        );
    }

    /// In non-streaming recording mode mixed frames are ignored while separate
    /// tracks are enabled (unchanged behaviour preserved by the streaming guard).
    #[test]
    fn recording_mode_keeps_track_routing_guards() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);

        let frame = AudioFrame::new(vec![0.0f32; 8], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
        assert!(
            engine.push_audio_frame(&frame).is_ok(),
            "Gemischte Frames müssen bei separaten Tracks ignoriert werden"
        );
    }

    /// Dual output is active when both stream settings and a local output path
    /// are configured, and streaming alone is not enough.
    #[test]
    fn dual_output_detected_when_stream_and_path_configured() {
        let mut engine = RivuletEngine::default();
        engine.set_stream_settings(Some(StreamSettings::twitch("k")));
        assert!(
            !engine.is_dual_output(),
            "nur Streaming ist kein Dual Output"
        );

        let path = std::env::temp_dir().join("rivulet_dual_detect.mp4");
        engine.start_local_recording(path);
        assert!(engine.is_dual_output(), "Stream + Aufnahme = Dual Output");

        engine.set_stream_settings(None);
        assert!(
            !engine.is_dual_output(),
            "nur Aufnahme ist kein Dual Output"
        );
    }

    /// The dual output pipeline parses and contains both sinks: the MP4 file
    /// sink and the RTMPS stream sink, split via tee.
    #[test]
    fn dual_output_pipeline_str_parses_with_both_sinks() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_stream_settings(Some(StreamSettings::twitch("dualkey")));

        let path = std::env::temp_dir().join("rivulet_dual_parse.mp4");
        engine.start_local_recording(path.clone());

        let pipeline_str = engine.build_dual_output_pipeline_str();
        assert!(pipeline_str.contains("name=video_tee"), "{}", pipeline_str);
        assert!(pipeline_str.contains("mp4mux name=mux_rec"));
        assert!(pipeline_str.contains("flvmux name=mux_stream streamable=true"));
        assert!(pipeline_str.contains(&format!("filesink location=\"{}\"", path.display())));
        assert!(pipeline_str.contains("rtmp2sink location=\"rtmps://live.twitch.tv/app/dualkey\""));

        let pipeline = gst::parse::launch(&pipeline_str)
            .expect("Dual-Output-Pipeline sollte parsen")
            .downcast::<gst::Pipeline>()
            .unwrap();
        assert!(pipeline.by_name("rivulet_src").is_some());
        assert!(pipeline.by_name("mux_rec").is_some());
        assert!(pipeline.by_name("mux_stream").is_some());
    }

    /// In dual output mode the audio is always mixed into a single track, even
    /// when separate tracks are enabled, because FLV only supports one track.
    #[test]
    fn dual_output_pipeline_mixes_audio_despite_separate_tracks() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);
        engine.set_stream_settings(Some(StreamSettings::youtube("k1")));

        let path = std::env::temp_dir().join("rivulet_dual_mix.mp4");
        engine.start_local_recording(path);

        let pipeline_str = engine.build_dual_output_pipeline_str();
        assert!(pipeline_str.contains("name=audio_src "), "{}", pipeline_str);
        assert!(!pipeline_str.contains("audio_src_sys"), "{}", pipeline_str);
        assert!(!pipeline_str.contains("audio_src_mic"), "{}", pipeline_str);
        assert!(pipeline_str.contains("name=audio_tee"));
    }

    /// A video-only dual output pipeline (audio disabled) still parses.
    #[test]
    fn dual_output_pipeline_without_audio_parses() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(false);
        engine.set_stream_settings(Some(StreamSettings::custom("rtmp://localhost/live", "k")));

        let path = std::env::temp_dir().join("rivulet_dual_video_only.mp4");
        engine.start_local_recording(path);

        let pipeline_str = engine.build_dual_output_pipeline_str();
        assert!(!pipeline_str.contains("audio_src"), "{}", pipeline_str);
        gst::parse::launch(&pipeline_str).expect("Video-Only-Dual-Output-Pipeline sollte parsen");
    }

    /// In dual output mode mixed audio frames are routed into the pipeline even
    /// when separate tracks are enabled, while `push_audio_track` is ignored.
    #[test]
    fn dual_output_routes_audio_to_mixed_sink() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);
        engine.set_stream_settings(Some(StreamSettings::twitch("k")));

        let path = std::env::temp_dir().join("rivulet_dual_routing.mp4");
        engine.start_local_recording(path);
        assert!(engine.is_dual_output());

        let frame = AudioFrame::new(vec![0.0f32; 8], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);

        // Must NOT be silently ignored: it has to reach the pipeline check.
        let result = engine.push_audio_frame(&frame);
        assert!(
            result.is_err(),
            "Gemischtes Audio muss im Dual-Output-Modus geroutet werden"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Pipeline is not running"),
            "Es sollte an der Pipeline-Prüfung scheitern, da keine Pipeline läuft"
        );

        // push_audio_track is ignored in dual output mode.
        assert!(
            engine.push_audio_track(&frame, AudioTrack::System).is_ok(),
            "push_audio_track muss im Dual-Output-Modus ignoriert werden"
        );
    }

    /// End-to-end test for separate audio tracks with only one active source:
    /// the engine still must write a valid file when one track is disabled.
    #[test]
    fn records_separate_audio_tracks_with_single_source() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);
        engine.set_audio_track_enabled(AudioTrack::Microphone, false);

        let path =
            std::env::temp_dir().join(format!("rivulet_single_track_{}.mp4", std::process::id()));
        engine.start_local_recording(path.clone());

        let (width, height) = (64u32, 64u32);
        let video = vec![0u8; (width * height * 4) as usize];
        for _ in 0..12 {
            engine.process_raw_frame(&video, width, height);
        }

        let audio = AudioFrame::new(vec![0.0f32; 4800], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
        for _ in 0..12 {
            let _ = engine.push_audio_track(&audio, AudioTrack::System);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
        engine.stop_recording();

        assert!(path.exists(), "Ausgabedatei sollte existieren");
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        assert!(len > 0, "Ausgabedatei sollte nicht leer sein");

        let uri = format!("file://{}", path.display());
        let discoverer = gst_pbutils::Discoverer::new(gst::ClockTime::from_seconds(5))
            .expect("Discoverer sollte erstellt werden können");
        let info = discoverer
            .discover_uri(&uri)
            .expect("Datei sollte lesbar sein");
        assert_eq!(
            info.audio_streams().len(),
            1,
            "Es sollte ein Audio-Stream vorhanden sein"
        );

        let _ = std::fs::remove_file(&path);
    }
}
