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
    "dependabot-auto-merge.yml",
    "security.yml",
    "scorecard.yml",
    "distribution-readiness.yml",
];

/// The reviewed pins. `(action@sha, human-readable version)` — the version
/// comment documents which upstream release the SHA corresponds to. Keeping the
/// exact SHA asserted here forces a deliberate, reviewed test change whenever
/// someone updates a pin.
const PINNED_ACTIONS: &[(&str, &str)] = &[
    (
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
        "v7.0.1",
    ),
    (
        "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
        "v7.0.1",
    ),
    (
        "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
        "v8.0.1",
    ),
    (
        "actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9",
        "v6.1.0",
    ),
    (
        "dtolnay/rust-toolchain@4360b52568e2003a75bf9bc1d59f33a8e3fc893c",
        "stable",
    ),
    (
        "softprops/action-gh-release@3d0d9888cb7fd7b750713d6e236d1fcb99157228",
        "v3.0.2",
    ),
    (
        "github/codeql-action/init@42947a340483f03ba47bb1a039b2c519aab3df85",
        "v3.37.8",
    ),
    (
        "github/codeql-action/autobuild@42947a340483f03ba47bb1a039b2c519aab3df85",
        "v3.37.8",
    ),
    (
        "github/codeql-action/analyze@42947a340483f03ba47bb1a039b2c519aab3df85",
        "v3.37.8",
    ),
    (
        "github/codeql-action/upload-sarif@42947a340483f03ba47bb1a039b2c519aab3df85",
        "v3.37.8",
    ),
    (
        "actions/dependency-review-action@2031cfc080254a8a887f58cffee85186f0e49e48",
        "v4.9.0",
    ),
    (
        "actions-rust-lang/audit@72c09e02f132669d52284a3323acdb503cfc1a24",
        "v1.2.7",
    ),
    (
        "EmbarkStudios/cargo-deny-action@3c6349835b2b7b196a839186cb8b78e02f7b5f25",
        "v2.1.1",
    ),
    (
        "ossf/scorecard-action@2d1146689b8cda280b9bc96326124645441f03bc",
        "v2.4.4",
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

#[test]
fn action_pin_reference_doc_is_in_sync() {
    // `docs/ci-action-pins.md` is the human-readable map from SHA to upstream
    // version used when reviewing Dependabot PRs. It must stay in lockstep with
    // the reviewed pins enforced here, so each (SHA, version) pair has to be
    // listed together on a single table row.
    let doc = read("docs/ci-action-pins.md");
    for (action, version) in PINNED_ACTIONS {
        let sha = action
            .split_once('@')
            .map(|(_, sha)| sha)
            .expect("pinned action must carry a SHA");
        let on_one_row = doc
            .lines()
            .any(|line| line.contains(sha) && line.contains(version));
        assert!(
            on_one_row,
            "docs/ci-action-pins.md must map {action} to {version} on a single row"
        );
    }
}

#[test]
fn action_pin_table_is_generated_and_checked() {
    let doc = read("docs/ci-action-pins.md");
    assert!(
        doc.contains("<!-- action-pins-table:start -->")
            && doc.contains("<!-- action-pins-table:end -->"),
        "the pin table must be delimited by the generation markers"
    );
    let generator = read("scripts/generate-action-pins.py");
    assert!(
        generator.contains("action-pins-table:start")
            && generator.contains("action-pins-table:end"),
        "generate-action-pins.py must own the table markers"
    );
    assert!(
        generator.contains("--check"),
        "generate-action-pins.py must support --check mode"
    );
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("generate-action-pins.py"),
        "CI must run generate-action-pins.py --check to catch table drift"
    );
}

#[test]
fn stale_pin_checker_is_wired_up() {
    // Dependabot only runs on its own schedule; this checker closes the gap by
    // comparing every pinned SHA against upstream tags/branches on demand.
    let checker = read("scripts/check-action-pins.py");
    assert!(
        checker.contains("git ls-remote"),
        "check-action-pins.py must resolve SHAs via git ls-remote"
    );
    assert!(
        checker.contains("refs/tags/") && checker.contains("refs/heads/"),
        "check-action-pins.py must distinguish tag-pinned and branch-pinned actions"
    );
    assert!(
        checker.contains("latest") && checker.contains("semver"),
        "check-action-pins.py must compare against the latest stable release"
    );
    assert!(
        checker.contains("outdated in v") && checker.contains("newer major"),
        "check-action-pins.py must report same-major and newer-major staleness separately"
    );
    assert!(
        checker.contains("--fail-on-major"),
        "check-action-pins.py must offer --fail-on-major to make major gaps fatal"
    );
    assert!(
        checker.contains("--json") && checker.contains("json.dumps"),
        "check-action-pins.py must offer a --json machine-readable mode"
    );
    assert!(
        checker.contains("--comment") && checker.contains("render_comment"),
        "check-action-pins.py must offer a --comment Markdown notification mode"
    );
    let nightly = read(".github/workflows/nightly.yml");
    assert!(
        nightly.contains("check-action-pins.py"),
        "the nightly workflow must run the stale-pin checker daily"
    );
    assert!(
        nightly.contains("GITHUB_STEP_SUMMARY"),
        "the nightly workflow must publish the comment to the step summary"
    );
}

#[test]
fn beta_gate_checker_is_wired_up() {
    // The Beta-Gate (README → Roadmap) is the criteria list that gates leaving
    // alpha. The checker must evaluate all six criteria and CI must publish the
    // result to the step summary on every push.
    let checker = read("scripts/check-beta-gate.py");
    assert!(
        checker.contains("M1 – Solid Recording") && checker.contains("M3 – Streaming"),
        "check-beta-gate.py must evaluate the M1 and M3 roadmap criteria"
    );
    assert!(
        checker.contains("Platform parity") && checker.contains("Windows/macOS feature parity"),
        "check-beta-gate.py must evaluate the platform-parity (M5) criterion"
    );
    assert!(
        checker.contains("REQUIRED_SECRETS") && checker.contains("WINDOWS_CERT_BASE64"),
        "check-beta-gate.py must check the code-signing secrets"
    );
    assert!(
        checker.contains("actions/runs") && checker.contains("conclusion"),
        "check-beta-gate.py must check the latest CI run on develop"
    );
    assert!(
        checker.contains("release-blocker"),
        "check-beta-gate.py must check for open release-blocker issues"
    );
    assert!(
        checker.contains("--json")
            && checker.contains("json.dumps")
            && checker.contains("--comment")
            && checker.contains("render_comment"),
        "check-beta-gate.py must offer --json and --comment output modes"
    );
    assert!(
        checker.contains("--fail"),
        "check-beta-gate.py must offer --fail to turn unmet criteria into exit 1"
    );

    // The gate itself lives in the roadmap; the README must define it.
    let readme = read("README.md");
    assert!(
        readme.contains("### Beta-Gate"),
        "README must contain the Beta-Gate section"
    );

    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("check-beta-gate.py"),
        "the CI workflow must run the beta-gate checker"
    );
    assert!(
        ci.contains("GITHUB_STEP_SUMMARY"),
        "the CI workflow must publish the beta-gate result to the step summary"
    );
}

#[test]
fn dependabot_auto_merge_workflow_is_wired_up() {
    let wf = read(".github/workflows/dependabot-auto-merge.yml");
    assert!(
        wf.contains("pull_request_target:"),
        "auto-merge must run in the base-repo context (write access, no PR checkout)"
    );
    assert!(
        wf.contains("github.actor == 'dependabot[bot]'"),
        "auto-merge must be gated to Dependabot PRs only"
    );
    assert!(
        wf.contains("gh pr merge --auto"),
        "auto-merge must enable auto-merge so GitHub merges after required checks pass"
    );
    assert!(
        !wf.contains("gh pr review --approve"),
        "auto-merge must not attempt bot approval, which GitHub Actions tokens reject"
    );
    assert!(
        wf.contains("secrets.GITHUB_TOKEN"),
        "auto-merge must authenticate with the workflow token"
    );
}

#[test]
fn rist_receiver_smoke_is_wired_up() {
    let workflow = read(".github/workflows/ci.yml");
    assert!(workflow.contains("name: RIST Receiver Smoke"));
    assert!(workflow.contains("rist-receiver-smoke.sh"));
    assert!(read("scripts/rist-receiver-smoke.sh").contains("ristsrc"));
    assert!(read("scripts/rist-receiver-smoke.sh").contains("address="));
    let smoke = read("scripts/rist-receiver-smoke.sh");
    assert!(!smoke.contains("ristsink uri="));
    assert!(smoke.contains("h264parse ! mpegtsmux alignment=7 !"));
    assert!(smoke.contains("application/x-rtp") && smoke.contains("ristsink"));
}

#[test]
fn resource_efficiency_gate_is_wired_up() {
    let checker = read("scripts/resource-efficiency-check.py");
    let fixture = read("scripts/resource-efficiency-sample.json");
    let ci = read(".github/workflows/ci.yml");
    assert!(checker.contains("MAX_CPU_DELTA_PERCENT"));
    assert!(checker.contains("MAX_MEMORY_GROWTH_MB"));
    assert!(checker.contains("MAX_FRAME_TIME_REGRESSION_PERCENT"));
    assert!(fixture.contains("schema_version"));
    assert!(fixture.contains("p95_frame_time_ms"));
    assert!(fixture.contains("one_percent_low_fps"));
    assert!(ci.contains("resource-efficiency-check.py"));
}

#[test]
fn release_workflow_reuses_existing_release_branches() {
    let workflow = read(".github/workflows/release.yml");
    assert!(
        workflow.contains("git fetch origin \"$RELEASE_BRANCH\"")
            && workflow.contains("git show-ref --verify")
            && workflow.contains("git push --set-upstream origin \"HEAD:$RELEASE_BRANCH\"")
            && workflow.contains("git diff --cached --quiet")
            && workflow.contains("git reset --hard \"$REMOTE_TIP\""),
        "release workflow must handle retries on an existing release branch idempotently"
    );
}

#[test]
fn develop_required_checks_have_stable_job_names() {
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("name: Pinning-Tests")
            && ci.contains("cargo test -p rivulet-core --test ci_pinning")
            && ci.contains("name: CI")
            && ci.contains(
                "needs: [lints, beta_gate, build_and_test, pinning_tests, srt_receiver_smoke, rist_receiver_smoke]"
            ),
        "CI must expose dedicated Pinning-Tests and aggregate CI checks"
    );

    let security = read(".github/workflows/security.yml");
    assert!(
        security.contains("name: CodeQL (${{ matrix.language }})")
            && security.contains("name: Dependency Review")
            && security.contains("name: Security")
            && security.contains("needs: [codeql, dependency-review, cargo_audit, cargo_deny]"),
        "security.yml must expose stable CodeQL, Dependency Review, and Security check names"
    );
    let scorecard = read(".github/workflows/scorecard.yml");
    assert!(
        scorecard.contains("name: OpenSSF Scorecard"),
        "scorecard.yml must expose a stable OpenSSF Scorecard check name"
    );
}

#[test]
fn distribution_readiness_workflow_is_opt_in_and_dry_run_first() {
    let workflow = read(".github/workflows/distribution-readiness.yml");
    assert!(
        workflow.contains("workflow_dispatch:")
            && workflow.contains("type: choice")
            && workflow.contains("default: true")
            && workflow.contains("DRY_RUN"),
        "distribution workflow must be manually triggered and dry-run-first"
    );
    assert!(
        workflow.contains("platform:")
            && workflow.contains("- winget")
            && workflow.contains("- flathub")
            && workflow.contains("- homebrew")
            && workflow.contains("- steam")
            && workflow.contains("- microsoft-store"),
        "distribution workflow must expose the documented channels"
    );
    assert!(
        workflow.contains("Publishing is intentionally not enabled yet")
            && workflow.contains("contents: read")
            && !workflow.contains("contents: write"),
        "distribution preparation must not publish or request write access"
    );
}

#[test]
fn audit_lockfile_uses_fixed_quick_xml_releases() {
    let lock = read("Cargo.lock").replace("\r\n", "\n");
    let versions: Vec<&str> = lock
        .split("[[package]]")
        .filter(|package| package.contains("name = \"quick-xml\""))
        .filter_map(|package| {
            package
                .lines()
                .find(|line| line.starts_with("version = \""))
                .and_then(|line| line.split('"').nth(1))
        })
        .collect();
    assert!(
        !versions.is_empty(),
        "Cargo.lock must contain the quick-xml package"
    );
    for version in versions {
        let minor = version
            .split('.')
            .nth(1)
            .and_then(|minor| minor.parse::<u64>().ok())
            .expect("quick-xml version must be valid semver");
        assert!(
            minor >= 41,
            "quick-xml {version} is below the RustSec-fixed 0.41 release"
        );
    }
}

#[test]
fn audited_dependencies_remain_on_fixed_releases() {
    let lock = read("Cargo.lock").replace("\r\n", "\n");
    for (name, minimum) in [
        ("anyhow", (1, 0, 103)),
        ("bytes", (1, 11, 1)),
        ("crossbeam-epoch", (0, 9, 20)),
    ] {
        let package = lock
            .split("[[package]]")
            .find(|package| package.contains(&format!("name = \"{name}\"")))
            .unwrap_or_else(|| panic!("Cargo.lock must contain {name}"));
        let version = package
            .lines()
            .find(|line| line.starts_with("version = \""))
            .and_then(|line| line.split('"').nth(1))
            .unwrap_or_else(|| panic!("{name} must have a lockfile version"));
        let mut parts = version.split('.').map(|part| part.parse::<u64>().unwrap());
        let actual = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        assert!(
            actual >= minimum,
            "{name} {version} is below the RustSec-fixed release {}.{}.{}",
            minimum.0,
            minimum.1,
            minimum.2
        );
    }
}

#[test]
fn workspace_manifests_declare_the_workspace_license() {
    for manifest in [
        "rivulet-audio/Cargo.toml",
        "rivulet-capture/Cargo.toml",
        "rivulet-core/Cargo.toml",
        "rivulet-gui/Cargo.toml",
        "rivulet-launcher/Cargo.toml",
        "rivulet-obs-compat/Cargo.toml",
        "rivulet-opengl-hook-dll/Cargo.toml",
        "rivulet-plugins/Cargo.toml",
        "rivulet-streaming/Cargo.toml",
        "rivulet-updater/Cargo.toml",
        "rivulet-vulkan-layer/Cargo.toml",
    ] {
        assert!(
            read(manifest).contains("license.workspace = true"),
            "{manifest} must inherit the workspace license for cargo-deny"
        );
    }
}

#[test]
fn lockfile_does_not_reintroduce_yanked_core2() {
    let lock = read("Cargo.lock");
    assert!(
        !lock.contains("name = \"core2\""),
        "the yanked core2 crate must not return through optional image codecs"
    );
}

#[test]
fn cargo_dependency_security_gates_are_wired_up() {
    let deny = read("deny.toml");
    assert!(
        deny.contains("[advisories]")
            && deny.contains("yanked = \"deny\"")
            && deny.contains("unmaintained = \"workspace\"")
            && deny.contains("[licenses]")
            && deny.contains("[sources]")
            && deny.contains("unknown-registry = \"deny\"")
            && deny.contains("unknown-git = \"deny\""),
        "deny.toml must define advisory, license, and dependency-source policy"
    );

    let workflow = read(".github/workflows/security.yml");
    assert!(
        workflow.contains("name: Cargo Audit")
            && workflow.contains("name: Cargo Deny")
            && workflow.contains("actions-rust-lang/audit@")
            && workflow.contains("denyWarnings: false")
            && workflow.contains("EmbarkStudios/cargo-deny-action@")
            && workflow.contains("CARGO_AUDIT_RESULT")
            && workflow.contains("CARGO_DENY_RESULT"),
        "security.yml must run and aggregate Cargo Audit and Cargo Deny"
    );
}

#[test]
fn security_workflow_enables_codeql_and_dependency_review() {
    let workflow = read(".github/workflows/security.yml");
    assert!(
        workflow.contains("github/codeql-action/init@")
            && workflow.contains("github/codeql-action/autobuild@")
            && workflow.contains("github/codeql-action/analyze@"),
        "security.yml must initialize, build, and analyze with CodeQL"
    );
    assert!(
        workflow.contains("actions/dependency-review-action@")
            && workflow.contains("fail-on-severity: high")
            && workflow.contains("name: Security")
            && workflow.contains("CODEQL_RESULT")
            && workflow.contains("DEPENDENCY_REVIEW_RESULT")
            && workflow.contains("CARGO_AUDIT_RESULT")
            && workflow.contains("CARGO_DENY_RESULT")
            && workflow.contains("actions-rust-lang/audit@")
            && workflow.contains("denyWarnings: false")
            && workflow.contains("EmbarkStudios/cargo-deny-action@"),
        "security.yml must run and aggregate CodeQL, Dependency Review, cargo-audit, and cargo-deny"
    );
    assert!(
        workflow.contains("security-events: write"),
        "CodeQL must be allowed to upload SARIF security events"
    );
    assert!(
        workflow.contains("pull-requests: write")
            && workflow.contains("comment-summary-in-pr: always"),
        "Dependency Review must be able to publish its PR summary"
    );
    assert!(
        workflow.contains("pull_request:") && workflow.contains("branches: [develop]"),
        "security checks must run for pull requests and develop pushes"
    );
}

#[test]
fn scorecard_workflow_is_wired_for_sarif_and_provenance() {
    let workflow = read(".github/workflows/scorecard.yml");
    assert!(
        workflow.contains("name: OpenSSF Scorecard")
            && workflow.contains("ossf/scorecard-action@")
            && workflow.contains("results_format: sarif")
            && workflow.contains("publish_results: true"),
        "scorecard.yml must run OpenSSF Scorecard and publish SARIF results"
    );
    assert!(
        workflow.contains("github/codeql-action/upload-sarif@")
            && workflow.contains("category: openssf-scorecard"),
        "scorecard.yml must upload a distinct Code Scanning SARIF category"
    );
    assert!(
        workflow.contains("id-token: write") && workflow.contains("persist-credentials: false"),
        "Scorecard must use OIDC publication and avoid persisted checkout credentials"
    );
    assert!(
        workflow.contains("actions/upload-artifact@")
            && workflow.contains("if-no-files-found: error"),
        "scorecard.yml must retain the SARIF artifact and fail if it is missing"
    );
}

#[test]
fn build_caches_do_not_restore_stale_target_artifacts() {
    // `target/` contains compiler-version- and dependency-metadata-specific
    // artifacts. Caching it across stable-toolchain updates caused E0460
    // failures (for example, cfg_expr/system_deps) before the build started.
    // Keep only Cargo's downloaded registry/git data in the shared cache.
    for workflow in ["ci.yml", "nightly.yml"] {
        let content = read(&format!(".github/workflows/{workflow}"));
        assert!(
            !content.contains("            target/"),
            "{workflow} must not cache target/ build artifacts"
        );
        assert!(
            content.contains("cargo-registry-${{ hashFiles('**/Cargo.toml') }}"),
            "{workflow} must key registry caches by Cargo manifests"
        );
        assert!(
            !content.contains("restore-keys:"),
            "{workflow} must not restore an unrelated old Cargo cache"
        );
    }
}
