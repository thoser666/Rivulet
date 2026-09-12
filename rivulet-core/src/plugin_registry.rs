//! Plugin registry: install directory scanning and the user-approval store.
//!
//! This is the Phase 3 core of the plugin system RFC
//! ([`docs/plugin-system-rfc.md`](../../docs/plugin-system-rfc.md)): the host
//! side that decides *which* plugins exist on disk, *which* of those the user
//! has approved (and with which capabilities), and *which* are enabled.
//!
//! Design (RFC §6, §8):
//! - Plugins live in per-plugin bundle directories
//!   (`<install_root>/<plugin-id>/rivulet-plugin.toml` + binary), mirroring
//!   the RFC bundle layout.
//! - Approvals are recorded **per plugin id and capability**, never globally:
//!   the user sees exactly what a plugin requests (default denial, RFC §6.1)
//!   and grants per capability (RFC §6.2). Sensitive capabilities
//!   (`secrets`, `capture`) stay load-time-denied for WASM regardless of the
//!   stored approval (RFC §6.3) — the store keeps them only so the UI can
//!   display the honest "always denied" state.
//! - Everything the GUI persists lives in [`PluginApprovals`] (serde), stored
//!   as `plugin-approvals.json` next to the app config by the GUI; the
//!   directory scan itself is I/O, not state.

use crate::plugin_manifest::{parse_manifest, PluginCapabilities, PluginManifest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ── Discovered plugin (scan result) ─────────────────────────────────────────

/// A plugin bundle found in the install directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPlugin {
    /// Parsed manifest.
    pub manifest: PluginManifest,
    /// Bundle directory (contains `rivulet-plugin.toml`).
    pub bundle_dir: PathBuf,
    /// Path to the plugin binary (`plugin.wasm`).
    pub binary_path: PathBuf,
}

impl DiscoveredPlugin {
    /// Convenience accessors for the UI list.
    pub fn id(&self) -> &str {
        &self.manifest.plugin.id
    }
    pub fn name(&self) -> &str {
        &self.manifest.plugin.name
    }
    pub fn version(&self) -> &str {
        &self.manifest.plugin.version
    }
    pub fn capabilities(&self) -> &PluginCapabilities {
        &self.manifest.plugin.capabilities
    }
}

/// Scan an install root for plugin bundles (RFC §8 layout). A directory
/// counts as a bundle when it contains a parseable `rivulet-plugin.toml`;
/// broken bundles are skipped (never fatal) and reported via `broken`.
pub fn scan_install_root(install_root: &Path) -> (Vec<DiscoveredPlugin>, Vec<String>) {
    let mut found = Vec::new();
    let mut broken = Vec::new();
    let entries = match std::fs::read_dir(install_root) {
        Ok(e) => e,
        Err(_) => return (found, broken), // missing root = no plugins, not an error
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("rivulet-plugin.toml");
        let Ok(text) = std::fs::read_to_string(&manifest_path) else {
            continue; // not a bundle (or unreadable) — ignore silently
        };
        match parse_manifest(&text) {
            Ok(manifest) => {
                let binary = path.join("plugin.wasm");
                if binary.exists() {
                    found.push(DiscoveredPlugin {
                        manifest,
                        bundle_dir: path,
                        binary_path: binary,
                    });
                } else {
                    broken.push(format!("{}: plugin.wasm missing", manifest.plugin.id));
                }
            }
            Err(e) => broken.push(format!("{}: {e}", path.display())),
        }
    }
    found.sort_by(|a, b| a.id().cmp(b.id()));
    (found, broken)
}

/// The default install root: `<app-data>/plugins` (the GUI passes its config
/// directory; kept as a function so tests can use temp dirs).
pub fn default_install_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("plugins")
}

// ── Approval records (persisted) ────────────────────────────────────────────

/// The approval state of one capability of one plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDecision {
    /// The user granted this capability.
    Approved,
    /// The user explicitly denied this capability.
    #[default]
    Denied,
}

/// A single plugin's user decisions. Serialized as a JSON object keyed by
/// plugin id inside [`PluginApprovals`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginRecord {
    /// Per-capability user decisions (capability name → decision).
    pub capabilities: BTreeMap<String, CapabilityDecision>,
    /// Whether the plugin is enabled (only honored when every *requested*
    /// capability has an explicit decision).
    pub enabled: bool,
}

/// The persisted approval store. The GUI keeps this in its own serde state;
/// on disk it lands as `plugin-approvals.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginApprovals {
    /// plugin id → record.
    pub plugins: BTreeMap<String, PluginRecord>,
}

impl PluginApprovals {
    /// Get (or create) the record for a plugin id.
    pub fn record_mut(&mut self, plugin_id: &str) -> &mut PluginRecord {
        self.plugins.entry(plugin_id.to_owned()).or_default()
    }

    pub fn record(&self, plugin_id: &str) -> Option<&PluginRecord> {
        self.plugins.get(plugin_id)
    }

    /// Remove all decisions for a plugin (used by "Forget decision").
    pub fn forget(&mut self, plugin_id: &str) {
        self.plugins.remove(plugin_id);
    }

    /// Approve one capability.
    pub fn approve(&mut self, plugin_id: &str, capability: &str) {
        self.record_mut(plugin_id)
            .capabilities
            .insert(capability.to_owned(), CapabilityDecision::Approved);
    }

    /// Deny one capability.
    pub fn deny(&mut self, plugin_id: &str, capability: &str) {
        self.record_mut(plugin_id)
            .capabilities
            .insert(capability.to_owned(), CapabilityDecision::Denied);
    }

    /// Set the enabled flag for a plugin.
    pub fn set_enabled(&mut self, plugin_id: &str, enabled: bool) {
        self.record_mut(plugin_id).enabled = enabled;
    }

    /// True when every capability the plugin requests has an explicit user
    /// decision (the RFC gate for showing the enable toggle as usable).
    pub fn fully_decided(&self, plugin_id: &str, caps: &PluginCapabilities) -> bool {
        let requested = caps.requested();
        !requested.is_empty()
            && requested.iter().all(|cap| {
                self.plugins
                    .get(plugin_id)
                    .and_then(|r| r.capabilities.get(*cap))
                    .is_some()
            })
    }

    /// The effective runtime grant for one capability of one plugin: approved
    /// by the user for *this* plugin, requested by its manifest, and — for
    /// sensitive capabilities (RFC §6.3) — never granted to WASM regardless
    /// of the stored approval. `is_wasm` is always true for the current
    /// runtime; the parameter keeps the policy in one place for the native
    /// bridge.
    pub fn effective_grant(
        &self,
        plugin_id: &str,
        caps: &PluginCapabilities,
        capability: &str,
        is_wasm: bool,
    ) -> bool {
        // Defense in depth: only capabilities the manifest actually requests
        // can ever be granted.
        if !caps.requested().contains(&capability) {
            return false;
        }
        let approved = self
            .plugins
            .get(plugin_id)
            .and_then(|r| r.capabilities.get(capability))
            == Some(&CapabilityDecision::Approved);
        if !approved {
            return false;
        }
        if is_wasm && matches!(capability, "secrets" | "capture") {
            return false; // RFC §6.3: always denied for WASM
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_toml(caps: &str) -> String {
        manifest_toml_with_id("com.example.demo", caps)
    }

    fn manifest_toml_with_id(id: &str, caps: &str) -> String {
        format!(
            r#"
[plugin]
id = "{id}"
version = "1.0.0"
api_version = {{ min = "1.0", max = "2.0" }}
name = "Demo"
type = {{ kind = "ui_panel", entry_point = "plugin.wasm" }}
[plugin.capabilities]
{caps}
"#
        )
    }

    fn caps(toml_caps: &str) -> PluginCapabilities {
        parse_manifest(&manifest_toml(toml_caps))
            .expect("manifest parses")
            .plugin
            .capabilities
    }

    #[test]
    fn scan_install_root_finds_valid_bundles_and_reports_broken() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("plugins");
        // Valid bundle.
        let good = root.join("com.example.good");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(
            good.join("rivulet-plugin.toml"),
            manifest_toml_with_id("com.example.good", "ui = true"),
        )
        .unwrap();
        std::fs::write(good.join("plugin.wasm"), b"fake wasm").unwrap();
        // Bundle without a binary → broken.
        let no_bin = root.join("com.example.nobin");
        std::fs::create_dir_all(&no_bin).unwrap();
        std::fs::write(
            no_bin.join("rivulet-plugin.toml"),
            manifest_toml_with_id("com.example.nobin", "ui = true"),
        )
        .unwrap();
        // Broken manifest → broken.
        let bad = root.join("com.example.bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("rivulet-plugin.toml"), "not = [ a manifest").unwrap();
        // Random dir without a manifest → ignored silently.
        std::fs::create_dir_all(root.join("unrelated")).unwrap();

        let (found, broken) = scan_install_root(&root);
        assert_eq!(found.len(), 1, "one valid bundle: {found:?}");
        assert_eq!(found[0].id(), "com.example.good");
        assert_eq!(found[0].binary_path, good.join("plugin.wasm"));
        assert_eq!(broken.len(), 2, "nobin + bad manifest reported: {broken:?}");
        assert!(broken
            .iter()
            .any(|b| !b.contains("com.example.good") && b.contains("plugin.wasm missing")));
    }

    #[test]
    fn scan_missing_root_is_empty_not_error() {
        let (found, broken) = scan_install_root(Path::new("/does/not/exist"));
        assert!(found.is_empty());
        assert!(broken.is_empty());
    }

    #[test]
    fn approvals_round_trip_through_serde() {
        let mut approvals = PluginApprovals::default();
        approvals.approve("com.example.demo", "ui");
        approvals.deny("com.example.demo", "audio_in");
        approvals.set_enabled("com.example.demo", true);
        let json = serde_json::to_string(&approvals).unwrap();
        let restored: PluginApprovals = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, approvals);
        assert_eq!(
            restored
                .record("com.example.demo")
                .unwrap()
                .capabilities
                .get("ui"),
            Some(&CapabilityDecision::Approved)
        );
    }

    #[test]
    fn forget_removes_all_decisions() {
        let mut approvals = PluginApprovals::default();
        approvals.approve("com.example.demo", "ui");
        approvals.set_enabled("com.example.demo", true);
        approvals.forget("com.example.demo");
        assert!(approvals.record("com.example.demo").is_none());
    }

    #[test]
    fn sensitive_capabilities_stay_denied_for_wasm_even_when_approved() {
        let caps = caps("secrets = true\ncapture = true\nui = true");
        let mut approvals = PluginApprovals::default();
        approvals.approve("com.example.demo", "secrets");
        approvals.approve("com.example.demo", "capture");
        approvals.approve("com.example.demo", "ui");
        // RFC §6.3: always denied for WASM regardless of the stored approval.
        assert!(!approvals.effective_grant("com.example.demo", &caps, "secrets", true));
        assert!(!approvals.effective_grant("com.example.demo", &caps, "capture", true));
        // Regular capabilities follow the user decision.
        assert!(approvals.effective_grant("com.example.demo", &caps, "ui", true));
        // The native bridge (is_wasm = false) may get sensitive grants.
        assert!(approvals.effective_grant("com.example.demo", &caps, "secrets", false));
    }

    #[test]
    fn effective_grant_requires_explicit_user_decision_per_plugin() {
        let caps = caps("ui = true\naudio_in = true");
        let mut approvals = PluginApprovals::default();
        // No decision at all → denied (default denial, RFC §6.1).
        assert!(!approvals.effective_grant("com.example.demo", &caps, "ui", true));
        // Another plugin's approval must not leak into this plugin.
        approvals.approve("com.other.plugin", "ui");
        assert!(!approvals.effective_grant("com.example.demo", &caps, "ui", true));
        // Explicit deny wins.
        approvals.deny("com.example.demo", "ui");
        assert!(!approvals.effective_grant("com.example.demo", &caps, "ui", true));
        // Explicit approve for the right plugin grants.
        approvals.approve("com.example.demo", "ui");
        assert!(approvals.effective_grant("com.example.demo", &caps, "ui", true));
        // A capability the manifest does not request can never be granted.
        assert!(!approvals.effective_grant("com.example.demo", &caps, "video_out", true));
    }

    #[test]
    fn fully_decided_requires_every_requested_capability() {
        let demo_caps = caps("ui = true\naudio_in = true");
        let mut approvals = PluginApprovals::default();
        assert!(!approvals.fully_decided("com.example.demo", &demo_caps));
        approvals.approve("com.example.demo", "ui");
        assert!(!approvals.fully_decided("com.example.demo", &demo_caps));
        approvals.deny("com.example.demo", "audio_in");
        assert!(approvals.fully_decided("com.example.demo", &demo_caps));
        // A plugin with no requested capabilities is never "fully decided"
        // (there is nothing to review, so the approval flow does not apply).
        let empty_caps = caps("");
        assert!(!approvals.fully_decided("com.example.demo", &empty_caps));
    }

    #[test]
    fn default_decision_is_denied() {
        let record = PluginRecord::default();
        assert_eq!(
            record.capabilities.get("ui"),
            None,
            "no decision recorded yet"
        );
        assert!(!record.enabled);
    }
}
