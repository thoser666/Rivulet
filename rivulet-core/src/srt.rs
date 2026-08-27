//! Configuration and validation contracts for SRT/RIST contribution outputs.
//!
//! Transport negotiation is intentionally kept separate from this contract. The
//! validated settings can be used by a future GStreamer srtsink/ristsink
//! adapter without exposing secrets in diagnostics.

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

    pub fn pipeline_sink_fragment(&self) -> String {
        match self.protocol {
            ContributionProtocol::Srt => format!("srtsink uri=\"{}\"", self.endpoint),
            ContributionProtocol::Rist => format!("ristsink uri=\"{}\"", self.endpoint),
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
                .starts_with("ristsink")
        );
    }
}
