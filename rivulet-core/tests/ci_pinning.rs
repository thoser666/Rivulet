//! Regression tests for CI supply-chain hardening: every third-party GitHub
//! Action must be pinned to a full commit SHA (not a mutable tag or branch) so
//! a repointed ref cannot silently swap in a malicious commit.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_file(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel)
}

fn read(rel: &str) -> String {
    fs::read_to_string(repo_file(rel)).unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
}

const WORKFLOWS: &[&str] = &[
    "ci.yml",
    "release.yml",
    "nightly.yml",
    "signing-e2e.yml",
    "build-package.yml",
];

/// The reviewed pins. `(action@sha, human-readable version)` — the version
/// comment documents which upstream release the SHA corresponds to. Keeping the
/// exact SHA asserted here forces a deliberate, reviewed test change whenever
/// someone updates a pin.
const PINNED_ACTIONS: &[(&str, &str)] = &[
    (
        "actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09",
        "v5.1.0",
    ),
    (
        "actions/upload-artifact@330a01c490aca151604b8cf639adc76d48f6c5d4",
        "v5.0.0",
    ),
    (
        "actions/download-artifact@634f93cb2916e3fdff6788551b99b062d0335ce0",
        "v5.0.0",
    ),
    (
        "actions/cache@caa296126883cff596d87d8935842f9db880ef25",
        "v5.1.0",
    ),
    (
        "dtolnay/rust-toolchain@4360b52568e2003a75bf9bc1d59f33a8e3fc893c",
        "stable",
    ),
    (
        "softprops/action-gh-release@3bb12739c298aeb8a4eeaf626c5b8d85266b0e65",
        "v2.6.2",
    ),
];

fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[test]
fn third_party_actions_are_pinned_to_full_commit_sha() {
    for name in WORKFLOWS {
        let content = read(&format!(".github/workflows/{name}"));
        for line in content.lines() {
            let Some(rest) = line.trim().strip_prefix("uses:") else {
                continue;
            };
            // Strip an optional inline `# version` comment before parsing.
            let action = rest.split('#').next().unwrap().trim();
            // Local reusable workflows (./...) are part of this repo and are
            // therefore pinned by definition; skip them.
            if action.starts_with("./") {
                continue;
            }
            let (_, refspec) = action.split_once('@').unwrap_or((action, ""));
            assert!(
                is_full_sha(refspec),
                "{name}: action `{action}` must be pinned to a full 40-char commit SHA"
            );
        }
    }
}

#[test]
fn reviewed_action_pins_are_used() {
    let mut all = String::new();
    for name in WORKFLOWS {
        all.push_str(&read(&format!(".github/workflows/{name}")));
        all.push('\n');
    }
    for (action, version) in PINNED_ACTIONS {
        assert!(
            all.contains(action),
            "workflows must pin `{action}` (upstream {version})"
        );
    }
}

#[test]
fn dependabot_updates_pinned_github_actions() {
    // Dependabot is what keeps the SHA pins from going stale: it opens a PR
    // that updates both the commit SHA and the `# vX.Y.Z` comment. Without this
    // entry the pins would freeze forever and only catch up manually.
    let config = read(".github/dependabot.yml");
    let marker = "package-ecosystem: \"github-actions\"";
    let idx = config
        .find(marker)
        .expect("dependabot.yml must configure the github-actions ecosystem");
    let entry = &config[idx..];
    assert!(
        entry.contains("directory: \"/\""),
        "the github-actions entry must scan the repo root"
    );
    assert!(
        entry.contains("interval: \"weekly\""),
        "the github-actions entry must have a weekly schedule"
    );
}
