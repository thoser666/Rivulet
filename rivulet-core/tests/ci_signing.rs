//! Regression tests for the release code-signing automation.
//!
//! These tests read the GitHub Actions workflow files and the packaging
//! scripts directly and assert the invariants that make signing actually work:
//! signing is gated on secret-presence outputs (never on raw secrets inside an
//! `if:` expression), the Windows MSI is signed *after* it is built (and the
//! bundled exe *before*), and the macOS app is codesigned, notarized and
//! stapled.

use std::fs;
use std::path::{Path, PathBuf};

/// Resolve a repository-relative path from the `rivulet-core` manifest dir.
fn repo_file(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel)
}

fn read(rel: &str) -> String {
    fs::read_to_string(repo_file(rel)).unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
}

/// 0-based line index of the first line containing `needle`.
fn line_index(text: &str, needle: &str) -> usize {
    text.lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("expected to find {needle:?}"))
}

fn workflows() -> Vec<(&'static str, String)> {
    ["ci.yml", "release.yml"]
        .into_iter()
        .map(|name| (name, read(&format!(".github/workflows/{name}"))))
        .collect()
}

#[test]
fn signing_scripts_exist() {
    let windows = read("packaging/windows/sign.ps1");
    assert!(
        windows.contains("signtool"),
        "Windows script must use signtool"
    );

    let macos = read("packaging/macos/sign-notarize.sh");
    assert!(macos.contains("codesign"), "macOS script must codesign");
    assert!(macos.contains("notarytool"), "macOS script must notarize");
    assert!(macos.contains("stapler"), "macOS script must staple");
}

#[test]
fn workflows_reference_signing_scripts() {
    for (name, content) in workflows() {
        assert!(
            content.contains("packaging/windows/sign.ps1"),
            "{name} must sign via the dedicated Windows script"
        );
        assert!(
            content.contains("packaging/macos/sign-notarize.sh"),
            "{name} must sign via the dedicated macOS script"
        );
    }
}

#[test]
fn signing_is_gated_on_secret_presence_outputs() {
    for (name, content) in workflows() {
        assert!(
            content.contains("id: signing"),
            "{name} must have a signing check step"
        );
        assert!(
            content.contains("steps.signing.outputs.windows_enabled"),
            "{name} must gate Windows signing on the check output"
        );
        assert!(
            content.contains("steps.signing.outputs.macos_enabled"),
            "{name} must gate macOS signing on the check output"
        );

        // Secrets must never be referenced directly in an `if:` condition;
        // GitHub does not evaluate them there.
        for line in content.lines() {
            if line.contains("if:") {
                assert!(
                    !line.contains("secrets."),
                    "{name}: secrets must not be used in if: -> {line}"
                );
            }
        }
    }
}

#[test]
fn windows_msi_is_signed_after_it_is_built() {
    for (name, content) in workflows() {
        let exe_sign = line_index(&content, "Sign Windows executable");
        let build = line_index(&content, "Build portable ZIP + MSI (Windows)");
        let msi_sign = line_index(&content, "Sign Windows MSI");

        assert!(
            exe_sign < build,
            "{name}: exe must be signed before packaging"
        );
        assert!(
            build < msi_sign,
            "{name}: MSI must be signed after it is built"
        );
    }
}

#[test]
fn macos_app_is_built_before_it_is_signed() {
    for (name, content) in workflows() {
        let app = line_index(&content, "Build macOS app bundle");
        let sign = line_index(&content, "Sign & notarize macOS app");
        assert!(app < sign, "{name}: app bundle must exist before signing");
    }
}

#[test]
fn signing_e2e_smoke_tests_are_wired_up() {
    let workflow = read(".github/workflows/signing-e2e.yml");
    assert!(
        workflow.contains("packaging/windows/test-signing.ps1"),
        "e2e workflow must smoke-test the Windows signer"
    );
    assert!(
        workflow.contains("packaging/macos/test-signing.sh"),
        "e2e workflow must smoke-test the macOS signer"
    );

    let windows = read("packaging/windows/test-signing.ps1");
    assert!(
        windows.contains("New-SelfSignedCertificate"),
        "Windows smoke test must generate a self-signed cert"
    );
    assert!(
        windows.contains("signtool verify"),
        "Windows smoke test must verify the signature"
    );

    let macos = read("packaging/macos/test-signing.sh");
    assert!(
        macos.contains("openssl req"),
        "macOS smoke test must generate a self-signed cert"
    );
    assert!(
        macos.contains("codesign --verify"),
        "macOS smoke test must verify the signature"
    );

    // The codesign step is shared between the release path and the test.
    let codesign_app = read("packaging/macos/codesign-app.sh");
    assert!(
        codesign_app.contains("MACOS_SIGN_IDENTITY"),
        "codesign-app.sh must allow an overridable identity"
    );
}
