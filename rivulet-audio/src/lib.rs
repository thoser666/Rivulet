//! Audio capture and mixing for Rivulet.
//!
//! Captures system audio (what you hear) and microphone input. By default the
//! sources are mixed into a single interleaved `f32` PCM stream (e.g. so a
//! streamer can hear their own voice alongside the game/desktop sound). With
//! [`AudioConfig::separate_tracks`] enabled the two sources are delivered as
//! separate streams for recording into distinct audio tracks.

pub mod capture;

pub use capture::{AudioCapture, AudioConfig, AudioFilters};
pub use rivulet_core::SkippedFilter;
