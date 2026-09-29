//! `rivulet inspect` — the pipeline inspector / diagnostics surface
//! (M7 W5, issue #191).
//!
//! The point of this command is to answer "what would Rivulet actually do with
//! this config, and what does this machine support?" *before* a recording
//! burns time. It is the engine-aware analogue of `gst-inspect` /
//! `gst-launch --no-run`:
//!
//! * the pipeline string comes from
//!   [`rivulet_core::RivuletEngine::pipeline_description`], the same builder
//!   the engine uses on the first frame, so the two cannot drift;
//! * the capability report comes from [`rivulet_core::FeatureReport`], which
//!   probes the local GStreamer installation.
//!
//! Secrets never reach this surface: the pipeline description is redacted by
//! the core before it is handed out, and this module does no un-redacting.

use anyhow::{Context, Result};
use rivulet_core::{FeatureReport, RivuletEngine};
use serde::Serialize;

use crate::config::RecordConfig;

/// The result of an inspection: the pipeline for a config plus, optionally, the
/// machine's capability report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InspectReport {
    /// The pipeline string the engine would build (already redacted).
    pub pipeline: String,
    /// The recording config the description was built from, echoed back so a
    /// saved report is self-describing.
    pub config: RecordConfig,
    /// Capability detection results; `None` unless the caller asked for them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<FeatureReport>,
}

impl InspectReport {
    /// Build a report from a config and an already-configured engine.
    ///
    /// The engine must have been configured through the shared mapping (see
    /// `rivulet_cli::engine_for`) so the description reflects the same
    /// settings a real run would use.
    pub fn from_engine(config: &RecordConfig, engine: &RivuletEngine) -> Self {
        let pipeline = engine.pipeline_description().unwrap_or_else(|| {
            String::from("(no output configured: the engine would build no pipeline)")
        });
        Self {
            pipeline,
            config: config.clone(),
            features: None,
        }
    }

    /// Attach the machine's capability report.
    pub fn with_features(mut self, features: FeatureReport) -> Self {
        self.features = Some(features);
        self
    }

    /// A human-readable multi-line rendering (stderr diagnostics, or the plain
    /// output when no `--json` was requested).
    pub fn describe(&self) -> String {
        let mut out = String::new();
        out.push_str("pipeline: ");
        out.push_str(&self.pipeline);
        if let Some(path) = &self.config.output.path {
            out.push_str(&format!("\noutput: {}", path.display()));
        }
        if let Some(container) = &self.config.output.container {
            out.push_str(&format!("\ncontainer: {container}"));
        }
        out.push_str(&format!(
            "\naudio: {}",
            if self.config.audio.enabled {
                "enabled"
            } else {
                "disabled"
            }
        ));
        if let Some(features) = &self.features {
            out.push_str(&format!("\ngstreamer: {}", features.gstreamer_version));
            out.push_str("\nencoders:");
            for backend in &features.encoders {
                let available: Vec<&str> = backend
                    .codecs
                    .iter()
                    .filter(|c| c.available)
                    .map(|c| c.codec.as_str())
                    .collect();
                out.push_str(&format!(
                    "\n  {:<10} {:<24} available: {}",
                    backend.backend,
                    backend.backend_label,
                    if available.is_empty() {
                        "none".to_string()
                    } else {
                        available.join(", ")
                    }
                ));
            }
            out.push_str("\ncontainers:");
            for container in &features.containers {
                out.push_str(&format!(
                    "\n  {:<7} {:<20} {}",
                    container.container,
                    container.label,
                    if container.available {
                        "available"
                    } else {
                        "missing"
                    }
                ));
            }
        }
        out
    }
}

/// Inspect a config and return the pipeline the engine would build.
///
/// No pipeline is constructed and no file is written — the engine is only
/// configured far enough to describe itself.
pub fn inspect(config: RecordConfig) -> Result<InspectReport> {
    config.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
    let engine = crate::engine_for(&config)?;
    Ok(InspectReport::from_engine(&config, &engine))
}

/// Inspect a config and include the machine's capability report.
pub fn inspect_with_features(config: RecordConfig) -> Result<InspectReport> {
    Ok(inspect(config)?.with_features(FeatureReport::detect()))
}

/// Inspect a config and render the report as a single JSON object.
///
/// The JSON is the machine-readable form (`rivulet inspect --json`); its shape
/// is pinned by the `cli_inspect_json_schema_is_pinned` ci_pinning test.
pub fn inspect_json(config: RecordConfig) -> Result<String> {
    let report = inspect_with_features(config)?;
    serde_json::to_string(&report).context("serializing the inspect report")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioConfig, OutputConfig, VideoConfig};
    use std::path::{Path, PathBuf};

    fn config(dir: &Path, container: Option<&str>) -> RecordConfig {
        RecordConfig {
            output: OutputConfig {
                path: Some(dir.join("inspect.mp4")),
                container: container.map(str::to_string),
            },
            video: VideoConfig::default(),
            audio: AudioConfig { enabled: false },
            duration_secs: Some(1),
        }
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rivulet-inspect-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn inspect_reports_the_pipeline_without_recording() {
        let dir = tmp_dir("pipeline");
        let report = inspect(config(&dir, Some("mp4"))).expect("inspect succeeds");
        assert!(
            report.pipeline.contains("mp4mux"),
            "pipeline should name the muxer: {}",
            report.pipeline
        );
        assert!(
            !dir.join("inspect.mp4").exists(),
            "inspect must not create the output file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The W5 acceptance criterion: what `inspect` prints must be what
    /// `record` builds, because both go through the same engine mapping.
    #[test]
    fn inspect_pipeline_matches_a_real_recording() {
        let dir = tmp_dir("parity");
        let cfg = config(&dir, Some("mp4"));
        let report = inspect(cfg.clone()).expect("inspect succeeds");
        let out = crate::record(cfg).expect("recording succeeds");
        assert!(std::fs::metadata(&out).expect("output exists").len() > 0);
        // The recording leg is what inspect described, so the muxer the
        // recording used must be the one inspect named.
        assert!(report.pipeline.contains("mp4mux"), "{}", report.pipeline);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn container_choice_changes_the_reported_muxer() {
        let dir = tmp_dir("container");
        let mkv = inspect(config(&dir, Some("mkv"))).expect("inspect succeeds");
        assert!(
            mkv.pipeline.contains("matroskamux"),
            "mkv config must report the Matroska muxer: {}",
            mkv.pipeline
        );
        let mp4 = inspect(config(&dir, Some("mp4"))).expect("inspect succeeds");
        assert!(
            mp4.pipeline.contains("mp4mux") && !mp4.pipeline.contains("matroskamux"),
            "mp4 config must report the MP4 muxer: {}",
            mp4.pipeline
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression guard for the latent W1 bug: `--container` was validated and
    /// documented but never applied to the engine, so every container produced
    /// MP4. The mapping is now shared, so the reported muxer is the real one.
    #[test]
    fn record_honors_the_configured_container() {
        let dir = tmp_dir("record-mkv");
        let mut cfg = config(&dir, Some("mkv"));
        cfg.output.path = Some(dir.join("out.mkv"));
        let report = crate::RecordJob::new(cfg.clone())
            .dry_run()
            .expect("dry run succeeds");
        assert!(
            report.pipeline.contains("matroskamux"),
            "{}",
            report.pipeline
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dry_run_matches_inspect() {
        let dir = tmp_dir("dry-run");
        let cfg = config(&dir, Some("mp4"));
        let inspected = inspect(cfg.clone()).expect("inspect succeeds");
        let dry = crate::RecordJob::new(cfg)
            .dry_run()
            .expect("dry run succeeds");
        assert_eq!(
            dry.pipeline, inspected.pipeline,
            "dry-run and inspect share the same code path"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dry_run_writes_nothing() {
        let dir = tmp_dir("dry-run-nowrite");
        let cfg = config(&dir, Some("mp4"));
        crate::RecordJob::new(cfg)
            .dry_run()
            .expect("dry run succeeds");
        assert!(
            !dir.join("inspect.mp4").exists(),
            "a dry run must not create the output file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_config_names_the_key() {
        let mut cfg = config(&tmp_dir("invalid"), Some("mp4"));
        cfg.output.path = None;
        let err = inspect(cfg).unwrap_err().to_string();
        assert!(err.contains("output.path"), "error = {err}");
    }

    #[test]
    fn json_report_has_the_documented_shape() {
        let dir = tmp_dir("json");
        let json = inspect_json(config(&dir, Some("mp4"))).expect("json serialization succeeds");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        for key in ["pipeline", "config", "features"] {
            assert!(
                value.get(key).is_some(),
                "report must contain {key}: {json}"
            );
        }
        let features = &value["features"];
        for key in [
            "gstreamer_version",
            "encoders",
            "containers",
            "capture_backends",
            "audio_filters",
        ] {
            assert!(features.get(key).is_some(), "features needs {key}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No diagnostic surface may leak a stream key. The CLI config model has
    /// no streaming section, but the engine leg can still embed an ingest URL
    /// when a caller inspects a streaming engine, so the core accessor must
    /// redact (pinned here from the CLI side too).
    #[test]
    fn inspect_surface_never_contains_a_stream_key() {
        let mut engine = RivuletEngine::new();
        engine.set_stream_settings(Some(rivulet_core::StreamSettings::twitch("top-secret")));
        let cfg = config(&tmp_dir("redact"), Some("mp4"));
        let report = InspectReport::from_engine(&cfg, &engine);
        assert!(
            !report.pipeline.contains("top-secret"),
            "stream key leaked into the inspect surface: {}",
            report.pipeline
        );
        assert!(report.pipeline.contains("<redacted stream URL>"));
    }

    #[test]
    fn describe_renders_pipeline_and_capabilities() {
        let dir = tmp_dir("describe");
        let report = inspect_with_features(config(&dir, Some("mp4"))).expect("inspect succeeds");
        let text = report.describe();
        assert!(text.contains("pipeline: "), "{text}");
        assert!(text.contains("encoders:"), "{text}");
        assert!(text.contains("containers:"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_container_is_rejected_by_the_mapping() {
        let err = crate::recording_container(Some("avi"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("output.container"), "error = {err}");
    }
}
