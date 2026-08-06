// In rivulet-core/src/lib.rs

use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use once_cell::sync::Lazy;
use std::path::PathBuf;
// Wichtig: Der Prelude wird immer noch für Methoden wie .set_state(), .by_name() etc. benötigt.
use gst::prelude::*;

pub mod audio;
pub use audio::AudioFrame;

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
    audio_enabled: bool,
    is_recording: bool,
    output_path: Option<PathBuf>,
}

impl Default for RivuletEngine {
    fn default() -> Self {
        Lazy::force(&GSTREAMER_INIT);
        Self {
            pipeline: None,
            appsrc: None,
            audio_appsrc: None,
            audio_enabled: false,
            is_recording: false,
            output_path: None,
        }
    }
}

impl RivuletEngine {
    pub fn new() -> Self {
        println!("[Engine] GStreamer bereit.");
        Self::default()
    }

    fn initialize_and_start_pipeline(&mut self, width: u32, height: u32) {
        let Some(path) = self.output_path.as_ref() else {
            return;
        };
        let location = path.to_str().expect("Dateipfad ist ungültig.");
        println!("[Engine] Initialisiere Aufnahme-Pipeline für: {}", location);

        let mut pipeline_str =
            "appsrc name=rivulet_src ! videoconvert ! x264enc tune=zerolatency ! queue ! mux. "
                .to_string();
        if self.audio_enabled {
            pipeline_str.push_str(
                "appsrc name=audio_src format=time is-live=true do-timestamp=true \
                 ! audioconvert ! audioresample ! avenc_aac ! queue ! mux. ",
            );
        }
        pipeline_str.push_str(&format!(
            "mp4mux name=mux ! filesink location=\"{}\"",
            location
        ));

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

        if pipeline.set_state(gst::State::Playing).is_err() {
            eprintln!("[Engine] Pipeline konnte nicht gestartet werden.");
            return;
        }

        self.pipeline = Some(pipeline);
        self.appsrc = Some(appsrc);
        println!("[Engine] Aufnahme-Pipeline läuft.");
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

    /// Push a mixed PCM frame into the recording pipeline.
    pub fn push_audio_frame(&mut self, frame: &AudioFrame) -> anyhow::Result<()> {
        if !self.audio_enabled {
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
