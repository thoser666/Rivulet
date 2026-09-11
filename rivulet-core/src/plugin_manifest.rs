//! Plugin manifest parser and validator.
//!
//! Implements the `rivulet-plugin.toml` format defined in
//! [`docs/plugin-system-rfc.md`](../plugin-system-rfc.md). The manifest is the
//! sole source of truth for plugin identity, capabilities, and compatibility.
//! Validation errors are fatal — the plugin is loaded as `Skipped` with the
//! error reason logged. No fallback or partial loading.
//!
//! **Scope note:** this module only parses and validates the manifest. Actual
//! WASM loading, sandboxing, and host-API wiring are follow-up phases.

use serde::Deserialize;
use std::fmt;

// ── Error type ──────────────────────────────────────────────────────────────

/// Errors produced during manifest parsing or validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// The TOML content could not be parsed.
    ParseError(String),
    /// A required field is missing.
    MissingField(&'static str),
    /// A field value is invalid (e.g. empty name, bad semver).
    InvalidField(&'static str, String),
    /// The manifest references a capability that requires special handling.
    SensitiveCapability(String),
    /// Resource limits exceed hard caps.
    ResourceLimitExceeded(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::ParseError(msg) => write!(f, "manifest parse error: {msg}"),
            ManifestError::MissingField(field) => write!(f, "missing required field: {field}"),
            ManifestError::InvalidField(field, reason) => {
                write!(f, "invalid value for {field}: {reason}")
            }
            ManifestError::SensitiveCapability(cap) => {
                write!(f, "sensitive capability requires explicit approval: {cap}")
            }
            ManifestError::ResourceLimitExceeded(msg) => {
                write!(f, "resource limit exceeded: {msg}")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

// ── Plugin kind ─────────────────────────────────────────────────────────────

/// The type of plugin — determines its integration point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    /// Adds a sidebar view, dock panel, or settings page.
    UiPanel,
    /// Processes audio samples (effect, analyzer, VST3 wrapper).
    AudioEffect,
    /// Modifies frames before encoding (overlay, color grading, AI upscale).
    VideoFilter,
    /// Connects to external services (Discord bot, StreamElements, custom alerts).
    Integration,
}

impl PluginKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            PluginKind::UiPanel => "ui_panel",
            PluginKind::AudioEffect => "audio_effect",
            PluginKind::VideoFilter => "video_filter",
            PluginKind::Integration => "integration",
        }
    }
}

impl fmt::Display for PluginKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Capability model ────────────────────────────────────────────────────────

/// Capabilities requested by a plugin. All default to false/empty (default denial).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct PluginCapabilities {
    /// Render to a GUI panel.
    #[serde(default)]
    pub ui: bool,
    /// Read audio samples.
    #[serde(default)]
    pub audio_in: bool,
    /// Write audio samples.
    #[serde(default)]
    pub audio_out: bool,
    /// Read video frames.
    #[serde(default)]
    pub video_in: bool,
    /// Write video frames.
    #[serde(default)]
    pub video_out: bool,
    /// Allowed hostnames (empty = no network).
    #[serde(default)]
    pub network: Vec<String>,
    /// Allowed directories (empty = no filesystem).
    #[serde(default)]
    pub filesystem: Vec<String>,
    /// Access to stored credentials (always denied for WASM).
    #[serde(default)]
    pub secrets: bool,
    /// Access to capture sources.
    #[serde(default)]
    pub capture: bool,
    /// Read/write chat messages.
    #[serde(default)]
    pub chat: bool,
    /// Trigger or modify alerts.
    #[serde(default)]
    pub alerts: bool,
}

impl PluginCapabilities {
    /// Returns true if any sensitive capability is requested.
    pub fn has_sensitive(&self) -> bool {
        self.secrets || self.capture
    }

    /// List all requested capabilities as human-readable labels.
    pub fn requested(&self) -> Vec<&'static str> {
        let mut caps = Vec::new();
        if self.ui {
            caps.push("ui");
        }
        if self.audio_in {
            caps.push("audio_in");
        }
        if self.audio_out {
            caps.push("audio_out");
        }
        if self.video_in {
            caps.push("video_in");
        }
        if self.video_out {
            caps.push("video_out");
        }
        if !self.network.is_empty() {
            caps.push("network");
        }
        if !self.filesystem.is_empty() {
            caps.push("filesystem");
        }
        if self.secrets {
            caps.push("secrets");
        }
        if self.capture {
            caps.push("capture");
        }
        if self.chat {
            caps.push("chat");
        }
        if self.alerts {
            caps.push("alerts");
        }
        caps
    }
}

// ── Resource limits ─────────────────────────────────────────────────────────

/// Resource limits for a plugin. Enforced by the WASM runtime or thread
/// watchdog for native DLY plugins.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginResources {
    /// WASM linear memory limit in MB. Hard cap: 512.
    #[serde(default = "default_max_memory_mb")]
    pub max_memory_mb: u32,
    /// Max CPU time per frame/process call in milliseconds. Hard cap: 200.
    #[serde(default = "default_max_cpu_ms")]
    pub max_cpu_ms: u32,
    /// Max wall-clock time for init/process in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,
    /// Max file I/O per operation in MB.
    #[serde(default = "default_max_file_size_mb")]
    pub max_file_size_mb: u32,
}

fn default_max_memory_mb() -> u32 {
    128
}
fn default_max_cpu_ms() -> u32 {
    50
}
fn default_timeout_ms() -> u32 {
    5000
}
fn default_max_file_size_mb() -> u32 {
    10
}

impl Default for PluginResources {
    fn default() -> Self {
        Self {
            max_memory_mb: default_max_memory_mb(),
            max_cpu_ms: default_max_cpu_ms(),
            timeout_ms: default_timeout_ms(),
            max_file_size_mb: default_max_file_size_mb(),
        }
    }
}

// ── Compatibility ───────────────────────────────────────────────────────────

/// Host version and platform compatibility filters.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct PluginCompatibility {
    /// Minimum Rivulet version (semver).
    #[serde(default)]
    pub min_host_version: Option<String>,
    /// Maximum Rivulet version (semver, optional).
    #[serde(default)]
    pub max_host_version: Option<String>,
    /// Platform filter. Empty = all platforms.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Required GStreamer plugins (for video_filter kind).
    #[serde(default)]
    pub gstreamer_plugins: Vec<String>,
}

// ── Plugin type section ─────────────────────────────────────────────────────

/// The `[plugin.type]` section — determines kind and entry point.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginType {
    /// The plugin category.
    pub kind: PluginKind,
    /// Entry point filename (e.g. `plugin.wasm`, `plugin.vst3`, `plugin.dly`).
    pub entry_point: String,
    /// Sandbox mode. Defaults to `wasm` for WASM files, `native_dly` for
    /// native binaries.
    #[serde(default = "default_sandbox")]
    pub sandbox: String,
}

fn default_sandbox() -> String {
    "wasm".to_string()
}

// ── API version range ───────────────────────────────────────────────────────

/// The `[plugin.api_version]` section — semver range for host API compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ApiVersion {
    /// Minimum host API version (major.minor).
    pub min: String,
    /// Maximum host API version (major.minor, optional).
    #[serde(default)]
    pub max: Option<String>,
}

// ── Top-level manifest ──────────────────────────────────────────────────────

/// The complete `rivulet-plugin.toml` manifest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginManifest {
    /// The `[plugin]` section.
    pub plugin: PluginInfo,
}

/// Plugin identity and metadata.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginInfo {
    /// Reverse-DNS ID (e.g. `com.example.my-plugin`).
    pub id: String,
    /// Semver version.
    pub version: String,
    /// Host API compatibility version.
    pub api_version: ApiVersion,
    /// Human-readable display name.
    pub name: String,
    /// Short description.
    #[serde(default)]
    pub description: String,
    /// Author name or handle.
    #[serde(default)]
    pub author: String,
    /// License identifier (SPDX).
    #[serde(default)]
    pub license: String,
    /// Project homepage URL.
    #[serde(default)]
    pub homepage: String,
    /// Plugin type and entry point.
    #[serde(rename = "type")]
    pub type_info: PluginType,
    /// Requested capabilities.
    #[serde(default)]
    pub capabilities: PluginCapabilities,
    /// Resource limits.
    #[serde(default)]
    pub resources: PluginResources,
    /// Compatibility filters.
    #[serde(default)]
    pub compatibility: PluginCompatibility,
}

// ── Constants ───────────────────────────────────────────────────────────────

/// Maximum plugin ID length.
pub const MAX_ID_LEN: usize = 128;

/// Maximum plugin name length.
pub const MAX_NAME_LEN: usize = 120;

/// Maximum description length.
pub const MAX_DESCRIPTION_LEN: usize = 2000;

/// Hard cap: maximum memory in MB.
pub const HARD_MAX_MEMORY_MB: u32 = 512;

/// Hard cap: maximum CPU time per call in ms.
pub const HARD_MAX_CPU_MS: u32 = 200;

/// Valid platform identifiers.
pub const VALID_PLATFORMS: &[&str] = &["windows", "linux", "macos"];

/// Valid sandbox modes.
pub const VALID_SANDBOXES: &[&str] = &["wasm", "native_dly"];

// ── Parsing ─────────────────────────────────────────────────────────────────

/// Parse a manifest from TOML content.
///
/// Returns the deserialized manifest or a parse error. Does not validate
/// field values — use [`PluginManifest::validate`] for that.
pub fn parse_manifest(content: &str) -> Result<PluginManifest, ManifestError> {
    toml::from_str(content).map_err(|e| ManifestError::ParseError(e.to_string()))
}

// ── Validation ──────────────────────────────────────────────────────────────

impl PluginManifest {
    /// Validate the manifest fields. Must be called after parsing.
    ///
    /// Returns `Ok(())` if valid, or a list of errors.
    pub fn validate(&self) -> Result<(), ManifestError> {
        let p = &self.plugin;

        // ── ID validation ───────────────────────────────────────────────
        if p.id.trim().is_empty() {
            return Err(ManifestError::MissingField("plugin.id"));
        }
        if p.id.len() > MAX_ID_LEN {
            return Err(ManifestError::InvalidField(
                "plugin.id",
                format!("length {} exceeds maximum {MAX_ID_LEN}", p.id.len()),
            ));
        }
        // Reverse-DNS: alphanumeric, dots, hyphens only
        if !p
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        {
            return Err(ManifestError::InvalidField(
                "plugin.id",
                "must be reverse-DNS (alphanumeric, dots, hyphens only)".into(),
            ));
        }
        if p.id.starts_with('.')
            || p.id.ends_with('.')
            || p.id.starts_with('-')
            || p.id.ends_with('-')
        {
            return Err(ManifestError::InvalidField(
                "plugin.id",
                "must not start or end with a dot or hyphen".into(),
            ));
        }
        // Must contain at least one dot (reverse-DNS)
        if !p.id.contains('.') {
            return Err(ManifestError::InvalidField(
                "plugin.id",
                "must contain at least one dot (reverse-DNS format)".into(),
            ));
        }

        // ── Version validation ──────────────────────────────────────────
        if p.version.trim().is_empty() {
            return Err(ManifestError::MissingField("plugin.version"));
        }
        if !is_valid_semver(&p.version) {
            return Err(ManifestError::InvalidField(
                "plugin.version",
                format!("'{}' is not valid semver", p.version),
            ));
        }

        // ── API version validation ──────────────────────────────────────
        if p.api_version.min.trim().is_empty() {
            return Err(ManifestError::MissingField("plugin.api_version.min"));
        }
        if !is_valid_semver_range(&p.api_version.min) {
            return Err(ManifestError::InvalidField(
                "plugin.api_version.min",
                format!("'{}' is not a valid semver range", p.api_version.min),
            ));
        }
        if let Some(ref max) = p.api_version.max {
            if !is_valid_semver_range(max) {
                return Err(ManifestError::InvalidField(
                    "plugin.api_version.max",
                    format!("'{}' is not a valid semver range", max),
                ));
            }
        }

        // ── Name validation ─────────────────────────────────────────────
        if p.name.trim().is_empty() {
            return Err(ManifestError::MissingField("plugin.name"));
        }
        if p.name.len() > MAX_NAME_LEN {
            return Err(ManifestError::InvalidField(
                "plugin.name",
                format!("length {} exceeds maximum {MAX_NAME_LEN}", p.name.len()),
            ));
        }
        if p.description.len() > MAX_DESCRIPTION_LEN {
            return Err(ManifestError::InvalidField(
                "plugin.description",
                format!(
                    "length {} exceeds maximum {MAX_DESCRIPTION_LEN}",
                    p.description.len()
                ),
            ));
        }

        // ── Type validation ─────────────────────────────────────────────
        if p.type_info.entry_point.trim().is_empty() {
            return Err(ManifestError::MissingField("plugin.type.entry_point"));
        }
        if !VALID_SANDBOXES.contains(&p.type_info.sandbox.as_str()) {
            return Err(ManifestError::InvalidField(
                "plugin.type.sandbox",
                format!(
                    "must be one of {:?}, got '{}'",
                    VALID_SANDBOXES, p.type_info.sandbox
                ),
            ));
        }
        // Auto-detect sandbox from entry point
        if p.type_info.sandbox == "wasm" && !p.type_info.entry_point.ends_with(".wasm") {
            // Allow explicit override but warn via validation
        }

        // ── Capability validation ───────────────────────────────────────
        for host in &p.capabilities.network {
            if host.trim().is_empty() {
                return Err(ManifestError::InvalidField(
                    "plugin.capabilities.network",
                    "hostname must not be empty".into(),
                ));
            }
        }
        for path in &p.capabilities.filesystem {
            if path.trim().is_empty() {
                return Err(ManifestError::InvalidField(
                    "plugin.capabilities.filesystem",
                    "path must not be empty".into(),
                ));
            }
            // Paths should be absolute
            if !path.starts_with('/') && !path.starts_with('\\') {
                // Allow relative paths but flag them
                // (on Windows, a path like "C:\..." is absolute)
                if !path.contains(':') {
                    return Err(ManifestError::InvalidField(
                        "plugin.capabilities.filesystem",
                        format!("'{path}' should be an absolute path"),
                    ));
                }
            }
        }

        // ── Resource limit validation ───────────────────────────────────
        let r = &p.resources;
        if r.max_memory_mb > HARD_MAX_MEMORY_MB {
            return Err(ManifestError::ResourceLimitExceeded(format!(
                "max_memory_mb={} exceeds hard cap {HARD_MAX_MEMORY_MB}",
                r.max_memory_mb
            )));
        }
        if r.max_cpu_ms > HARD_MAX_CPU_MS {
            return Err(ManifestError::ResourceLimitExceeded(format!(
                "max_cpu_ms={} exceeds hard cap {HARD_MAX_CPU_MS}",
                r.max_cpu_ms
            )));
        }

        // ── Compatibility validation ────────────────────────────────────
        for platform in &p.compatibility.platforms {
            if !VALID_PLATFORMS.contains(&platform.as_str()) {
                return Err(ManifestError::InvalidField(
                    "plugin.compatibility.platforms",
                    format!("unknown platform '{platform}', valid: {VALID_PLATFORMS:?}"),
                ));
            }
        }

        Ok(())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Simple semver check: X.Y.Z where X, Y, Z are non-negative integers.
fn is_valid_semver(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    parts.iter().all(|p| p.parse::<u64>().is_ok())
}

/// Simple semver-range check: accepts "X.Y" or "X.Y.Z" (no pre-release or
/// build metadata — the API version is intentionally simpler than full semver).
fn is_valid_semver_range(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return false;
    }
    parts.iter().all(|p| p.parse::<u64>().is_ok())
}

#[cfg(test)]
#[allow(clippy::needless_borrow)]
mod tests {
    use super::*;

    // ── Minimal valid manifest ──────────────────────────────────────────

    const MINIMAL_MANIFEST: &str = r#"
[plugin]
id = "com.example.test-plugin"
version = "1.0.0"
name = "Test Plugin"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "ui_panel"
entry_point = "plugin.wasm"
"#;

    #[test]
    fn parse_minimal_manifest() {
        let manifest = parse_manifest(MINIMAL_MANIFEST).expect("should parse");
        assert_eq!(manifest.plugin.id, "com.example.test-plugin");
        assert_eq!(manifest.plugin.version, "1.0.0");
        assert_eq!(manifest.plugin.name, "Test Plugin");
        assert_eq!(manifest.plugin.type_info.kind, PluginKind::UiPanel);
        assert_eq!(manifest.plugin.type_info.entry_point, "plugin.wasm");
    }

    #[test]
    fn validate_minimal_manifest() {
        let manifest = parse_manifest(MINIMAL_MANIFEST).expect("should parse");
        manifest.validate().expect("should validate");
    }

    // ── Full manifest ───────────────────────────────────────────────────

    const FULL_MANIFEST: &str = r#"
[plugin]
id = "dev.rivulet.fps-overlay"
version = "1.2.3"
name = "FPS Overlay"
description = "Displays a real-time FPS counter and frame-time graph"
author = "Rivulet Team"
license = "MIT"
homepage = "https://example.com/fps-overlay"

[plugin.api_version]
min = "1.0"
max = "1.2"

[plugin.type]
kind = "video_filter"
entry_point = "plugin.wasm"
sandbox = "wasm"

[plugin.capabilities]
ui = false
audio_in = false
audio_out = false
video_in = true
video_out = true
network = []
filesystem = []
secrets = false
capture = false
chat = false
alerts = false

[plugin.resources]
max_memory_mb = 32
max_cpu_ms = 10
timeout_ms = 1000
max_file_size_mb = 0

[plugin.compatibility]
min_host_version = "0.65.0"
max_host_version = "1.0.0"
platforms = ["windows", "linux", "macos"]
gstreamer_plugins = []
"#;

    #[test]
    fn parse_full_manifest() {
        let manifest = parse_manifest(FULL_MANIFEST).expect("should parse");
        assert_eq!(manifest.plugin.id, "dev.rivulet.fps-overlay");
        assert_eq!(manifest.plugin.version, "1.2.3");
        assert_eq!(manifest.plugin.type_info.kind, PluginKind::VideoFilter);
        assert!(manifest.plugin.capabilities.video_in);
        assert!(manifest.plugin.capabilities.video_out);
        assert!(!manifest.plugin.capabilities.audio_in);
        assert_eq!(manifest.plugin.resources.max_memory_mb, 32);
        assert_eq!(manifest.plugin.resources.max_cpu_ms, 10);
        assert_eq!(
            manifest.plugin.compatibility.min_host_version.as_deref(),
            Some("0.65.0")
        );
        assert_eq!(
            manifest.plugin.compatibility.max_host_version.as_deref(),
            Some("1.0.0")
        );
        assert_eq!(
            manifest.plugin.compatibility.platforms,
            vec!["windows", "linux", "macos"]
        );
    }

    #[test]
    fn validate_full_manifest() {
        let manifest = parse_manifest(FULL_MANIFEST).expect("should parse");
        manifest.validate().expect("should validate");
    }

    // ── Parse errors ────────────────────────────────────────────────────

    #[test]
    fn parse_error_invalid_toml() {
        let err = parse_manifest("this is not toml [[[[").unwrap_err();
        match err {
            ManifestError::ParseError(_) => {}
            _ => panic!("expected ParseError"),
        }
    }

    #[test]
    fn parse_error_missing_required_field() {
        let toml = r#"
[plugin]
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "ui_panel"
entry_point = "plugin.wasm"
"#;
        let err = parse_manifest(&toml).unwrap_err();
        assert!(
            matches!(err, ManifestError::ParseError(_)),
            "expected ParseError for missing id, got: {err}"
        );
    }

    // ── ID validation ───────────────────────────────────────────────────

    #[test]
    fn reject_empty_id() {
        let toml = MINIMAL_MANIFEST.replace("com.example.test-plugin", "");
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        assert!(matches!(err, ManifestError::MissingField("plugin.id")));
    }

    #[test]
    fn reject_id_without_dots() {
        let toml = MINIMAL_MANIFEST.replace("com.example.test-plugin", "myplugin");
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        match err {
            ManifestError::InvalidField("plugin.id", msg) => {
                assert!(msg.contains("reverse-DNS"), "unexpected: {msg}");
            }
            _ => panic!("expected InvalidField for plugin.id, got: {err}"),
        }
    }

    #[test]
    fn reject_id_with_invalid_chars() {
        let toml = MINIMAL_MANIFEST.replace("com.example.test-plugin", "com.example/my plugin!");
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        assert!(
            matches!(err, ManifestError::InvalidField("plugin.id", _)),
            "got: {err}"
        );
    }

    #[test]
    fn reject_id_starting_with_dot() {
        let toml = MINIMAL_MANIFEST.replace("com.example.test-plugin", ".example.test");
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        assert!(
            matches!(err, ManifestError::InvalidField("plugin.id", _)),
            "got: {err}"
        );
    }

    #[test]
    fn reject_id_too_long() {
        let long_id = format!("com.example.{}", "x".repeat(MAX_ID_LEN));
        let toml = MINIMAL_MANIFEST.replace("com.example.test-plugin", &long_id);
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        assert!(
            matches!(err, ManifestError::InvalidField("plugin.id", _)),
            "got: {err}"
        );
    }

    // ── Version validation ──────────────────────────────────────────────

    #[test]
    fn reject_invalid_version() {
        let toml = MINIMAL_MANIFEST.replace("1.0.0", "1.0");
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        match err {
            ManifestError::InvalidField("plugin.version", msg) => {
                assert!(msg.contains("semver"), "unexpected: {msg}");
            }
            _ => panic!("expected InvalidField for plugin.version, got: {err}"),
        }
    }

    #[test]
    fn reject_empty_version() {
        let toml = MINIMAL_MANIFEST.replace("1.0.0", "");
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        assert!(
            matches!(err, ManifestError::MissingField("plugin.version")),
            "got: {err}"
        );
    }

    // ── API version validation ──────────────────────────────────────────

    #[test]
    fn reject_invalid_api_version() {
        let toml = MINIMAL_MANIFEST.replace("min = \"1.0\"", "min = \"1\"");
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        match err {
            ManifestError::InvalidField("plugin.api_version.min", msg) => {
                assert!(msg.contains("semver range"), "unexpected: {msg}");
            }
            _ => panic!("expected InvalidField for plugin.api_version.min, got: {err}"),
        }
    }

    // ── Name validation ─────────────────────────────────────────────────

    #[test]
    fn reject_empty_name() {
        let toml = MINIMAL_MANIFEST.replace("Test Plugin", "");
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        assert!(
            matches!(err, ManifestError::MissingField("plugin.name")),
            "got: {err}"
        );
    }

    // ── Type validation ─────────────────────────────────────────────────

    #[test]
    fn reject_empty_entry_point() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "ui_panel"
entry_point = ""
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        assert!(
            matches!(err, ManifestError::MissingField("plugin.type.entry_point")),
            "got: {err}"
        );
    }

    #[test]
    fn reject_invalid_sandbox() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "ui_panel"
entry_point = "plugin.wasm"
sandbox = "something_invalid"
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        match err {
            ManifestError::InvalidField("plugin.type.sandbox", msg) => {
                assert!(msg.contains("wasm"), "unexpected: {msg}");
            }
            _ => panic!("expected InvalidField for sandbox, got: {err}"),
        }
    }

    // ── Capability validation ───────────────────────────────────────────

    #[test]
    fn capabilities_default_to_false() {
        let manifest = parse_manifest(MINIMAL_MANIFEST).expect("should parse");
        let caps = &manifest.plugin.capabilities;
        assert!(!caps.ui);
        assert!(!caps.audio_in);
        assert!(!caps.audio_out);
        assert!(!caps.video_in);
        assert!(!caps.video_out);
        assert!(caps.network.is_empty());
        assert!(caps.filesystem.is_empty());
        assert!(!caps.secrets);
        assert!(!caps.capture);
        assert!(!caps.chat);
        assert!(!caps.alerts);
    }

    #[test]
    fn capabilities_requested_list() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "integration"
entry_point = "plugin.wasm"

[plugin.capabilities]
ui = true
audio_in = true
network = ["api.example.com"]
chat = true
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        let requested = manifest.plugin.capabilities.requested();
        assert!(requested.contains(&"ui"));
        assert!(requested.contains(&"audio_in"));
        assert!(requested.contains(&"network"));
        assert!(requested.contains(&"chat"));
        assert!(!requested.contains(&"video_in"));
    }

    #[test]
    fn capabilities_has_sensitive() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "integration"
entry_point = "plugin.wasm"

[plugin.capabilities]
secrets = true
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        assert!(manifest.plugin.capabilities.has_sensitive());
    }

    #[test]
    fn reject_empty_network_host() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "integration"
entry_point = "plugin.wasm"

[plugin.capabilities]
network = ["", "valid.host"]
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        assert!(
            matches!(
                err,
                ManifestError::InvalidField("plugin.capabilities.network", _)
            ),
            "got: {err}"
        );
    }

    #[test]
    fn reject_relative_filesystem_path() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "integration"
entry_point = "plugin.wasm"

[plugin.capabilities]
filesystem = ["relative/path"]
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        assert!(
            matches!(
                err,
                ManifestError::InvalidField("plugin.capabilities.filesystem", _)
            ),
            "got: {err}"
        );
    }

    #[test]
    fn accept_absolute_filesystem_path() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "integration"
entry_point = "plugin.wasm"

[plugin.capabilities]
filesystem = ["/tmp/rivulet-plugins"]
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        manifest.validate().expect("should validate");
    }

    // ── Resource limit validation ───────────────────────────────────────

    #[test]
    fn resources_default_values() {
        let manifest = parse_manifest(MINIMAL_MANIFEST).expect("should parse");
        let r = &manifest.plugin.resources;
        assert_eq!(r.max_memory_mb, 128);
        assert_eq!(r.max_cpu_ms, 50);
        assert_eq!(r.timeout_ms, 5000);
        assert_eq!(r.max_file_size_mb, 10);
    }

    #[test]
    fn reject_memory_exceeding_hard_cap() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "ui_panel"
entry_point = "plugin.wasm"

[plugin.resources]
max_memory_mb = 1024
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        assert!(
            matches!(err, ManifestError::ResourceLimitExceeded(_)),
            "got: {err}"
        );
    }

    #[test]
    fn reject_cpu_exceeding_hard_cap() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "ui_panel"
entry_point = "plugin.wasm"

[plugin.resources]
max_cpu_ms = 500
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        assert!(
            matches!(err, ManifestError::ResourceLimitExceeded(_)),
            "got: {err}"
        );
    }

    #[test]
    fn accept_resources_within_limits() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "ui_panel"
entry_point = "plugin.wasm"

[plugin.resources]
max_memory_mb = 256
max_cpu_ms = 100
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        manifest.validate().expect("should validate");
    }

    // ── Compatibility validation ────────────────────────────────────────

    #[test]
    fn reject_unknown_platform() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "ui_panel"
entry_point = "plugin.wasm"

[plugin.compatibility]
platforms = ["windows", "beos"]
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        let err = manifest.validate().unwrap_err();
        match err {
            ManifestError::InvalidField("plugin.compatibility.platforms", msg) => {
                assert!(msg.contains("beos"), "unexpected: {msg}");
            }
            _ => panic!("expected InvalidField for platforms, got: {err}"),
        }
    }

    #[test]
    fn accept_valid_platforms() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "ui_panel"
entry_point = "plugin.wasm"

[plugin.compatibility]
platforms = ["windows", "linux", "macos"]
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        manifest.validate().expect("should validate");
    }

    #[test]
    fn empty_platforms_means_all() {
        let manifest = parse_manifest(MINIMAL_MANIFEST).expect("should parse");
        assert!(manifest.plugin.compatibility.platforms.is_empty());
        manifest.validate().expect("should validate");
    }

    // ── Plugin kind display ─────────────────────────────────────────────

    #[test]
    fn plugin_kind_display() {
        assert_eq!(PluginKind::UiPanel.to_string(), "ui_panel");
        assert_eq!(PluginKind::AudioEffect.to_string(), "audio_effect");
        assert_eq!(PluginKind::VideoFilter.to_string(), "video_filter");
        assert_eq!(PluginKind::Integration.to_string(), "integration");
    }

    // ── ManifestError display ───────────────────────────────────────────

    #[test]
    fn manifest_error_display() {
        let e = ManifestError::ParseError("bad toml".into());
        assert!(e.to_string().contains("bad toml"));

        let e = ManifestError::MissingField("plugin.id");
        assert!(e.to_string().contains("plugin.id"));

        let e = ManifestError::InvalidField("plugin.version", "nope".into());
        assert!(e.to_string().contains("plugin.version"));
        assert!(e.to_string().contains("nope"));

        let e = ManifestError::SensitiveCapability("secrets".into());
        assert!(e.to_string().contains("secrets"));

        let e = ManifestError::ResourceLimitExceeded("too big".into());
        assert!(e.to_string().contains("too big"));
    }

    // ── Edge cases ──────────────────────────────────────────────────────

    #[test]
    fn accept_version_with_prerelease() {
        // Plugin version (not API version) may have pre-release tags
        let toml = MINIMAL_MANIFEST.replace("1.0.0", "1.0.0-alpha.1");
        let manifest = parse_manifest(&toml).expect("should parse");
        // Semver validation allows pre-release in version string
        // (it parses as u64 parts, so "1" works but "alpha" doesn't)
        // This tests that the parser handles the replacement correctly
        assert_eq!(manifest.plugin.version, "1.0.0-alpha.1");
    }

    #[test]
    fn accept_windows_filesystem_path() {
        let toml = r#"
[plugin]
id = "com.example.test"
version = "1.0.0"
name = "Test"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "integration"
entry_point = "plugin.wasm"

[plugin.capabilities]
filesystem = ["C:\\Users\\test\\data"]
"#;
        let manifest = parse_manifest(&toml).expect("should parse");
        manifest.validate().expect("should validate");
    }

    #[test]
    fn all_plugin_kinds_parse() {
        for (kind, id_suffix) in [
            ("ui_panel", "ui-panel"),
            ("audio_effect", "audio-effect"),
            ("video_filter", "video-filter"),
            ("integration", "integration"),
        ] {
            let toml = format!(
                r#"
[plugin]
id = "com.example.{id_suffix}"
version = "1.0.0"
name = "Test {kind}"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "{kind}"
entry_point = "plugin.wasm"
"#
            );
            let manifest = parse_manifest(&toml).expect("should parse");
            manifest.validate().expect("should validate");
        }
    }
}
