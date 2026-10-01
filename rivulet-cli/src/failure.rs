//! Structured, machine-readable run failures (M7 W5, issue #191).
//!
//! Historically a failing run surfaced only as an `error: <anyhow chain>` line
//! on stderr. The spec's acceptance criterion — *"a failing run names the
//! failing stage in the machine-readable report"* — requires the failure to be
//! part of the stable JSON contract, so a consumer can tell *where* a run broke
//! without scraping prose.
//!
//! [`RunFailure`] is the library-side error type: it carries the [`Stage`], an
//! actionable message, and — when known — the redacted pipeline and the
//! resolved config. The binary renders it two ways and never mixes them:
//!
//! * [`RunFailure::to_json`] → one `failed` object on **stdout**, appended to
//!   the JSON status stream;
//! * [`RunFailure::describe`] → one human line on **stderr**.
//!
//! Secrets never reach this type by construction: the pipeline it carries is
//! whatever [`rivulet_core::RivuletEngine::pipeline_description`] returned,
//! which the core redacts before handing it out, and this module does no
//! un-redacting.

use crate::config::{RecordConfig, Stage, StatusEvent};
use crate::exit_code;

/// A failed `record`, `inspect`, or argument parse.
#[derive(Debug, Clone, PartialEq)]
pub struct RunFailure {
    stage: Stage,
    message: String,
    pipeline: Option<String>,
    /// Boxed so the error type stays small: `run`/`inspect` return it by value
    /// and clippy's `result_large_err` fires on an inline `RecordConfig`.
    config: Option<Box<RecordConfig>>,
}

impl RunFailure {
    /// Build a failure in `stage` with an actionable, secret-free `message`.
    pub fn new(stage: Stage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
            pipeline: None,
            config: None,
        }
    }

    /// Invalid command-line arguments.
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(Stage::Usage, message)
    }

    /// Invalid config file or values.
    pub fn config(message: impl Into<String>) -> Self {
        Self::new(Stage::Config, message)
    }

    /// The output location could not be prepared.
    pub fn output(message: impl Into<String>) -> Self {
        Self::new(Stage::Output, message)
    }

    /// The media engine failed while running.
    pub fn engine(message: impl Into<String>) -> Self {
        Self::new(Stage::Engine, message)
    }

    /// The engine failed while stopping, or produced no output.
    pub fn finalize(message: impl Into<String>) -> Self {
        Self::new(Stage::Finalize, message)
    }

    /// Attach the redacted pipeline the engine was running, if one was built.
    pub fn with_pipeline(mut self, pipeline: Option<String>) -> Self {
        self.pipeline = pipeline;
        self
    }

    /// Attach the resolved config the run was driven from.
    pub fn with_config(mut self, config: RecordConfig) -> Self {
        self.config = Some(Box::new(config));
        self
    }

    /// The stage the run failed in.
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The actionable failure message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The redacted pipeline, if one was built before the failure.
    pub fn pipeline(&self) -> Option<&str> {
        self.pipeline.as_deref()
    }

    /// The resolved config, if one was parsed before the failure.
    pub fn resolved_config(&self) -> Option<&RecordConfig> {
        self.config.as_deref()
    }

    /// The process exit code for this failure (spec § CLI surface reference):
    /// usage/config errors are `2`, runtime failures are `1`.
    pub fn exit_code(&self) -> i32 {
        match self.stage {
            Stage::Usage | Stage::Config => exit_code::USAGE,
            Stage::Output | Stage::Engine | Stage::Finalize => exit_code::RUNTIME,
        }
    }

    /// The failure as the stable `failed` status event.
    pub fn to_status_event(&self) -> StatusEvent {
        StatusEvent::Failed {
            stage: self.stage,
            exit_code: self.exit_code(),
            message: self.message.clone(),
            pipeline: self.pipeline.clone(),
            config: self.config.as_deref().cloned(),
        }
    }

    /// The machine-readable form: one JSON object on stdout.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.to_status_event()).expect("a status event serializes")
    }

    /// The human form: one diagnostics line on stderr.
    pub fn describe(&self) -> String {
        crate::config::describe(&self.to_status_event())
    }
}

impl std::fmt::Display for RunFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.stage, self.message)
    }
}

impl std::error::Error for RunFailure {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AudioConfig, OutputConfig, VideoConfig};
    use std::path::PathBuf;

    fn sample_config() -> RecordConfig {
        RecordConfig {
            output: OutputConfig {
                path: Some(PathBuf::from("out.mp4")),
                container: Some("mp4".to_string()),
            },
            video: VideoConfig::default(),
            audio: AudioConfig { enabled: false },
            duration_secs: None,
        }
    }

    #[test]
    fn exit_codes_follow_the_stage() {
        assert_eq!(RunFailure::usage("bad flag").exit_code(), exit_code::USAGE);
        assert_eq!(
            RunFailure::config("output.path: required").exit_code(),
            exit_code::USAGE
        );
        for failure in [
            RunFailure::output("cannot create dir"),
            RunFailure::engine("engine error: x"),
            RunFailure::finalize("no output file"),
        ] {
            assert_eq!(
                failure.exit_code(),
                exit_code::RUNTIME,
                "stage {} must be a runtime exit",
                failure.stage()
            );
        }
    }

    #[test]
    fn json_event_names_the_stage_and_exit_code() {
        let failure = RunFailure::engine("engine error: bus error")
            .with_pipeline(Some("videotestsrc ! mp4mux ! filesink".to_string()))
            .with_config(sample_config());
        let value: serde_json::Value = serde_json::from_str(&failure.to_json()).unwrap();

        assert_eq!(value["event"], "failed");
        assert_eq!(value["stage"], "engine");
        assert_eq!(value["exit_code"], 1);
        assert_eq!(value["message"], "engine error: bus error");
        assert_eq!(value["pipeline"], "videotestsrc ! mp4mux ! filesink");
        assert_eq!(value["config"]["output"]["container"], "mp4");
    }

    #[test]
    fn optional_fields_are_omitted_when_unknown() {
        // A usage error happens before a config exists, so the object must stay
        // small rather than carrying nulls a consumer would have to special-case.
        let json = RunFailure::usage("unknown flag --nope").to_json();
        assert!(json.contains(r#""event":"failed""#), "{json}");
        assert!(json.contains(r#""stage":"usage""#), "{json}");
        assert!(!json.contains("pipeline"), "{json}");
        assert!(!json.contains("config"), "{json}");
    }

    #[test]
    fn every_stage_serializes_as_its_snake_case_name() {
        for (stage, name) in [
            (Stage::Usage, "usage"),
            (Stage::Config, "config"),
            (Stage::Output, "output"),
            (Stage::Engine, "engine"),
            (Stage::Finalize, "finalize"),
        ] {
            let value: serde_json::Value =
                serde_json::from_str(&RunFailure::new(stage, "m").to_json()).unwrap();
            assert_eq!(value["stage"], name);
        }
    }

    #[test]
    fn human_line_names_the_stage() {
        let failure = RunFailure::config("output.path: required");
        let line = failure.describe();
        assert!(line.starts_with("error: "), "{line}");
        assert!(line.contains("config"), "stage must be named: {line}");
        assert!(line.contains("output.path"), "{line}");
        // Display is the message with its stage, for `{e}` formatting.
        assert_eq!(failure.to_string(), "config: output.path: required");
    }

    #[test]
    fn to_status_event_is_lossless() {
        let failure = RunFailure::finalize("recording produced no output file")
            .with_pipeline(Some("pipeline".to_string()))
            .with_config(sample_config());
        let event = failure.to_status_event();
        assert_eq!(crate::config::describe(&event), failure.describe());
        assert_eq!(failure.pipeline(), Some("pipeline"));
        assert_eq!(failure.resolved_config(), Some(&sample_config()));
        assert_eq!(failure.message(), "recording produced no output file");
    }
}
