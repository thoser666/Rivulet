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
