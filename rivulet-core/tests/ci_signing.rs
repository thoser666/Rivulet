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

/// The reusable workflow that owns packaging + signing, shared by ci.yml and
/// release.yml.
fn signing_workflow() -> String {
    read(".github/workflows/build-package.yml")
}

fn all_workflows() -> Vec<(&'static str, String)> {
    [
        "ci.yml",
        "release.yml",
        "build-package.yml",
        "signing-e2e.yml",
    ]
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
fn signing_workflow_references_signing_scripts() {
    let content = signing_workflow();
    assert!(
        content.contains("packaging/windows/sign.ps1"),
        "reusable workflow must sign via the Windows script"
    );
    assert!(
        content.contains("packaging/macos/sign-notarize.sh"),
        "reusable workflow must sign via the macOS script"
    );
}

#[test]
fn signing_is_gated_on_secret_presence_outputs() {
    let content = signing_workflow();
    assert!(
        content.contains("id: signing"),
        "reusable workflow must have a signing check step"
    );
    assert!(
        content.contains("steps.signing.outputs.windows_enabled"),
        "reusable workflow must gate Windows signing on the check output"
    );
    assert!(
        content.contains("steps.signing.outputs.macos_enabled"),
        "reusable workflow must gate macOS signing on the check output"
    );

    // Secrets must never be referenced directly in an `if:` condition;
    // GitHub does not evaluate them there. Applies to every workflow.
    for (name, content) in all_workflows() {
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
    let content = signing_workflow();
    let exe_sign = line_index(&content, "Sign Windows executable");
    let build = line_index(&content, "Build portable ZIP + MSI (Windows)");
    let msi_sign = line_index(&content, "Sign Windows MSI");

    assert!(
        exe_sign < build,
        "exe must be signed before the ZIP/MSI are packaged"
    );
    assert!(build < msi_sign, "MSI must be signed after it is built");
}

#[test]
fn macos_app_is_built_before_it_is_signed() {
    let content = signing_workflow();
    let app = line_index(&content, "Build macOS app bundle");
    let sign = line_index(&content, "Sign & notarize macOS app");
    assert!(app < sign, "app bundle must exist before signing");
}

#[test]
fn caller_workflows_use_the_reusable_workflow() {
    for name in ["ci.yml", "release.yml"] {
        let content = read(&format!(".github/workflows/{name}"));
        assert!(
            content.contains("uses: ./.github/workflows/build-package.yml"),
            "{name} must call the reusable build+package workflow"
        );
        assert!(
            content.contains("secrets: inherit"),
            "{name} must pass secrets to the reusable workflow"
        );
    }
}

#[test]
fn signing_e2e_smoke_tests_are_wired_up() {
    let workflow = read(".github/workflows/signing-e2e.yml");
    assert!(
        workflow.contains("packaging/windows/sign-pester.tests.ps1"),
        "e2e workflow must run the Windows Pester tests"
    );
    assert!(
        workflow.contains("Invoke-Pester"),
        "e2e workflow must run the Windows tests with Pester"
    );
    assert!(
        workflow.contains("packaging/macos/test-signing.sh"),
        "e2e workflow must smoke-test the macOS signer"
    );

    let windows = read("packaging/windows/sign-pester.tests.ps1");
    assert!(
        windows.contains("test-cert.pfx"),
        "Windows Pester tests must use the committed self-signed cert"
    );
    assert!(
        windows.contains("Get-AuthenticodeSignature"),
        "Windows Pester tests must verify the signature"
    );
    assert!(
        windows.contains("remove /s"),
        "Windows Pester tests must strip the pre-existing signature first"
    );

    let macos = read("packaging/macos/test-signing.sh");
    assert!(
        macos.contains("openssl req"),
        "macOS smoke test must generate a self-signed cert"
    );
    assert!(
        macos.contains("extendedKeyUsage=codeSigning"),
        "macOS smoke-test cert must carry the Code Signing EKU"
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

#[test]
fn shellcheck_and_pester_checks_are_wired_up() {
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("shellcheck packaging/macos/*.sh"),
        "CI must run shellcheck on the macOS packaging scripts"
    );

    let e2e = read(".github/workflows/signing-e2e.yml");
    assert!(
        e2e.contains("Invoke-Pester"),
        "signing e2e workflow must run the Windows Pester tests"
    );
}
