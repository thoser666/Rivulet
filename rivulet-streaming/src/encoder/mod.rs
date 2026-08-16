use crate::{EncodableFrame, RecordingSettings};
use anyhow::Result;

mod video;
pub use video::VideoEncoder;

// This is only a placeholder struct
pub struct Encoder {
    settings: RecordingSettings,
    start_time: Option<std::time::Instant>,
    frame_count: u64,
}

impl Encoder {
    pub fn new(settings: RecordingSettings) -> Result<Self> {
        if settings.width == 0 || settings.height == 0 {
            anyhow::bail!("Invalid dimensions: {}x{}", settings.width, settings.height);
        }

        Ok(Self {
            settings,
            start_time: None,
            frame_count: 0,
        })
    }

    pub fn encode_frame(&mut self, frame: &EncodableFrame) -> Result<()> {
        if self.start_time.is_none() {
            self.start_time = Some(frame.timestamp);
        }

        self.frame_count += 1;
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<()> {
        println!(
            "Encoded {} frames to {:?}",
            self.frame_count, self.settings.output_path
        );
        Ok(())
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EncodableFrame;
    use std::path::PathBuf;

    fn make_settings() -> RecordingSettings {
        RecordingSettings {
            output_path: PathBuf::from("out.mp4"),
            width: 1280,
            height: 720,
            fps: 30,
            bitrate: 2_000_000,
            codec: "h264".to_string(),
        }
    }

    fn make_frame() -> EncodableFrame {
        EncodableFrame {
            data: vec![0; 100],
            width: 1280,
            height: 720,
            stride: 1280 * 4,
            timestamp: std::time::Instant::now(),
        }
    }

    #[test]
    fn encoder_rejects_zero_dimensions() {
        let mut settings = make_settings();
        settings.width = 0;
        assert!(Encoder::new(settings).is_err());

        let mut settings = make_settings();
        settings.height = 0;
        assert!(Encoder::new(settings).is_err());
    }

    #[test]
    fn encoder_accepts_valid_settings() {
        let encoder = Encoder::new(make_settings()).unwrap();
        assert_eq!(encoder.frame_count(), 0);
    }

    #[test]
    fn encoder_counts_encoded_frames() {
        let mut encoder = Encoder::new(make_settings()).unwrap();
        encoder.encode_frame(&make_frame()).unwrap();
        encoder.encode_frame(&make_frame()).unwrap();
        encoder.encode_frame(&make_frame()).unwrap();
        assert_eq!(encoder.frame_count(), 3);
        encoder.finalize().unwrap();
    }
}
