//! Configuration and validation contracts for SRT/RIST contribution outputs.
//!
//! Transport-specific settings for GStreamer `srtsink`/`ristsink` outputs.

use std::time::Duration;

/// Low-latency contribution transport an output can publish over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContributionProtocol {
    /// Secure Reliable Transport: `srtsink`, `srt://` endpoints, optional
    /// passphrase.
    Srt,
    /// Reliable Internet Stream Transport: `ristsink` behind an MPEG-TS
    /// muxer, `rist://` endpoints.
    Rist,
}

impl ContributionProtocol {
    /// URI scheme the endpoint must start with (`srt` or `rist`) — what
    /// [`ContributionSettings::validate`] compares the endpoint against.
    pub fn scheme(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Rist => "rist",
        }
    }

    /// GStreamer element implementing this transport (`srtsink` or
    /// `ristsink`), which is also the name `plugin_available` looks up.
    pub fn sink_element_name(self) -> &'static str {
        match self {
            Self::Srt => "srtsink",
            Self::Rist => "ristsink",
        }
    }
}

/// Everything a contribution output needs: transport, destination, optional
/// passphrase and latency budget.
///
/// Build it with [`ContributionSettings::new`], refine it with the `with_*`
/// builders and hand it over through
/// [`ContributionSettings::validated_pipeline_sink_fragment`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributionSettings {
    /// Transport the sink fragment is built for; it also decides which URI
    /// scheme the endpoint has to use.
    pub protocol: ContributionProtocol,
    /// Destination URI including the scheme, e.g. `srt://host:9000`. An empty
    /// or scheme-less value is rejected by [`ContributionSettings::validate`].
    pub endpoint: String,
    /// Optional SRT passphrase. `None` (and, for compatibility, an empty
    /// string) means the connection is not encrypted; SRT itself requires
    /// 10 to 79 characters, which `validate` enforces.
    pub passphrase: Option<String>,
    /// Receive-buffer latency, emitted to the sink as `latency=<ms>`.
    /// [`ContributionSettings::with_latency`] keeps it inside 20 ms – 10 s.
    pub latency: Duration,
}

impl ContributionSettings {
    /// Settings for `protocol` and `endpoint`, with no passphrase and the
    /// default 120 ms latency (which still needs `validate` to pass).
    pub fn new(protocol: ContributionProtocol, endpoint: impl Into<String>) -> Self {
        Self {
            protocol,
            endpoint: endpoint.into(),
            passphrase: None,
            latency: Duration::from_millis(120),
        }
    }

    /// Sets the SRT passphrase. Not checked here — a length outside the SRT
    /// range is reported by [`Self::validate`], so defaults can be applied
    /// before validation runs.
    pub fn with_passphrase(mut self, passphrase: impl Into<String>) -> Self {
        self.passphrase = Some(passphrase.into());
        self
    }

    /// Sets the latency budget, clamped into the supported 20 ms – 10 s
    /// window: a smaller value is raised to the floor instead of rejected,
    /// because the floor is a protocol limit, not a typo.
    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = latency.clamp(Duration::from_millis(20), Duration::from_secs(10));
        self
    }

    /// Checks the settings against the transport before a pipeline is built.
    ///
    /// Rejects an endpoint without the selected protocol's scheme (or with a
    /// bare scheme and no destination) and a non-empty passphrase outside the
    /// 10–79 character SRT range. The message is static and names no secret,
    /// so it is safe to log.
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

    /// The endpoint as configured — it can be logged, since the passphrase is
    /// never part of it.
    ///
    /// Note this is not a general scrubber: a `streamid=` parameter inside the
    /// URI is returned verbatim, so an endpoint carrying an embedded secret
    /// still needs masking upstream.
    pub fn redacted_endpoint(&self) -> String {
        self.endpoint.clone()
    }

    /// Whether a usable (non-empty) passphrase is configured — the check the
    /// UI uses to show encryption state without reading the secret.
    pub fn has_passphrase(&self) -> bool {
        self.passphrase
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    }

    /// Return a GStreamer sink fragment after validating this contribution.
    /// Invalid settings return the error instead of a fragment, so callers can
    /// skip an unavailable transport and report why.
    pub fn validated_pipeline_sink_fragment(&self) -> Result<String, &'static str> {
        self.validate()?;
        Ok(self.pipeline_sink_fragment())
    }

    /// Check whether the required GStreamer element is available in the
    /// current installation without constructing a pipeline.
    pub fn sink_element_name(&self) -> &'static str {
        self.protocol.sink_element_name()
    }

    /// Whether this transport's GStreamer element is installed, checked
    /// without building a pipeline: a missing plugin means the transport
    /// should be skipped rather than failing a session later.
    pub fn plugin_available(&self) -> bool {
        gstreamer::ElementFactory::find(self.sink_element_name()).is_some()
    }

    /// The GStreamer sink fragment for these settings, without validating
    /// them first.
    ///
    /// SRT emits a single `srtsink` element (with `passphrase=` only when one
    /// is set); RIST has no URI-based sink, so the fragment muxes
    /// `h264parse ! mpegtsmux ! rtpmp2tpay ! ristsink` and splits the endpoint
    /// into `address=`/`port=` (default port 1968). Prefer
    /// [`Self::validated_pipeline_sink_fragment`] so a wrong scheme cannot
    /// reach the pipeline.
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
                    "h264parse ! mpegtsmux alignment=7 ! rtpmp2tpay ! ristsink address=\"{}\" port={} latency={} ",
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
        assert!(fragment.contains("rtpmp2tpay"));
        assert!(fragment.contains("address=\"receiver\""));
        assert!(fragment.contains("port=10080"));
        assert!(!fragment.contains("uri="));
        assert!(!fragment.contains("application/x-rtp"));
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
