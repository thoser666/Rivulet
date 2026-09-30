//! Test/loopback frame sources for headless runs (spec: Linux CI has no
//! capture hardware, so the CLI sources video from a generated pattern and
//! optional audio from a silence generator).
//!
//! The video generator itself lives in `rivulet_core::source::TestVideoSource`
//! so the deterministic tests (M7 W2b, issue #188) can generate frames with
//! the same code the recording path pushes; it is re-exported here, so the
//! CLI's public surface is unchanged.

use rivulet_core::AudioFrame;

pub use rivulet_core::source::TestVideoSource;

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
        // The generator moved to rivulet-core for W2b's golden-frame tests;
        // this keeps the CLI's re-export honest as the path a real run takes.
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
        assert_eq!(src.fps(), 30, "the cadence is retained after the move");
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
