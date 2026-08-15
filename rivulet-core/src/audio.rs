/// Identifies which input channel an [`AudioFrame`] belongs to.
///
/// Used when recording separate audio tracks so that the system audio and the
/// microphone are stored in distinct tracks of the output file instead of
/// being mixed together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTrack {
    /// The system/desktop audio ("what you hear").
    System,
    /// The microphone input.
    Microphone,
}

/// Interleaved PCM audio data produced by an audio capture source.
///
/// Samples are `f32` in the range `[-1.0, 1.0]` and interleaved by channel
/// (`[L, R, L, R, ...]`).
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub data: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioFrame {
    pub fn new(data: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        Self {
            data,
            sample_rate,
            channels,
        }
    }

    /// Number of samples per channel in this frame.
    pub fn frame_len(&self) -> usize {
        self.data.len() / self.channels.max(1) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_frame_with_given_data() {
        let data = vec![0.1, -0.2, 0.3, -0.4];
        let frame = AudioFrame::new(data.clone(), 48_000, 2);
        assert_eq!(frame.data, data);
        assert_eq!(frame.sample_rate, 48_000);
        assert_eq!(frame.channels, 2);
    }

    #[test]
    fn frame_len_computes_samples_per_channel() {
        let frame = AudioFrame::new(vec![0.0; 8], 48_000, 2);
        assert_eq!(frame.frame_len(), 4);
    }

    #[test]
    fn frame_len_never_divides_by_zero() {
        let frame = AudioFrame::new(vec![0.0; 4], 48_000, 0);
        assert_eq!(frame.frame_len(), 4);
    }

    #[test]
    fn empty_frame_has_zero_len() {
        let frame = AudioFrame::new(Vec::new(), 48_000, 2);
        assert_eq!(frame.frame_len(), 0);
    }

    #[test]
    fn audio_track_variants_are_distinct() {
        assert_ne!(AudioTrack::System, AudioTrack::Microphone);
        assert_eq!(AudioTrack::System, AudioTrack::System);
        assert_eq!(AudioTrack::Microphone, AudioTrack::Microphone);
    }
}
