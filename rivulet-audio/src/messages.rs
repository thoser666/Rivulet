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

/// Error returned by the non-Linux stub, where capture is not implemented.
pub(crate) fn capture_linux_only() -> &'static str {
    "Audio capture is currently only supported on Linux"
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
}
