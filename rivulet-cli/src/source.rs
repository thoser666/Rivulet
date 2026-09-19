//! Test/loopback frame sources for headless runs (spec: Linux CI has no
//! capture hardware, so the CLI sources video from a generated pattern and
//! optional audio from a silence generator).

use rivulet_core::AudioFrame;

/// Generates deterministic RGBA video frames at a fixed cadence.
///
/// The engine's appsrc consumes raw RGBA (`gst_video::VideoFormat::Rgba`,
/// see `initialize_and_start_pipeline`); frame *content* is a simple
/// time-varying gradient so encoded frames actually differ between frames.
pub struct TestVideoSource {
    width: u32,
    height: u32,
    #[allow(dead_code)]
    fps: u32,
    frame_index: u64,
}

impl TestVideoSource {
    pub fn new(width: u32, height: u32, fps: u32) -> Self {
        Self {
            width,
            height,
            fps,
            frame_index: 0,
        }
    }

    /// Produce the next RGBA frame (width * height * 4 bytes).
    pub fn next_frame(&mut self) -> Vec<u8> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut data = vec![0u8; w * h * 4];
        // Cheap deterministic animation: a slowly moving vertical band plus
        // a static horizontal gradient. Distinct per-frame content proves
        // the encoder received real picture data.
        let band = (self.frame_index % self.width.max(1) as u64) as usize;
        for y in 0..h {
            let row = &mut data[y * w * 4..(y + 1) * w * 4];
            for x in 0..w {
                let px = &mut row[x * 4..x * 4 + 4];
                px[0] = (x * 255 / w.max(1)) as u8; // R: horizontal ramp
                px[1] = if x >= band.saturating_sub(8) && x <= band + 8 {
                    255
                } else {
                    40
                }; // G: moving band
                px[2] = (y * 255 / h.max(1)) as u8; // B: vertical ramp
                px[3] = 255; // A
            }
        }
        self.frame_index += 1;
        data
    }
}

/// Generates silent stereo f32 PCM frames in the engine's input format
/// (48 kHz, 2 channels, interleaved).
#[derive(Default)]
pub struct SilenceSource;

impl SilenceSource {
    pub fn new() -> Self {
        Self
    }

    /// Produce the next audio frame with `samples_per_channel` samples per
    /// channel (interleaved stereo, so the buffer holds twice as many f32
    /// values).
    pub fn next_frame(&mut self, samples_per_channel: usize) -> AudioFrame {
        AudioFrame::new(
            vec![0.0f32; samples_per_channel * usize::from(rivulet_core::AUDIO_CHANNELS)],
            rivulet_core::AUDIO_SAMPLE_RATE,
            rivulet_core::AUDIO_CHANNELS,
        )
    }
}

/// Convenience: a single silent stereo frame.
pub fn silence_frame(samples_per_channel: usize) -> AudioFrame {
    SilenceSource::new().next_frame(samples_per_channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_frames_are_rgba_sized_and_deterministic_then_distinct() {
        let mut src = TestVideoSource::new(64, 48, 30);
        let f0 = src.next_frame();
        assert_eq!(f0.len(), 64 * 48 * 4);
        let f0_again = TestVideoSource::new(64, 48, 30).next_frame();
        assert_eq!(
            f0, f0_again,
            "same config → same first frame (deterministic)"
        );
        let f1 = src.next_frame();
        assert_ne!(f0, f1, "frames must differ over time (animated content)");
        assert_eq!(f0[3], 255, "alpha channel is opaque");
    }

    #[test]
    fn silence_frames_match_engine_audio_caps() {
        let frame = silence_frame(480);
        assert_eq!(frame.sample_rate, rivulet_core::AUDIO_SAMPLE_RATE);
        assert_eq!(frame.channels, rivulet_core::AUDIO_CHANNELS);
        assert_eq!(
            frame.data.len(),
            480 * usize::from(rivulet_core::AUDIO_CHANNELS)
        );
        assert!(frame.data.iter().all(|&s| s == 0.0));
    }
}
