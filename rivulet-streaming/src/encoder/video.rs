use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub struct VideoEncoder {
    output_path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    #[allow(dead_code)] // Erlaubt, dass das Feld unbenutzt ist
    bitrate: u64,
    frame_count: u64,
    ffmpeg_process: Option<Child>,
}

impl VideoEncoder {
    pub fn new(
        output_path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u64,
    ) -> Result<Self> {
        if width == 0 || height == 0 {
            anyhow::bail!("Invalid dimensions: {}x{}", width, height);
        }

        if fps == 0 {
            anyhow::bail!("Invalid FPS: {}", fps);
        }

        println!("Creating VideoEncoder:");
        println!("  Output: {:?}", output_path);
        println!("  Resolution: {}x{}", width, height);
        println!("  FPS: {}", fps);
        println!("  Bitrate: {} bps", bitrate);

        // Starte FFmpeg Prozess
        let ffmpeg = Command::new("ffmpeg")
            .args([
                // KORREKTUR: `&` entfernt
                "-y", // Überschreibe Output-Datei
                "-f",
                "rawvideo",
                "-pixel_format",
                "bgra",
                "-video_size",
                &format!("{}x{}", width, height),
                "-framerate",
                &fps.to_string(),
                "-i",
                "pipe:0",
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-pix_fmt",
                "yuv420p",
                "-b:v",
                &bitrate.to_string(),
                output_path.to_str().context("Invalid output path")?,
            ])
            .stdin(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Failed to start ffmpeg. Is it installed?")?;

        Ok(Self {
            output_path: output_path.to_path_buf(),
            width,
            height,
            fps,
            bitrate,
            frame_count: 0,
            ffmpeg_process: Some(ffmpeg),
        })
    }

    pub fn encode_frame(
        &mut self,
        frame_data: &[u8],
        width: u32,
        height: u32,
        stride: u32,
    ) -> Result<()> {
        if width != self.width || height != self.height {
            anyhow::bail!(
                "Frame dimensions {}x{} don't match encoder {}x{}",
                width,
                height,
                self.width,
                self.height
            );
        }

        let expected_size = (stride * height) as usize;
        if frame_data.len() < expected_size {
            anyhow::bail!(
                "Frame data too small: {} bytes, expected at least {}",
                frame_data.len(),
                expected_size
            );
        }

        if let Some(process) = &mut self.ffmpeg_process {
            if let Some(stdin) = process.stdin.as_mut() {
                if stride == width * 4 {
                    stdin
                        .write_all(frame_data)
                        .context("Failed to write frame to ffmpeg")?;
                } else {
                    let row_size = (width * 4) as usize;
                    for y in 0..height as usize {
                        let start = y * stride as usize;
                        let end = start + row_size;
                        stdin
                            .write_all(&frame_data[start..end])
                            .context("Failed to write frame row to ffmpeg")?;
                    }
                }
                self.frame_count += 1;
            } else {
                anyhow::bail!("FFmpeg stdin not available");
            }
        } else {
            anyhow::bail!("FFmpeg process not running");
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        println!("Finalizing video...");
        println!("  Total frames: {}", self.frame_count);
        println!(
            "  Duration: {:.2}s",
            self.frame_count as f64 / self.fps as f64
        );

        if let Some(mut process) = self.ffmpeg_process.take() {
            drop(process.stdin.take());
            let status = process.wait().context("Failed to wait for ffmpeg")?;
            if !status.success() {
                anyhow::bail!("FFmpeg exited with error: {}", status);
            }
        }
        println!("✅ Video saved to: {:?}", self.output_path);
        Ok(())
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
}

impl Drop for VideoEncoder {
    fn drop(&mut self) {
        if let Some(mut process) = self.ffmpeg_process.take() {
            let _ = process.stdin.take();
            let _ = process.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Diese Validierungen laufen ab, bevor FFmpeg gestartet wird,
    // daher benötigen die Tests kein ffmpeg-Binary.
    #[test]
    fn rejects_zero_width() {
        let result = VideoEncoder::new(Path::new("out.mp4"), 0, 720, 30, 1_000_000);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_zero_height() {
        let result = VideoEncoder::new(Path::new("out.mp4"), 1280, 0, 30, 1_000_000);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_zero_fps() {
        let result = VideoEncoder::new(Path::new("out.mp4"), 1280, 720, 0, 1_000_000);
        assert!(result.is_err());
    }
}
