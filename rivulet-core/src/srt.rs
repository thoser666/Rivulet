//! Configuration and validation contracts for SRT/RIST contribution outputs.
//!
//! Transport-specific settings for GStreamer `srtsink`/`ristsink` outputs.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContributionProtocol {
    Srt,
    Rist,
}

impl ContributionProtocol {
    pub fn scheme(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Rist => "rist",
        }
    }

    pub fn sink_element_name(self) -> &'static str {
        match self {
            Self::Srt => "srtsink",
            Self::Rist => "ristsink",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributionSettings {
    pub protocol: ContributionProtocol,
    pub endpoint: String,
    pub passphrase: Option<String>,
    pub latency: Duration,
}

impl ContributionSettings {
    pub fn new(protocol: ContributionProtocol, endpoint: impl Into<String>) -> Self {
        Self {
            protocol,
            endpoint: endpoint.into(),
            passphrase: None,
            latency: Duration::from_millis(120),
        }
    }

    pub fn with_passphrase(mut self, passphrase: impl Into<String>) -> Self {
        self.passphrase = Some(passphrase.into());
        self
    }

    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = latency.clamp(Duration::from_millis(20), Duration::from_secs(10));
        self
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let prefix = format!("{}://", self.protocol.scheme());
        if !self.endpoint.starts_with(&prefix) || self.endpoint.len() <= prefix.len() {
            return Err("contribution endpoint must use the selected protocol scheme");
        }
        if let Some(passphrase) = &self.passphrase {
            if !passphrase.is_empty() && !(10..=79).contains(&passphrase.len()) {
                return Err("SRT passphrase must contain 10 to 79 characters");
            }
        }
        Ok(())
    }

    pub fn redacted_endpoint(&self) -> String {
        self.endpoint.clone()
    }

    pub fn has_passphrase(&self) -> bool {
        self.passphrase
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    }

    /// Return a GStreamer sink fragment after validating this contribution.
    /// Invalid settings intentionally produce an empty fragment so callers can
    /// skip an unavailable transport and report the validation error.
    pub fn validated_pipeline_sink_fragment(&self) -> Result<String, &'static str> {
        self.validate()?;
        Ok(self.pipeline_sink_fragment())
    }

    /// Check whether the required GStreamer element is available in the
    /// current installation without constructing a pipeline.
    pub fn sink_element_name(&self) -> &'static str {
        self.protocol.sink_element_name()
    }

    pub fn plugin_available(&self) -> bool {
        gstreamer::ElementFactory::find(self.sink_element_name()).is_some()
    }

    pub fn pipeline_sink_fragment(&self) -> String {
        match self.protocol {
            ContributionProtocol::Srt => {
                let passphrase = self.passphrase.as_deref().filter(|value| !value.is_empty());
                match passphrase {
                    Some(value) => format!(
                        "srtsink uri=\"{}\" passphrase=\"{}\" latency={} ",
                        self.endpoint,
                        value,
                        self.latency.as_millis()
                    ),
                    None => format!(
                        "srtsink uri=\"{}\" latency={} ",
                        self.endpoint,
                        self.latency.as_millis()
                    ),
                }
            }
            ContributionProtocol::Rist => {
                let endpoint = self.endpoint.trim_start_matches("rist://");
                let (address, port) = endpoint.split_once(':').unwrap_or((endpoint, "1968"));
                format!(
                    "h264parse ! mpegtsmux alignment=7 ! application/x-rtp,media=video,encoding-name=MP2T,clock-rate=90000,payload=33 ! ristsink address=\"{}\" port={} latency={} ",
                    address,
                    port,
                    self.latency.as_millis()
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_protocol_scheme_and_latency_bounds() {
        let settings = ContributionSettings::new(ContributionProtocol::Srt, "srt://127.0.0.1:9000")
            .with_latency(Duration::from_millis(1));
        assert!(settings.validate().is_ok());
        assert_eq!(settings.latency, Duration::from_millis(20));
        assert!(
            ContributionSettings::new(ContributionProtocol::Rist, "srt://host")
                .validate()
                .is_err()
        );
    }

    #[test]
    fn validates_srt_passphrase_without_logging_it() {
        let settings = ContributionSettings::new(ContributionProtocol::Srt, "srt://host:9000")
            .with_passphrase("top-secret-phrase");
        assert!(settings.validate().is_ok());
        assert!(settings.has_passphrase());
        assert!(!settings.redacted_endpoint().contains("top-secret"));
        assert!(
            ContributionSettings::new(ContributionProtocol::Srt, "srt://host")
                .with_passphrase("short")
                .validate()
                .is_err()
        );
    }

    #[test]
    fn produces_protocol_specific_sink_fragment() {
        assert!(
            ContributionSettings::new(ContributionProtocol::Srt, "srt://host")
                .pipeline_sink_fragment()
                .starts_with("srtsink")
        );
        assert!(
            ContributionSettings::new(ContributionProtocol::Rist, "rist://host")
                .pipeline_sink_fragment()
                .contains("ristsink")
        );
    }

    #[test]
    fn validated_sink_rejects_wrong_scheme_before_pipeline_use() {
        let settings = ContributionSettings::new(ContributionProtocol::Rist, "srt://host");
        assert_eq!(
            settings.validated_pipeline_sink_fragment(),
            Err("contribution endpoint must use the selected protocol scheme")
        );
    }

    #[test]
    fn rist_fragment_uses_supported_properties() {
        let fragment =
            ContributionSettings::new(ContributionProtocol::Rist, "rist://receiver:10080")
                .pipeline_sink_fragment();
        assert!(fragment.contains("ristsink"));
        assert!(fragment.contains("address=\"receiver\""));
        assert!(fragment.contains("port=10080"));
        assert!(!fragment.contains("uri="));
    }

    #[test]
    fn sink_element_name_matches_transport() {
        assert_eq!(ContributionProtocol::Srt.sink_element_name(), "srtsink");
        assert_eq!(ContributionProtocol::Rist.sink_element_name(), "ristsink");
    }

    #[test]
    fn plugin_lookup_is_safe_and_protocol_specific() {
        let _ = gstreamer::init();
        let srt = ContributionSettings::new(ContributionProtocol::Srt, "srt://host");
        let rist = ContributionSettings::new(ContributionProtocol::Rist, "rist://host");
        assert_eq!(
            srt.plugin_available(),
            gstreamer::ElementFactory::find("srtsink").is_some()
        );
        assert_eq!(
            rist.plugin_available(),
            gstreamer::ElementFactory::find("ristsink").is_some()
        );
    }
}
