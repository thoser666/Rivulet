//! Audio capture and mixing for Rivulet.
//!
//! Captures system audio (what you hear) and microphone input and mixes them
//! into a single interleaved `f32` PCM stream, so that e.g. a streamer can
//! hear their own voice alongside the game/desktop sound.

pub mod capture;

pub use capture::{AudioCapture, AudioConfig};
