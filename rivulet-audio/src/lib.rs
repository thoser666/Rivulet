//! Audio capture and mixing for Rivulet.
//!
//! Captures system audio (what you hear) and microphone input. By default the
//! sources are mixed into a single interleaved `f32` PCM stream (e.g. so a
//! streamer can hear their own voice alongside the game/desktop sound). With
//! [`AudioConfig::separate_tracks`] enabled the two sources are delivered as
//! separate streams for recording into distinct audio tracks.

#[cfg(target_os = "macos")]
pub mod app_audio_macos;
#[cfg(target_os = "linux")]
pub mod app_audio_pw;
pub mod capture;
pub(crate) mod messages;
#[cfg(target_os = "windows")]
pub mod process_loopback;

#[cfg(target_os = "macos")]
pub use app_audio_macos::{list_audio_processes, AppAudioCapture, AppAudioProcess};
#[cfg(target_os = "linux")]
pub use app_audio_pw::{list_audio_processes, AppAudioCapture, AppAudioProcess};
pub use capture::{AudioCapture, AudioConfig, AudioFilters};
#[cfg(target_os = "windows")]
pub use process_loopback::{list_audio_processes, AppAudioCapture, AppAudioProcess};
pub use rivulet_core::SkippedFilter;
