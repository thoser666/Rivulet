use std::path::PathBuf;

pub mod encoder;
pub mod recorder;

pub use encoder::*;
pub use recorder::*;

/// Frame data for encoding
#[derive(Debug, Clone)]
pub struct EncodableFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub timestamp: std::time::Instant,
}

/// Recording settings
#[derive(Debug, Clone)]
pub struct RecordingSettings {
    pub output_path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u64, // bits per second
    pub codec: String,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            output_path: PathBuf::from("recording.mp4"),
            width: 1920,
            height: 1080,
            fps: 30,
            bitrate: 5_000_000, // 5 Mbps
            codec: "h264".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_recording_settings() {
        let settings = RecordingSettings::default();
        assert_eq!(settings.output_path, PathBuf::from("recording.mp4"));
        assert_eq!(settings.width, 1920);
        assert_eq!(settings.height, 1080);
        assert_eq!(settings.fps, 30);
        assert_eq!(settings.bitrate, 5_000_000);
        assert_eq!(settings.codec, "h264");
    }
}
