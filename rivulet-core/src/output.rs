use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// Output settings for recording/streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSettings {
    width: u32,
    height: u32,
    fps: u32,
    bitrate: u32,
    encoder: String,
}

impl OutputSettings {
    /// Width of the encoded video, in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }
    /// Height of the encoded video, in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Frames per second the output is configured for.
    pub fn fps(&self) -> u32 {
        self.fps
    }
    /// Target bitrate handed to the encoder. [`OutputSettings::default`] uses
    /// `5000`, which the built-in H.264 pipeline reads as kbit/s.
    pub fn bitrate(&self) -> u32 {
        self.bitrate
    }
    /// Encoder identifier as a string; the default is `"h264"`.
    pub fn encoder(&self) -> &str {
        &self.encoder
    }

    /// Build settings from explicit values instead of taking the default.
    pub fn new(
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        encoder: impl Into<String>,
    ) -> Self {
        Self {
            width,
            height,
            fps,
            bitrate,
            encoder: encoder.into(),
        }
    }
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30,
            bitrate: 5000,
            encoder: "h264".to_string(),
        }
    }
}

/// Trait for all output types (recording, streaming, etc.)
#[async_trait]
pub trait Output: Debug + Send + Sync {
    /// Take the output live. Called once before frames are pushed; an error
    /// means the output never started and the caller is expected to abort the
    /// session instead of pushing into it.
    async fn start(&self) -> anyhow::Result<()>;
    /// Stop the output and release what it holds. An error reports a failed
    /// shutdown (a file that could not be finalised, a connection that would
    /// not close), not a refused call.
    async fn stop(&self) -> anyhow::Result<()>;
    /// Whether the output is currently running, i.e. started and not yet
    /// stopped.
    fn is_active(&self) -> bool;
    /// The settings this output was created with.
    fn get_settings(&self) -> &OutputSettings;
}

/// File recording output
#[derive(Debug)]
pub struct FileOutput {
    file_path: String,
    settings: OutputSettings,
    active: std::sync::atomic::AtomicBool,
}

impl FileOutput {
    /// Create an inactive output that will write to `file_path` with
    /// `settings`. Nothing touches the filesystem until [`Output::start`].
    pub fn new(file_path: impl Into<String>, settings: OutputSettings) -> Self {
        Self {
            file_path: file_path.into(),
            settings,
            active: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The destination path, exactly as handed to [`FileOutput::new`].
    pub fn file_path(&self) -> &str {
        &self.file_path
    }

    /// The settings this output was created with.
    pub fn settings(&self) -> &OutputSettings {
        &self.settings
    }
}

#[async_trait]
impl Output for FileOutput {
    async fn start(&self) -> anyhow::Result<()> {
        tracing::info!("Starting file recording to: {}", self.file_path);
        self.active
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        tracing::info!("Stopping file recording");
        self.active
            .store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn get_settings(&self) -> &OutputSettings {
        &self.settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_settings() {
        let settings = OutputSettings::default();
        assert_eq!(settings.width, 1920);
        assert_eq!(settings.height, 1080);
        assert_eq!(settings.fps, 30);
        assert_eq!(settings.bitrate, 5000);
        assert_eq!(settings.encoder, "h264");
    }

    #[tokio::test]
    async fn file_output_start_stop_lifecycle() {
        let output = FileOutput::new("/tmp/example.mp4", OutputSettings::default());
        assert!(!output.is_active());

        output.start().await.unwrap();
        assert!(output.is_active());

        output.stop().await.unwrap();
        assert!(!output.is_active());
        assert_eq!(output.get_settings().width, 1920);
    }
}
