//! Configuration contract for an NDI (Network Device Interface) output.
//!
//! NDI is used for LAN contribution and monitoring. Like the SRT/RIST
//! contribution contracts, this module provides a validated, deterministic
//! configuration without requiring the (platform-specific) NewTek NDI runtime
//! to be present at test time. GStreamer exposes the sender as the `ndisink`
//! element from the `ndi` plugin.
//!
//! The engine embeds the sink as an extra branch of the *encoded* H.264 video
//! whenever a recording/streaming session runs with an active output
//! configured (recording, streaming, or dual output) — see
//! [`crate::RivuletEngine::set_ndi_output`]. Publishing is off by default so a
//! feed is never announced unintentionally.

/// NDI output configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NdiOutput {
    /// The NDI source name announced on the local network.
    pub name: String,
    /// Whether the output is actively publishing. Off by default so a NDI feed
    /// is never published unintentionally.
    pub active: bool,
    /// Video group ID (optional group filter, e.g. for VLAN workflows).
    pub group: Option<String>,
}

impl Default for NdiOutput {
    fn default() -> Self {
        Self {
            name: "Rivulet".into(),
            active: false,
            group: None,
        }
    }
}

impl NdiOutput {
    pub const MAX_NAME_LEN: usize = 80;

    pub fn new(name: impl Into<String>, active: bool) -> Self {
        Self {
            name: name.into(),
            active,
            group: None,
        }
    }

    pub fn with_group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    /// Validate the NDI configuration without touching the network runtime.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.trim().is_empty() {
            return Err("NDI source name must not be empty");
        }
        if self.name.trim().chars().count() > Self::MAX_NAME_LEN {
            return Err("NDI source name is too long");
        }
        if let Some(group) = &self.group {
            if group.is_empty() {
                return Err("NDI group must not be empty when set");
            }
        }
        Ok(())
    }

    /// The GStreamer NDI sender element name (`ndisink`), used for a runtime
    /// availability probe.
    pub fn sink_element_name(&self) -> &'static str {
        "ndisink"
    }

    /// Whether the `ndi` GStreamer plugin provides the sender element in the
    /// current installation.
    pub fn plugin_available(&self) -> bool {
        gstreamer::ElementFactory::find(self.sink_element_name()).is_some()
    }

    /// Whether this output should actually publish: it is active **and** the
    /// `ndisink` element exists in the current GStreamer installation.
    pub fn should_publish(&self) -> bool {
        self.active && self.plugin_available()
    }

    /// A validated GStreamer sink fragment. The name is passed as the NDI
    /// source name so the feed is discoverable on the LAN.
    pub fn validated_pipeline_sink_fragment(&self) -> Result<String, &'static str> {
        self.validate()?;
        // Escape double quotes and backslashes so a hostile name cannot break
        // out of the GStreamer parse string into extra pipeline arguments.
        let name = escape(self.name.trim());
        let group = self
            .group
            .as_deref()
            .filter(|group| !group.is_empty())
            .map(|group| format!(" ndi-groups=\"{}\"", escape(group)))
            .unwrap_or_default();
        Ok(format!("ndisink ndi-name=\"{name}\"{group} "))
    }
}

/// Escape double quotes and backslashes for a GStreamer parse-string property.
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndi_output_is_off_by_default() {
        let output = NdiOutput::default();
        assert!(!output.active);
        assert_eq!(output.name, "Rivulet");
    }

    #[test]
    fn ndi_output_validates_name() {
        assert!(NdiOutput::new("Studio Feed", true).validate().is_ok());
        // The GStreamer-escaped fragment escapes quotes in the name to avoid
        // pipeline-injection.
        let fragment = NdiOutput::new("Studio Feed", true)
            .validated_pipeline_sink_fragment()
            .unwrap();
        assert!(fragment.contains("Studio Feed"));
        assert!(fragment.starts_with("ndisink"));
        assert!(NdiOutput::new("", true).validate().is_err());
        let too_long: String = "n".repeat(NdiOutput::MAX_NAME_LEN + 1);
        assert!(NdiOutput::new(too_long, true).validate().is_err());
    }

    #[test]
    fn ndi_output_escapes_quotes_to_prevent_pipeline_breakout() {
        // A malicious name with an embedded quote must not produce raw quotes
        // that could escape the parse string into extra pipeline arguments.
        let output = NdiOutput::new("main \" ; rm -rf /", true);
        let fragment = output.validated_pipeline_sink_fragment().unwrap();
        // The embedded quote must be backslash-escaped (no raw `" ;` joined
        // that could open a second pipeline argument).
        assert!(
            fragment.contains("\\\""),
            "NDI fragment must backslash-escape an embedded quote: {fragment}"
        );
        // Every quote except the two enclosing ones must be escaped (preceded
        // by a backslash). Count raw quotes by subtracting the escaped ones.
        let quotes = fragment.matches('\"').count();
        let escaped = fragment.matches("\\\"").count();
        assert_eq!(
            quotes - escaped,
            2,
            "exactly the two enclosing quotes: {fragment}"
        );
    }

    #[test]
    fn ndi_output_fragment_contains_group_when_set() {
        let fragment = NdiOutput::new("Rivulet Main", true)
            .with_group("streams")
            .validated_pipeline_sink_fragment()
            .unwrap();
        assert!(fragment.contains("streams"));
    }

    #[test]
    fn ndi_plugin_availability_is_deterministic() {
        let _ = gstreamer::init();
        let output = NdiOutput::new("Smoke", true);
        assert_eq!(
            output.plugin_available(),
            gstreamer::ElementFactory::find("ndisink").is_some()
        );
    }
}
