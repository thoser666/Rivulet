//! Human-readable warning and error messages for the audio capture pipeline.
//!
//! Each message is a pure function of its inputs so the exact text can be
//! unit-tested without constructing a GStreamer pipeline or a capture
//! session. Call sites pass the result to `anyhow` or `tracing`.
//!
//! Some messages are only reachable from the Linux pipeline, others only from
//! the non-Linux stub, so a message may be unused on one platform by design.
#![allow(dead_code)]

use std::fmt::Display;

/// Error when neither system audio nor microphone capture is enabled.
pub(crate) fn no_audio_input_sources() -> &'static str {
    "No audio input sources enabled"
}

/// Error when the GStreamer pipeline description could not be built.
pub(crate) fn audio_pipeline_build_failed(error: &impl Display) -> String {
    format!("Failed to build audio pipeline: {error}")
}

/// Error when `start()` is called while separate tracks are configured.
pub(crate) fn separate_tracks_enabled_misuse() -> &'static str {
    "Separate audio tracks are enabled; use start_separated()"
}

/// Error when `start_separated()` is called while mixed tracks are configured.
pub(crate) fn separate_tracks_disabled_misuse() -> &'static str {
    "Separate audio tracks are disabled; use start()"
}

/// Error when a required appsink element is missing from the parsed pipeline.
pub(crate) fn missing_appsink(name: &str) -> String {
    format!("Missing appsink '{name}' in audio pipeline")
}

/// Error when a pipeline element cannot be downcast to an appsink.
pub(crate) fn not_an_appsink(name: &str) -> String {
    format!("Element '{name}' is not an appsink")
}

/// Error returned by the non-Linux/non-macOS stub, where capture is not
/// implemented.
pub(crate) fn capture_linux_only() -> &'static str {
    "Audio capture is currently only supported on Linux"
}

/// macOS: no loopback driver (BlackHole/Soundflower/VB-Cable) installed, so
/// system audio cannot be captured. Shown as a hint instead of failing the
/// recording — the microphone keeps working.
pub(crate) fn macos_system_audio_unavailable() -> &'static str {
    "System audio unavailable: no loopback device found (install BlackHole, Soundflower or VB-Cable). Microphone capture still works."
}

/// macOS: cpal could not resolve the default input device.
pub(crate) fn macos_no_input_device() -> &'static str {
    "No microphone input device found"
}

/// macOS: cpal could not read the default input format.
pub(crate) fn macos_no_input_format() -> &'static str {
    "Could not read the input device format"
}

/// macOS: the input device uses a sample format cpal reports but Rivulet
/// does not convert (only f32/i16/u16 are supported).
pub(crate) fn macos_unsupported_sample_format(format: &str) -> String {
    format!("Unsupported audio sample format: {format}")
}

/// macOS: opening the cpal input stream failed.
pub(crate) fn macos_stream_failed(error: &str) -> String {
    format!("Failed to open the audio input stream: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_audio_input_sources_message_is_exact() {
        assert_eq!(no_audio_input_sources(), "No audio input sources enabled");
    }

    #[test]
    fn audio_pipeline_build_failed_includes_the_cause() {
        assert_eq!(
            audio_pipeline_build_failed(&String::from("boom")),
            "Failed to build audio pipeline: boom"
        );
    }

    #[test]
    fn separate_track_misuse_messages_are_exact() {
        assert_eq!(
            separate_tracks_enabled_misuse(),
            "Separate audio tracks are enabled; use start_separated()"
        );
        assert_eq!(
            separate_tracks_disabled_misuse(),
            "Separate audio tracks are disabled; use start()"
        );
    }

    #[test]
    fn appsink_error_messages_are_exact() {
        assert_eq!(
            missing_appsink("sys_sink"),
            "Missing appsink 'sys_sink' in audio pipeline"
        );
        assert_eq!(
            not_an_appsink("sys_vol"),
            "Element 'sys_vol' is not an appsink"
        );
    }

    #[test]
    fn linux_only_message_is_exact() {
        assert_eq!(
            capture_linux_only(),
            "Audio capture is currently only supported on Linux"
        );
    }

    #[test]
    fn macos_system_audio_unavailable_mentions_the_loopback_drivers() {
        let msg = macos_system_audio_unavailable();
        assert!(msg.contains("BlackHole"));
        assert!(msg.contains("loopback"));
        assert!(msg.contains("Microphone"));
    }

    #[test]
    fn macos_error_messages_include_the_cause() {
        assert_eq!(macos_no_input_device(), "No microphone input device found");
        assert_eq!(
            macos_no_input_format(),
            "Could not read the input device format"
        );
        assert_eq!(
            macos_unsupported_sample_format("I64"),
            "Unsupported audio sample format: I64"
        );
        assert!(macos_stream_failed("boom").contains("boom"));
    }
}
