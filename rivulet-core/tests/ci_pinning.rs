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
        "softprops/action-gh-release@efb35369e0ad2afab669f228072c1b0d510eae64",
        "v3.0.3",
    ),
    (
        "github/codeql-action/init@cdf488f595d80d6e07e03d4674febd5ab45fa938",
        "v4.37.9",
    ),
    (
        "github/codeql-action/autobuild@cdf488f595d80d6e07e03d4674febd5ab45fa938",
        "v4.37.9",
    ),
    (
        "github/codeql-action/analyze@cdf488f595d80d6e07e03d4674febd5ab45fa938",
        "v4.37.9",
    ),
    (
        "github/codeql-action/upload-sarif@cdf488f595d80d6e07e03d4674febd5ab45fa938",
        "v4.37.9",
    ),
    (
        "actions/dependency-review-action@a1d282b36b6f3519aa1f3fc636f609c47dddb294",
        "v5.0.0",
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
fn wiki_policy_and_user_guide_are_linked() {
    let readme = read("README.md");
    let policy = read("docs/wiki-content-policy.md");
    assert!(readme.contains("github.com/thoser666/Rivulet/wiki"));
    assert!(readme.contains("docs/wiki-content-policy.md"));
    for required in [
        "## Zweck des Wikis",
        "## Was im Repository bleiben muss",
        "## Pflege-Regeln",
        "## Aktivierung und initiale Einrichtung",
        "Definition of Done",
        "docs/user-guide.md",
    ] {
        assert!(
            policy.contains(required),
            "wiki policy must contain {required}"
        );
    }
}

#[test]
fn user_guide_is_linked_and_covers_core_workflows() {
    let readme = read("README.md");
    let guide = read("docs/user-guide.md");
    assert!(readme.contains("docs/user-guide.md"));
    for required in [
        "## 3. Eine Aufnahme erstellen",
        "## 5. Szenen und Quellen",
        "## 6. Live-Vorschau",
        "## 7. Streaming einrichten",
        "## 9. Updates",
        "## 10. Logs und Fehler melden",
        "Waiting",
        "Fallback",
        "3010",
    ] {
        assert!(guide.contains(required), "user guide must cover {required}");
    }
}

#[test]
fn m3_completion_report_is_linked_and_records_follow_ups() {
    let readme = read("README.md");
    let report = read("docs/m3-streaming-completion-report.md");
    assert!(readme.contains("docs/m3-streaming-completion-report.md"));
    for required in [
        "## Summary",
        "## Findings and explicit follow-ups",
        "## Decision",
        "CONDITIONAL PASS",
        "F-M3-001",
        "F-M3-002",
        "F-M3-006",
        "#70",
        "#77",
    ] {
        assert!(
            report.contains(required),
            "M3 completion report must contain {required}"
        );
    }
}

#[test]
fn security_policy_is_linked_from_readme_and_docs() {
    let readme = read("README.md");
    let policy = read("SECURITY.md");
    let security_docs = read("docs/security.md");
    assert!(readme.contains("SECURITY.md"));
    assert!(security_docs.contains("SECURITY.md"));
    for required in [
        "## Reporting a Vulnerability",
        "security/advisories/new",
        "## Scope",
        "## Supported Versions",
        "## Coordinated Disclosure",
        "docs/m3-streaming-completion-report.md",
    ] {
        assert!(
            policy.contains(required),
            "SECURITY.md must contain {required}"
        );
    }
}

#[test]
fn contributing_defines_code_review_policy_and_definition_of_done() {
    // OpenSSF Best Practices (passing): the contribution instructions must
    // explain the contribution process and the requirements for acceptable
    // contributions (test policy, coding standard), and the project must
    // document how changes are reviewed. These live in CONTRIBUTING.md and
    // must not drift out of existence, otherwise the badge evidence map in
    // docs/openssf-best-practices.md would silently become stale.
    let contributing = read("CONTRIBUTING.md");
    for marker in [
        "## Code Review Policy",
        "Pull requests for contributors",
        "Maintainer direct pushes",
        "Sensitive changes always get a PR",
        "Automated review is part of review",
        "## Definition of Done (Acceptable Contributions)",
        "Tests for new functionality",
        "i18n parity",
        "Documentation",
        "Checks are green",
        "Commit convention",
        "No secrets",
        "## Release Verification",
        "Built from the tested commit",
        "Notes are complete",
        "Fixed vulnerabilities are identified",
        "Artifacts are integrity-checked",
        "release_notes_vulns",
        "CVE/GHSA/RUSTSEC",
    ] {
        assert!(
            contributing.contains(marker),
            "CONTRIBUTING.md must document the OpenSSF passing-badge policy: {marker}"
        );
    }

    // The OpenSSF evidence map must exist, carry the three mapped sections
    // and link back to the artifacts it vouches for.
    let map = read("docs/openssf-best-practices.md");
    for marker in [
        "# OpenSSF Best Practices Badge — Evidence Map",
        "## Contribution process",
        "## Code review policy",
        "## Release verification",
        "## How this page stays true",
        "## Remaining (maintainer action, not repo files)",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "rivulet-core/tests/ci_pinning.rs",
        "SHA256SUMS",
        "bestpractices.dev",
    ] {
        assert!(
            map.contains(marker),
            "docs/openssf-best-practices.md must contain {marker}"
        );
    }
}

/// Canonical OSPS baseline-1 criterion keys (lowercase `osps_*` form used by
/// `.bestpractices.json` and proposal URLs), from the OpenSSF Baseline
/// criteria (`criteria/baseline_criteria.yml`, OSPS v2026.08.28 — 24 controls).
const BASELINE_1_CRITERIA: &[&str] = &[
    "osps_ac_01_01", // OSPS-AC-01.01 MFA on sensitive repository access
    "osps_ac_02_01", // OSPS-AC-02.01 collaborators get least privilege
    "osps_ac_03_01", // OSPS-AC-03.01 no direct commits to primary branch
    "osps_ac_03_02", // OSPS-AC-03.02 primary-branch deletion protected
    "osps_br_01_01", // OSPS-BR-01.01 untrusted metadata sanitized in CI
    "osps_br_01_03", // OSPS-BR-01.03 untrusted code has no CI credentials
    "osps_br_03_01", // OSPS-BR-03.01 official URIs over encrypted channels
    "osps_br_03_02", // OSPS-BR-03.02 distribution MITM-protected
    "osps_br_07_01", // OSPS-BR-07.01 no unencrypted secrets in VCS
    "osps_do_01_01", // OSPS-DO-01.01 user guides for basic functionality
    "osps_do_02_01", // OSPS-DO-02.01 defect-reporting guide
    "osps_gv_02_01", // OSPS-GV-02.01 public discussion mechanism
    "osps_gv_03_01", // OSPS-GV-03.01 contribution process documented
    "osps_le_02_01", // OSPS-LE-02.01 source license OSI/FSF approved
    "osps_le_02_02", // OSPS-LE-02.02 released-asset license OSI/FSF approved
    "osps_le_03_01", // OSPS-LE-03.01 LICENSE file maintained in the repo
    "osps_le_03_02", // OSPS-LE-03.02 license shipped with release assets
    "osps_qa_01_01", // OSPS-QA-01.01 public repo at a static URL
    "osps_qa_01_02", // OSPS-QA-01.02 public change record (who/when)
    "osps_qa_02_01", // OSPS-QA-02.01 dependency list
    "osps_qa_04_01", // OSPS-QA-04.01 multi-repo codebase list
    "osps_qa_05_01", // OSPS-QA-05.01 no generated executable artifacts
    "osps_qa_05_02", // OSPS-QA-05.02 no unreviewable binary artifacts
    "osps_vm_02_01", // OSPS-VM-02.01 security contacts documented
];

#[test]
fn bestpractices_json_claims_are_schema_valid_and_canonical() {
    // `.bestpractices.json` is the draft automation-proposal file the OpenSSF
    // badge site reads from the repository (see docs/bestpractices-json.md).
    // Because the site silently ignores unknown field names and invalid status
    // values, a typo'd criterion name or a misspelled status would make a
    // claim vanish without any error — the badge would show fewer "Met" rows
    // than the file intends. This guard makes the file's schema a CI
    // contract: it must parse, may only use canonical baseline-1 keys, and
    // may only carry status values the site understands.
    let raw = read(".bestpractices.json");
    let doc: serde_json::Value =
        serde_json::from_str(&raw).expect(".bestpractices.json must be valid JSON");
    let obj = doc
        .as_object()
        .expect(".bestpractices.json must be a JSON object");

    // Non-criteria fields the site accepts in any section, plus our internal
    // documentation comment (unknown keys are ignored by the site).
    let headers = [
        "name",
        "description",
        "license",
        "implementation_languages",
        "_comment",
    ];
    // Legal status values, case-insensitive with surrounding whitespace
    // stripped (site rule); `?`/`unknown` mean "no information" and are
    // ignored entirely by the site, so they are legal placeholders.
    let legal_statuses = ["met", "unmet", "n/a", "?", "unknown"];

    for (key, value) in obj {
        if let Some(criterion) = key.strip_suffix("_status") {
            assert!(
                BASELINE_1_CRITERIA.contains(&criterion),
                ".bestpractices.json: `{key}` is not a canonical baseline-1 criterion key"
            );
            let status = value
                .as_str()
                .unwrap_or_else(|| panic!(".bestpractices.json: `{key}` must be a string"));
            let norm = status.trim().to_ascii_lowercase();
            assert!(
                legal_statuses.contains(&norm.as_str()),
                ".bestpractices.json: `{key}` has illegal status value {status:?} \
                 (expected Met, Unmet, N/A, ? or unknown)"
            );
        } else if let Some(criterion) = key.strip_suffix("_justification") {
            assert!(
                BASELINE_1_CRITERIA.contains(&criterion),
                ".bestpractices.json: `{key}` is not a canonical baseline-1 criterion key"
            );
            assert!(
                value.is_string(),
                ".bestpractices.json: `{key}` must be a string"
            );
            // A justification without its paired status would be ignored by
            // the site and mislead the reviewer about the claim it supports.
            let status_key = format!("{criterion}_status");
            assert!(
                obj.contains_key(&status_key),
                ".bestpractices.json: `{key}` has no paired `{status_key}`"
            );
        } else {
            assert!(
                headers.contains(&key.as_str()),
                ".bestpractices.json: unexpected key `{key}` (not a canonical \
                 baseline-1 criterion field or allowed header)"
            );
        }
    }

    // Every concrete claim (Met/Unmet/N/A — anything except the ignored
    // unknown placeholders) must carry a non-empty justification, so a bare
    // "Met" without evidence cannot silently reach the review form.
    for (key, value) in obj {
        let Some(criterion) = key.strip_suffix("_status") else {
            continue;
        };
        let status = value.as_str().expect("status string checked above");
        let norm = status.trim().to_ascii_lowercase();
        if norm == "?" || norm == "unknown" {
            continue;
        }
        let just_key = format!("{criterion}_justification");
        let justification = obj
            .get(&just_key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(
            !justification.trim().is_empty(),
            ".bestpractices.json: `{key}` claims {status:?} but has no \
             non-empty `{just_key}`"
        );
    }

    // The evidence map must document the file and this very guard, so the
    // schema contract cannot be deleted without a conscious doc+test change.
    let map = read("docs/openssf-best-practices.md");
    assert!(
        map.contains(".bestpractices.json"),
        "docs/openssf-best-practices.md must reference .bestpractices.json"
    );
    assert!(
        map.contains("bestpractices_json_claims_are_schema_valid_and_canonical"),
        "docs/openssf-best-practices.md must list the .bestpractices.json guard"
    );
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
    assert!(
        !config.contains("package-ecosystem: \"cargo\""),
        "Dependabot must not duplicate Renovate's Cargo update ownership"
    );
}

#[test]
fn renovate_owns_grouped_cargo_updates() {
    let config = read("renovate.json");
    assert!(config.contains("\"enabledManagers\": [\"cargo\"]"));
    assert!(config.contains("\":dependencyDashboard\""));
    assert!(config.contains("\"groupName\": \"Rust dependencies\""));
    assert!(config.contains("\"dependencyDashboardApproval\": true"));
    assert!(
        !config.contains("github-actions"),
        "Renovate must not compete with Dependabot for GitHub Actions pins"
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
    assert!(
        checker.contains("timeout=30"),
        "check-action-pins.py must bound upstream lookups"
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
    assert!(
        nightly.contains("--fail-on-major"),
        "the nightly workflow must treat newer-major gaps as fatal (--fail-on-major)"
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
    assert!(workflow.contains("name: Build RIST smoke image for diagnostics"));
    assert!(
        workflow.contains("docker build --pull=true -t rivulet-rist-smoke:ci docker/rist-smoke")
    );
    let inspect = workflow
        .find("name: Inspect RIST plugin")
        .expect("RIST inspection step");
    let build = workflow
        .find("name: Build RIST smoke image for diagnostics")
        .expect("RIST diagnostic image build");
    assert!(
        build < inspect,
        "the diagnostic image must be built before it is inspected"
    );
    assert!(workflow.contains("rist-receiver-smoke.sh"));
    let smoke_script = read("scripts/rist-receiver-smoke.sh");
    assert!(smoke_script.contains("ristsrc"));
    assert!(smoke_script.contains("gst-launch-1.0 -e"));
    assert!(smoke_script.contains("identity silent=false dump=true"));
    assert!(smoke_script.contains("received_buffers"));
    assert!(smoke_script.contains("[[:xdigit:]]{8}"));
    assert!(smoke_script.contains("hexadecimal buffer dump"));
    assert!(!smoke_script.contains("gst-launch-1.0.0"));
    assert!(smoke_script.contains("address="));
    assert!(smoke_script.contains("timeout --signal=TERM --kill-after=5s 30s"));
    assert!(smoke_script.contains("did not remain running"));
    let smoke = read("scripts/rist-receiver-smoke.sh");
    assert!(!smoke.contains("ristsink uri="));
    assert!(smoke.contains("h264parse ! mpegtsmux alignment=7 !"));
    assert!(smoke.contains("rtpmp2tpay"));
    assert!(smoke.contains("ristsink"));
    assert!(smoke.contains("sender_status"));
    assert!(smoke.contains("-ne 124"));
    assert!(!smoke.contains("application/x-rtp"));
    let checker = read("scripts/check-rist-pipeline.py");
    assert!(checker.contains("RIST pipeline contract OK"));
}

#[test]
fn obs_websocket_smoke_is_wired_up() {
    let workflow = read(".github/workflows/ci.yml");
    assert!(workflow.contains("name: OBS WebSocket Smoke"));
    let smoke_job = workflow
        .find("name: OBS WebSocket Smoke")
        .expect("OBS WebSocket smoke job");
    let aggregate = workflow.find("    name: CI").expect("CI aggregate job");
    assert!(
        smoke_job < aggregate,
        "the OBS WebSocket smoke job must run before the CI aggregate"
    );
    assert!(
        workflow.contains("cargo test -p rivulet-obs-websocket --test client_smoke -- --nocapture"),
        "CI must run the real-client smoke test against the server"
    );
    assert!(
        workflow.contains("needs.obs_websocket_smoke.result"),
        "the CI aggregate must require the OBS WebSocket smoke result"
    );
    // The smoke must drive the server over a real loopback TCP connection.
    let smoke = read("rivulet-obs-websocket/tests/client_smoke.rs");
    assert!(smoke.contains("real (non-mock) WebSocket client"));
    assert!(smoke.contains("ws://127.0.0.1:{port}"));
    // Regression: the accept loop runs a non-blocking listener and accepted
    // sockets on Windows inherit that mode, which made tungstenite's handshake
    // read fail intermittently (`Protocol(HandshakeIncomplete)`) under
    // parallel test load. Blocking must be restored **before** the handshake.
    let server = read("rivulet-obs-websocket/src/server.rs");
    let accept = server
        .split_once("fn run_session")
        .map(|(_, rest)| rest)
        .expect("run_session must exist");
    let handshake = accept
        .find("tungstenite::accept_hdr")
        .expect("accept_hdr call");
    let blocking = accept
        .find("set_nonblocking(false)")
        .expect("set_nonblocking call");
    assert!(
        blocking < handshake,
        "set_nonblocking(false) must come before accept_hdr"
    );
    // The auth-close smoke must use the same retry as TestClient::connect so a
    // transient handshake drop cannot flake it.
    assert!(smoke.contains("fn connect_with_retry"));
    // Handshake-under-load must stay covered permanently: the parallel burst
    // test connects many clients at once and requires every one plus a fresh
    // client afterwards to complete Hello/Identify and a request round-trip.
    assert!(smoke.contains("fn parallel_clients_all_complete_handshake_under_load"));
    assert!(smoke.contains("const CLIENTS: usize = 24;"));
    assert!(smoke.contains("std::sync::Barrier"));
    assert!(smoke.contains("TestClient::connect(port, None)"));
}

#[test]
fn responsive_layout_contract_is_pinned_in_the_gui() {
    // Shrinking the window must never clip controls without a way to reach
    // them: the central view content and the sidebar live in scroll areas and
    // the window has a minimum inner size (guards in main.rs).
    let app = read("rivulet-gui/src/app.rs");
    let main = read("rivulet-gui/src/main.rs");
    assert!(app.contains("egui::ScrollArea::vertical()"));
    assert!(app.contains("auto_shrink([false, false])"));
    assert!(app.contains("nav_panel"));
    assert!(main.contains("with_min_inner_size"));
    // The source-contract tests must stay wired to these guarantees.
    let smoke = read("rivulet-gui/tests/ui_smoke.rs");
    assert!(smoke.contains("responsive_contract_keeps_controls_reachable_on_narrow_windows"));
    let accessibility = read("rivulet-gui/tests/ui_accessibility.rs");
    assert!(accessibility.contains("fn narrow_layout_is_responsive"));
    let regression = read("rivulet-gui/tests/ui_regression.rs");
    assert!(regression.contains("640, 480"), "640x480 must be covered");
    assert!(regression.contains("content=scrollable"));
}

#[test]
fn discord_presence_tracks_record_state_from_every_view() {
    // The Discord adapter must stay in sync with recording/streaming
    // transitions regardless of which view is open: the per-frame reconcile
    // call lives in the ui() entry next to the other reconcilers, NOT only
    // inside the Stream view's draw code. Reverting this lets the presence
    // freeze on "Ready" while the user records from the Record view.
    let gui = read("rivulet-gui/src/app.rs");
    assert!(gui.contains("fn sync_discord_presence"));
    assert!(gui.contains("fn current_presence_status"));
    // The frame-level call must sit INSIDE the ui() entry body, after the
    // reconcile block anchor (fn ui( ... self.reconcile_midi();), NOT merely
    // inside the Stream view's draw_presence_status. Both calls may coexist;
    // the ui() one is what keeps Record-view toggles in sync.
    let ui_fn = gui
        .find("fn ui(&mut self, ui: &mut egui::Ui")
        .expect("ui() entry");
    let body: &str = &gui[ui_fn..];
    assert!(
        body.contains("self.reconcile_midi();"),
        "reconcile block must exist in ui() as the anchor"
    );
    assert!(
        body.contains("self.sync_discord_presence();"),
        "per-frame presence sync must live in the ui() reconcile block"
    );
}

#[test]
fn discord_app_id_retirement_chain_is_guarded() {
    // Rationiertes Update der offiziellen App-ID: die Retirement-Kette
    // (konfigurierte ID → offizieller Default → Adapter aus) muss im Core
    // als zentrale Helper existieren und von der GUI benutzt werden. Das
    // Updater-Manifest ist die Release-Payload selbst — ein ID-Wechsel wird
    // über einen DEFAULT_CLIENT_ID-Bump + Release-Notes-Eintrag ausgerollt.
    let discord = read("rivulet-core/src/discord.rs");
    let chain_helpers = discord.contains("pub fn effective_client_id")
        && discord.contains("pub fn effective_large_image_key");
    assert!(
        chain_helpers,
        "the fallback-chain helpers must exist in the core module"
    );

    let gui = read("rivulet-gui/src/app.rs");
    let chain_wired = gui.contains("effective_client_id(Some(&client_id))")
        && gui.contains("effective_large_image_key(Some(&large_image))");
    assert!(
        chain_wired,
        "the adapter reconcile must resolve ids through the fallback chain"
    );

    // The rotation procedure must stay documented in the versioned docs so
    // the runbook travels with the code that implements it.
    let docs = read("docs/activity-status.md");
    assert!(
        docs.contains("## Runbook: rotating the official application id")
            && docs.contains("DEFAULT_CLIENT_ID")
            && docs.contains("SET_ACTIVITY delivered"),
        "activity-status.md must keep the id-rotation runbook (code bump, \
         release-notes text, verification steps)"
    );
}

#[test]
fn discord_presence_ships_official_defaults_and_migrates_empty_ids() {
    // Zero-config Rich Presence: the core config must default to the official
    // application id (validated snowflake) plus the official logo asset key,
    // and the GUI must start with those defaults and migrate restores with an
    // empty persisted id to them (custom ids stay untouched).
    let discord = read("rivulet-core/src/discord.rs");
    assert!(
        discord.contains("pub const DEFAULT_CLIENT_ID: &str = \"1544027006847680532\";"),
        "the official application id must be the shipped default"
    );
    assert!(
        discord.contains("pub const DEFAULT_LARGE_IMAGE_KEY: &str = \"rivulet_logo\";"),
        "the official logo asset key must be the shipped default"
    );
    assert!(
        discord.contains("large_image_key: Some(DEFAULT_LARGE_IMAGE_KEY.to_owned())"),
        "DiscordPresenceConfig::default must enable the logo"
    );

    let gui = read("rivulet-gui/src/app.rs");
    assert!(
        gui.contains(
            "discord_presence_client_id: rivulet_core::discord::DEFAULT_CLIENT_ID.to_owned()"
        ),
        "a fresh install must start with the official client id"
    );
    assert!(
        gui.contains("fn restore_from_storage")
            && gui.contains("Discord client id was empty - applying the official default"),
        "the restore path must migrate empty persisted ids to the default"
    );
}

#[test]
fn discord_presence_error_state_is_wired_into_the_status_model() {
    // PresenceActivity::Error must be produced by the presence status logic
    // when the engine reported a failure (last_error set), take priority over
    // the activity labels, and never leak the raw error text. Streaming starts
    // must clear a stale error so the status recovers.
    let gui = read("rivulet-gui/src/app.rs");
    assert!(gui.contains("PresenceActivity::Error"));
    assert!(
        gui.contains("fn current_presence_activity"),
        "error must have priority in current_presence_activity"
    );
    assert!(
        gui.contains("// Clear a stale error so a new stream starts"),
        "streaming starts must clear last_error"
    );
    // The privacy guarantee lives in the presence payload builder.
    let presence = read("rivulet-core/src/presence.rs");
    assert!(presence.contains("PresenceActivity::Error"));
    assert!(presence.contains("Self::Error => \"presence_error\""));
    let i18n = read("rivulet-core/src/i18n.rs");
    assert!(i18n.matches("\"presence_error\"").count() >= 2);
}

#[test]
fn discord_framing_uses_the_opcode_length_header() {
    // Regression: the adapter framed every IPC message with only a 4-byte
    // length prefix, but Discord v1 requires the 8-byte header
    // `[opcode:u32][length:u32]` (op 0 = HANDSHAKE, op 1 = FRAME). Discord
    // rejected the old framing with `{"code":1003,"message":"protocol
    // error"}` and closed the connection, so the presence never appeared
    // even though the client id and pipe were correct.
    let discord = read("rivulet-core/src/discord.rs");
    assert!(
        discord.contains("pub const HANDSHAKE: u32 = 0;"),
        "opcode table must define HANDSHAKE"
    );
    assert!(
        discord.contains("pub const FRAME: u32 = 1;"),
        "opcode table must define FRAME"
    );
    assert!(
        discord.contains("write_frame(w, op::HANDSHAKE, &bytes)"),
        "handshake must be sent with the HANDSHAKE opcode"
    );
    assert!(
        discord.contains("write_frame(w, op::FRAME, &bytes)"),
        "SET_ACTIVITY must be sent with the FRAME opcode"
    );
    // The writer must emit the 8-byte header (opcode first, then length).
    let write = discord
        .split_once("fn write_frame")
        .map(|(_, rest)| rest)
        .expect("write_frame must exist");
    assert!(
        write.contains("opcode.to_le_bytes()") && write.contains("len.to_le_bytes()"),
        "write_frame must write opcode then length"
    );
    // The wire tests must assert the opcodes end to end.
    assert!(discord.contains("handshake must use the HANDSHAKE opcode"));
    assert!(discord.contains("SET_ACTIVITY must use the FRAME opcode"));
}

#[test]
fn discord_payload_validation_contract_is_ci_enforced() {
    // Regression: Discord rejected a SET_ACTIVITY containing an empty string
    // with `4000: "..." is not allowed to be empty` (verified live), silently
    // dropping the whole status update. The payload validator plus the
    // exhaustive wire-contract test must stay wired so such 4000 rejections
    // surface locally in CI instead of on a live Discord client.
    let discord = read("rivulet-core/src/discord.rs");
    // The reusable pre-wire validator encodes the documented rules.
    assert!(discord.contains("pub enum PayloadIssue"));
    assert!(discord.contains("pub fn validate_set_activity_payload"));
    assert!(discord.contains("FieldTooLong"));
    assert!(discord.contains("InvalidAssetKey"));
    // The serializer must filter implausible asset keys, never send them
    // verbatim (Discord silently drops the image, no error).
    assert!(discord.contains("key.trim().len() <= 64"));
    // The exhaustive contract test serializes every payload variant and
    // asserts the wire output satisfies all three rules.
    assert!(discord.contains("every_payload_variant_conforms_to_discord_rules_on_the_wire"));
    assert!(discord.contains("empty state on the wire"));
    assert!(discord.contains("text.len() <= 128"));
    assert!(discord.contains("implausible large_image on the wire"));
    // CI runs the contract check as a dedicated, named step.
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("Discord payload contract check")
            && ci.contains("cargo test -p rivulet-core --lib payload"),
        "CI must expose a dedicated Discord payload contract check"
    );
    // The Settings UI must warn immediately on Apply: the payload validator
    // runs next to the client-id check and both warnings render with the
    // error palette, in both locales.
    let gui = read("rivulet-gui/src/app.rs");
    assert!(gui.contains("fn apply_discord_payload_validation"));
    assert!(gui.contains("discord_payload_warning"));
    assert!(gui.contains("discord_payload_error_field_too_long"));
    assert!(gui.contains("discord_payload_error_asset_key"));
    let i18n = read("rivulet-core/src/i18n.rs");
    assert!(
        i18n.matches("\"discord_payload_error_field_too_long\"")
            .count()
            >= 2
    );
    assert!(i18n.matches("\"discord_payload_error_asset_key\"").count() >= 2);
    // The docs must describe the rules and the 4000 rejection.
    let docs = read("docs/activity-status.md");
    assert!(
        docs.contains("128 characters") && docs.contains("4000"),
        "activity-status.md must document the payload rules and the 4000 rejection"
    );
}

#[test]
fn discord_reconnect_button_rebuilds_the_adapter() {
    // The Stream view must offer a one-click reconnect when the adapter is
    // not connected, and the flag must force a fresh worker + handshake in
    // the reconcile (no app restart needed).
    let gui = read("rivulet-gui/src/app.rs");
    assert!(gui.contains("discord_reconnect_requested: bool"));
    assert!(
        gui.contains("self.discord_client_id_dirty || self.discord_reconnect_requested"),
        "reconnect must trigger the same rebuild path as a client-id change"
    );
    let draw = gui
        .split_once("fn draw_presence_status")
        .map(|(_, rest)| rest)
        .expect("draw_presence_status must exist");
    assert!(draw.contains("discord_reconnect"));
    assert!(draw.contains("!is_connected"));
    assert!(draw.contains("self.discord_reconnect_requested = true;"));
    // The behavior test must stay wired.
    assert!(gui.contains("discord_presence_reconnect_rebuilds_the_adapter"));
    // The i18n key exists in both locales (parity test enforces agreement).
    let i18n = read("rivulet-core/src/i18n.rs");
    assert!(i18n.matches("\"discord_reconnect\"").count() >= 2);
}

#[test]
fn discord_client_id_is_validated_on_apply() {
    // Settings must validate the client id format on Apply and warn instead of
    // silently accepting a mistyped id (which would keep the adapter off).
    let discord = read("rivulet-core/src/discord.rs");
    assert!(discord.contains("pub fn validate_client_id"));
    assert!(discord.contains("pub enum ClientIdError"));
    assert!(discord.contains("ClientIdError::NotNumeric"));
    assert!(discord.contains("ClientIdError::Length"));
    // The validator must be unit-tested in the core.
    assert!(discord.contains("client_id_validation_accepts_realistic_snowflakes"));
    assert!(discord.contains("client_id_validation_rejects_non_numeric_values"));

    let gui = read("rivulet-gui/src/app.rs");
    assert!(gui.contains("fn apply_discord_client_id"));
    assert!(gui.contains("validate_client_id(self.discord_presence_client_id.trim())"));
    assert!(gui.contains("discord_client_id_warning"));
    // Behavior test must stay wired.
    assert!(gui.contains("discord_client_id_validation_blocks_invalid_apply_and_warns"));
    // Both locales must translate the two error messages.
    let i18n = read("rivulet-core/src/i18n.rs");
    for key in [
        "discord_client_id_error_not_numeric",
        "discord_client_id_error_length",
    ] {
        let k = format!("\"{key}\"");
        assert!(
            i18n.matches(&k).count() >= 2,
            "{key} must exist in EN and DE"
        );
    }
}

#[test]
fn discord_client_id_is_restored_from_eframe_storage() {
    // Bug regression: `save()` wrote the full app (including the Discord
    // application id) under eframe::APP_KEY, but nothing ever read the value
    // back — every launch started from Default and silently dropped ALL
    // persisted settings. The startup path must restore from storage and
    // re-attach the live engine/CLI values.
    let gui = read("rivulet-gui/src/app.rs");
    assert!(
        gui.contains("fn restore_from_storage"),
        "a storage restore helper must exist"
    );
    assert!(
        gui.contains("eframe::get_value::<RivuletApp>(storage?, eframe::APP_KEY)"),
        "restore must read via eframe::get_value under APP_KEY"
    );
    // The constructor must actually wire the restore in (not just define it).
    let new_fn = gui
        .split_once("pub fn new(")
        .map(|(_, rest)| rest)
        .expect("RivuletApp::new must exist");
    assert!(
        new_fn.contains("Self::restore_from_storage(cc.storage)"),
        "new() must call restore_from_storage with cc.storage"
    );
    assert!(
        new_fn.contains("restored.engine = app.engine;"),
        "restored state must keep the live engine"
    );
    assert!(
        new_fn.contains("restored.no_frame_timeout"),
        "restored state must keep the CLI no-frame timeout"
    );
    // The regression test that guarantees the round trip must stay wired.
    assert!(gui.contains("discord_client_id_survives_eframe_storage_round_trip"));
    assert!(gui.contains("impl eframe::Storage for MemoryStorage"));
}

#[test]
fn discord_presence_uses_obs_style_assets_and_composed_title() {
    // Like OBS, the activity card should render the app artwork (large image
    // from the Discord Developer Portal) instead of the generic placeholder
    // icon. The first card line (`details`) is the dynamic composed title
    // "Rivulet · <localized status>" so small hover cards identify Rivulet
    // even when they do not render the registration title; the app word is
    // deliberately NOT duplicated into the game slot (`state`) or the assets.
    let presence = read("rivulet-core/src/presence.rs");
    assert!(
        presence.contains("let state = match game"),
        "the game name must live in state (second card line)"
    );
    assert!(
        presence.contains("let details = format!(\"Rivulet · {label}\")"),
        "details must be the composed title \"Rivulet · <status>\""
    );
    let discord = read("rivulet-core/src/discord.rs");
    assert!(discord.contains("large_image_key: Option<String>"));
    assert!(discord.contains("struct ActivityAssets"));
    assert!(discord.contains("#[serde(rename = \"large_image\")]"));
    // The key is mirrored to small_image (member list) alongside large_image
    // (profile card) so the same uploaded asset replaces the placeholder in
    // both places; Discord renders small_image in the member list.
    assert!(discord.contains("#[serde(rename = \"small_image\")]"));
    assert!(discord.contains("small_image: key,"));
    // Wire-level coverage: assets attached when configured, absent otherwise.
    assert!(discord.contains("set_activity_attaches_large_image_when_configured"));
    // Discord rejects an empty string field (4000: "..." is not allowed to be
    // empty, verified live), so `state` must be omitted when there is no game
    // name instead of being sent empty; `details` always carries the label.
    assert!(
        discord.contains("details: String") && discord.contains("!status.state.trim().is_empty()"),
        "empty state must be omitted, never sent as an empty string"
    );
    assert!(discord.contains("empty_state_is_omitted_not_sent_empty"));
    let gui = read("rivulet-gui/src/app.rs");
    assert!(gui.contains("discord_presence_large_image"));
    assert!(gui.contains("discord_large_image"));
    // The artwork key must survive the eframe persistence round trip.
    assert!(gui.contains("discord_presence_large_image, \"rivulet_logo\""));
}

#[test]
fn discord_presence_errors_are_logged_and_connection_state_is_exposed() {
    // Regression: the presence worker swallowed IPC failures silently, so the
    // GUI showed the desired status while Discord displayed only the plain
    // "Playing Rivulet" game card. The worker must log (crash-log feature)
    // and expose a shared connection state that the Stream view renders.
    let discord = read("rivulet-core/src/discord.rs");
    assert!(discord.contains("pub enum DiscordConnState"));
    assert!(discord.contains("pub fn connection_state"));
    assert!(
        discord.contains("tracing::warn!(")
            && discord.contains("Discord Rich Presence IPC unavailable"),
        "IPC failures must be logged for the crash logs"
    );
    assert!(
        discord.contains("tracing::info!") && discord.contains("SET_ACTIVITY delivered"),
        "successful delivery must be logged at info level (visible with the default RUST_LOG)"
    );
    assert!(
        discord.contains("DiscordConnState::Connected"),
        "worker must flip to Connected"
    );

    let gui = read("rivulet-gui/src/app.rs");
    assert!(
        gui.contains("p.connection_state()"),
        "Stream view must poll the real connection state"
    );
    assert!(gui.contains("discord_conn_off"));
    assert!(gui.contains("discord_conn_connected"));
    // Locales must translate every new key (parity test enforces agreement).
    let i18n = read("rivulet-core/src/i18n.rs");
    for key in [
        "discord_conn_off",
        "discord_conn_connecting",
        "discord_conn_connected",
    ] {
        let k = format!("\"{key}\"");
        assert!(
            i18n.matches(&k).count() >= 2,
            "{key} must exist in EN and DE"
        );
    }
}

#[test]
fn presence_legend_lists_every_state_with_a_tooltip() {
    // The Stream view renders a legend: one row per state with an explanatory
    // tooltip, highlighting the active row. The i18n tables must translate
    // every tooltip key in both locales (parity test enforces key agreement).
    let presence = read("rivulet-core/src/presence.rs");
    assert!(presence.contains("pub const fn all() -> [PresenceActivity; 6]"));
    assert!(presence.contains("pub const fn tooltip_i18n_key"));
    let gui = read("rivulet-gui/src/app.rs");
    let draw = gui
        .split_once("fn draw_presence_status")
        .map(|(_, rest)| rest)
        .expect("draw_presence_status must exist");
    assert!(draw.contains("presence_legend"));
    assert!(draw.contains("for activity in PresenceActivity::all()"));
    assert!(draw.contains("activity.tooltip_i18n_key()"));
    assert!(draw.contains("response.on_hover_text(tip)"));
    let i18n = read("rivulet-core/src/i18n.rs");
    for key in [
        "presence_tooltip_ready",
        "presence_tooltip_recording",
        "presence_tooltip_streaming",
        "presence_tooltip_recording_streaming",
        "presence_tooltip_paused",
        "presence_tooltip_error",
    ] {
        assert!(i18n.matches(key).count() >= 2, "{key} must be localized");
    }
    // The operator-facing docs must document all six states with their labels
    // and transitions so the model cannot drift from the documented contract.
    let docs = read("docs/activity-status.md");
    assert!(
        docs.contains("## State reference"),
        "docs must have the state table"
    );
    for (de, en) in [
        ("Bereit", "Ready"),
        ("Aufnahme", "Recording"),
        ("Streamt", "Streaming"),
        ("Aufnahme + Stream", "Recording + streaming"),
        ("Pausiert", "Paused"),
        ("Fehler", "Error"),
    ] {
        assert!(
            docs.contains(de) && docs.contains(en),
            "docs must document both labels ({de}/{en})"
        );
    }
    // The documented transition language (arrows) signals the table is alive.
    assert!(docs.contains("Transitions out"));
}

#[test]
fn midi_mapping_is_wired_into_gui_and_ci() {
    // The hardware-free mapping/parse core lives in rivulet-core::midi.
    let midi_core = read("rivulet-core/src/midi.rs");
    assert!(midi_core.contains("pub struct MidiMapping"));
    assert!(midi_core.contains("pub struct MidiPresetLibrary"));
    assert!(midi_core.contains("pub fn parse_midi"));
    assert!(midi_core.contains("pub enum MidiAction"));
    // The GUI owns the midir device bridge.
    let gui_cargo = read("rivulet-gui/Cargo.toml");
    assert!(gui_cargo.contains("midir"), "GUI must depend on midir");
    let gui = read("rivulet-gui/src/app.rs");
    assert!(gui.contains("fn reconcile_midi"));
    assert!(gui.contains("fn apply_midi_action"));
    assert!(
        gui.contains("midi_section"),
        "Settings must expose the MIDI section"
    );
    // Learn mode + per-device presets are part of the MIDI configuration UI.
    assert!(gui.contains("midi_learn"), "Learn mode must be wired up");
    assert!(
        gui.contains("midi_presets"),
        "Device presets must be wired up"
    );
    // Linux needs the ALSA dev headers to build midir; CI must install them
    // in both the test and the release package workflow.
    let workflow = read(".github/workflows/ci.yml");
    assert!(
        workflow.contains("libasound2-dev"),
        "CI must install libasound2-dev for the midir ALSA backend"
    );
    let release = read(".github/workflows/build-package.yml");
    assert!(
        release.contains("libasound2-dev"),
        "Release builds must install libasound2-dev for the midir ALSA backend"
    );
    // The i18n catalogs must cover the MIDI section (incl. learn/presets) in
    // both locales.
    let i18n = read("rivulet-core/src/i18n.rs");
    assert!(i18n.matches("midi_section").count() >= 2);
    assert!(i18n.matches("midi_learn").count() >= 2);
    assert!(i18n.matches("midi_presets").count() >= 2);
}

#[test]
fn adaptive_bitrate_live_change_diagnostics_are_wired_up() {
    let runtime = read("rivulet-core/src/stream_runtime.rs");
    let engine = read("rivulet-core/src/lib.rs");
    let docs = read("docs/m3-streaming-quality-gate.md");
    assert!(runtime.contains("pub struct BitrateChange"));
    assert!(runtime.contains("last_change_info"));
    assert!(engine.contains("pub fn last_bitrate_change"));
    assert!(docs.contains("last_bitrate_change()"));
}

#[test]
fn rust_toolchain_is_pinned_for_local_and_ci_parity() {
    // rustfmt output differs between compiler versions (the 0.65.0-alpha.100
    // window failed CI because rustfmt 1.9.0 collapsed a long `contains()`
    // line that the CI runner's older rustfmt wanted wrapped). A repo-root
    // rust-toolchain.toml pins the toolchain for EVERY cargo invocation in
    // any checkout, so local and CI formatting can never diverge again.
    let pinned = read("rust-toolchain.toml");
    assert!(
        pinned.contains("channel = \"1.98.0\""),
        "the toolchain channel must be pinned to an exact version"
    );
    assert!(
        pinned.contains("components = [\"rustfmt\", \"clippy\"]"),
        "rustfmt and clippy must ship with the pinned toolchain"
    );
    // CI installs the same toolchain through the pinned dtolnay action; the
    // rust-toolchain.toml channel keeps `cargo fmt`/`clippy` on that exact
    // version instead of whatever `stable` resolves to that day.
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("components: rustfmt, clippy"),
        "CI must keep installing rustfmt + clippy for the lints job"
    );
}

#[test]
fn wiki_repo_doc_link_audit_is_wired_up() {
    // Wiki pages link to three drifting kinds of targets: other wiki pages,
    // the versioned repo docs (GitHub serves them with HTTP 200 even when
    // the #anchor is gone), and external URLs. The auditor validates all
    // three — interwiki pages + anchors, repo-doc files + GitHub heading
    // slugs, external URL reachability — runs in the scheduled wiki workflow
    // against origin/develop, and is part of the local sync smoke.
    let auditor = read("scripts/audit-wiki-links.py");
    assert!(
        auditor.contains("github_slug") && auditor.contains("heading_anchors"),
        "the auditor must reproduce GitHub's heading-anchor slugs"
    );
    assert!(
        auditor.contains("url_reachable") && auditor.contains("--skip-external"),
        "the auditor must check external URL reachability with an offline skip"
    );
    assert!(
        auditor.contains("in_code_fence"),
        "template/example links inside code fences must be ignored"
    );
    let workflow = read(".github/workflows/wiki-translations.yml");
    assert!(
        workflow.contains("scripts/audit-wiki-links.py")
            && workflow.contains("--develop-from-origin"),
        "the wiki workflow must run the full link audit"
    );
    let smoke = read("scripts/wiki-sync-smoke.sh");
    assert!(
        smoke.contains("audit-wiki-links.py") && smoke.contains("WIKI_LINK_AUDIT_EXTRA"),
        "the local sync smoke must include the full link audit (offline override)"
    );
    // The audit runs in both directions: wiki -> repo docs (forward) and
    // repo docs -> wiki (backwards). Deep wiki links from repo docs must
    // resolve to real pages + heading anchors, and backticked page
    // references must not drift from the canonical page names.
    assert!(
        auditor.contains("--check-repo-docs"),
        "the auditor must support the backwards repo-docs audit mode"
    );
    assert!(
        auditor.contains("WIKI_LINK_RE") && auditor.contains("page_key"),
        "the auditor must validate deep wiki links and fuzzy page-name drift"
    );
    assert!(
        workflow.contains("--check-repo-docs"),
        "the wiki workflow must also audit repo docs for wiki references"
    );
    assert!(
        smoke.contains("--check-repo-docs"),
        "the local sync smoke must include the backwards repo-docs audit"
    );
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
fn release_workflow_publishes_current_source_onto_release_branch() {
    let workflow = read(".github/workflows/release.yml");
    assert!(
        workflow.contains("bash scripts/release-branch.sh \"$RELEASE_BRANCH\""),
        "release workflow must reconcile the release branch via scripts/release-branch.sh"
    );

    let script = read("scripts/release-branch.sh");
    assert!(
        script.contains("git fetch origin \"$BRANCH\"")
            && script.contains("git show-ref --verify --quiet \"refs/remotes/origin/$BRANCH\"")
            && script.contains("git merge-base --is-ancestor \"$REMOTE_TIP\" HEAD")
            && script.contains("DEST=\"refs/heads/$BRANCH\"")
            && script.contains("git push origin \"HEAD:$DEST\"")
            && script.contains("git push --force-with-lease origin \"HEAD:$DEST\""),
        "release-branch.sh must publish current HEAD onto the release branch, \
         fast-forwarding when the remote is at/behind HEAD and force-with-lease \
         overwriting a divergent/stale tip so a release always builds current source"
    );
    assert!(
        !script.contains("git push origin \"HEAD:$BRANCH\""),
        "every push must use the fully-qualified refs/heads/ destination: newer git \
         versions reject an unqualified destination when pushing HEAD (\"The destination \
         you provided is not a full refname\")"
    );
}

#[test]
fn release_gate_includes_build_and_chore_commits() {
    let workflow = read(".github/workflows/release.yml");
    assert!(
        workflow.contains("'^(feat|fix|build|chore|ci)(\\(.*\\))?!?:'"),
        "release.yml must treat build/chore/ci commits as releasable so packaging, \
         housekeeping and CI changes ship without a manual workflow dispatch"
    );

    let script = read("scripts/release-version.sh");
    // Bound the needle so the assert line stays under rustfmt's max_width in
    // every toolchain version (CI rustfmt and local rustfmt must agree).
    let build_rule =
        "build:*|build\\(*\\):*|chore:*|chore\\(*\\):*|ci:*|ci\\(*\\):*) HAS_BUILD=true";
    assert!(
        script.contains(build_rule),
        "release-version.sh must classify build/chore/ci commits as releasable"
    );
    assert!(
        script.contains("HAS_BUILD\" == true"),
        "release-version.sh must bump the patch version for build/chore-only ranges"
    );
    assert!(
        script.contains("No feat/fix/build/chore/ci commits"),
        "release-version.sh must keep the version when only non-releasable \
         commits (docs/test/style) exist, matching the workflow gate"
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
                "needs: [lints, beta_gate, build_and_test, pinning_tests, srt_receiver_smoke, rist_receiver_smoke, obs_websocket_smoke, fuzz_smoke, roadmap_sync]"
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
fn roadmap_sync_check_keeps_docs_and_milestones_aligned() {
    // The milestone sequence is owned by GitHub (canonical `M<n> – Title`
    // names). The checker must (1) compare the README overview table, the
    // gates-doc resource rows/sections, and the live GitHub milestones, and
    // (2) be wired into CI as its own stable check with issues read access.
    let checker = read("scripts/check-roadmap-sync.py");
    for marker in [
        "def parse_readme_table",
        "def parse_gates",
        "def local_checks",
        "def github_checks",
        "def fetch_milestones",
        "milestones?state=all&per_page=100",
        "--self-test",
        "--fixture",
    ] {
        assert!(checker.contains(marker), "checker must expose {marker}");
    }
    // The checker must know the structural contracts it validates: README
    // rows carry a badge id that must point at the GitHub milestone with the
    // same title, and gates sections are required from M2 onward (M0/M1
    // predate the gates document).
    assert!(checker.contains("FIRST_GATE_SECTION = 2"));
    assert!(checker.contains("no GitHub milestone is titled"));
    assert!(checker.contains("badge points at"));
    assert!(checker.contains("missing a `### M<n>:` quality-gate section"));

    let readme = read("README.md");
    // Overview table rows must use the full canonical milestone names so the
    // live GitHub title comparison cannot silently pass on shortened labels.
    assert!(readme.contains("M4 – Advanced Output & Capture"));
    assert!(readme.contains("M8 – Embeddable Engine & API"));
    assert!(readme.contains("M11 – Extensible UI & Plugin Platform"));
    let gates = read("docs/milestone-quality-gates.md");
    assert!(gates.contains("### M11: Extensible UI and Plugin Platform"));

    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("name: Roadmap-Sync Check")
            && ci.contains("scripts/check-roadmap-sync.py --self-test")
            && ci.contains("python3 scripts/check-roadmap-sync.py")
            && ci.contains("issues: read")
            && ci.contains("needs.roadmap_sync.result"),
        "CI must run the roadmap-sync checker with issues read access and \
         require it in the aggregate"
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
        "rivulet-obs-websocket/Cargo.toml",
        "rivulet-opengl-hook-dll/Cargo.toml",
        "rivulet-plugins/Cargo.toml",
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
fn updater_verifies_release_checksums_before_install() {
    // The release workflow must attach a SHA256SUMS manifest covering the
    // installer assets, and the updater must verify a downloaded installer
    // against it BEFORE the install path can be reached. This closes the
    // gap where a tampered release asset would be launched unchecked.
    let updater = read("rivulet-updater/src/lib.rs");
    assert!(
        updater.contains("pub fn verify_downloaded_asset"),
        "the updater must expose the manifest-based verification entry point"
    );
    assert!(
        updater.contains("pub fn verify_checksum"),
        "the updater must verify digests fail-closed (malformed digest = reject)"
    );
    assert!(
        updater.contains("SHA256SUMS"),
        "the manifest asset name is part of the public contract"
    );
    assert!(
        updater.contains("checksum mismatch"),
        "digest mismatches must produce an actionable error"
    );

    let workflow = read(".github/workflows/release.yml");
    assert!(
        workflow.contains("Generate SHA256SUMS manifest"),
        "the release workflow must generate the checksum manifest"
    );
    assert!(
        workflow.contains("sha256sum) > SHA256SUMS.tmp"),
        "the manifest must cover every attached asset (relative paths, sorted) \
         and be written outside the scanned directory (shellcheck SC2094)"
    );
    assert!(
        workflow.contains("release-assets/SHA256SUMS"),
        "the generated manifest must be attached to the release"
    );

    let gui = read("rivulet-gui/src/app.rs");
    assert!(
        gui.contains("verify_downloaded_asset"),
        "the GUI download flow must verify the installer before Downloaded"
    );
}

#[test]
fn fuzz_targets_cover_the_untrusted_input_parsers() {
    // Every parser that consumes remote-controlled bytes must have a fuzz
    // target, and CI must run a libFuzzer smoke over all of them so parser
    // regressions (panics on crafted input) fail the build.
    for (target, _crate_under_test, symbol) in [
        ("parse_irc_line.rs", "rivulet-core", "parse_irc_line"),
        (
            "sdp_offer_endpoint.rs",
            "rivulet-core",
            "SdpOffer::h264_opus",
        ),
        (
            "parse_latest_release.rs",
            "rivulet-updater",
            "parse_latest_release",
        ),
        ("parse_checksums.rs", "rivulet-updater", "parse_checksums"),
    ] {
        let path = format!("fuzz/fuzz_targets/{target}");
        let source = read(&path);
        assert!(
            source.contains("no_main") && source.contains("fuzz_target!"),
            "{path} must be a real libFuzzer target"
        );
        assert!(source.contains(symbol), "{path} must exercise {symbol}");
    }
    assert!(
        read("fuzz/Cargo.toml").contains("cargo-fuzz = true"),
        "fuzz/Cargo.toml must be a cargo-fuzz crate"
    );
    let workspace = read("Cargo.toml");
    assert!(
        workspace.contains("exclude = [\"fuzz\"]"),
        "the fuzz crate must stay excluded from the normal workspace build"
    );

    let smoke = read("scripts/fuzz-smoke.sh");
    for target in [
        "parse_irc_line",
        "sdp_offer_endpoint",
        "parse_latest_release",
        "parse_checksums",
    ] {
        assert!(smoke.contains(target), "smoke must run {target}");
    }

    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("Fuzz smoke (regression corpus)")
            && ci.contains("scripts/fuzz-smoke.sh")
            && ci.contains("fuzz-crashes"),
        "CI must run the fuzz smoke and upload crash artifacts"
    );
    assert!(
        ci.contains("libglib2.0-dev") && ci.contains("libgstreamer1.0-dev"),
        "the fuzz job must install the glib/gstreamer pkg-config files rivulet-core's -sys crates need"
    );
    assert!(
        !ci.contains("# nightly"),
        "toolchain selection must use the action input, not a ref comment the pin generator misreads"
    );
}

#[test]
fn deep_fuzz_campaign_is_scheduled_with_corpus_persistence() {
    // The weekly deep campaign is the complement to the push-time smoke: it
    // must exist, give each target a 10-minute budget, persist the corpus
    // through the actions cache so coverage accumulates, and stay
    // triggerable on demand. Without it, the smoke-only gate is the only
    // fuzzing the project ever does.
    let deep = read(".github/workflows/fuzz-deep.yml");
    assert!(
        deep.contains("schedule:") && deep.contains("0 2 * * 1"),
        "the deep fuzz workflow must run on a weekly schedule"
    );
    assert!(
        deep.contains("workflow_dispatch:"),
        "the deep fuzz workflow must be triggerable manually"
    );
    assert!(
        deep.contains("FUZZ_MAX_TOTAL_TIME") && deep.contains("\"600\""),
        "the deep campaign must budget 10 minutes (600s) per target"
    );
    assert!(
        deep.contains("actions/cache@")
            && deep.contains("fuzz/corpus/")
            && deep.contains("restore-keys")
            && deep.contains("fuzz-corpus-"),
        "the deep fuzz workflow must persist the corpus via the actions cache"
    );
    let smoke = read("scripts/fuzz-smoke.sh");
    assert!(
        smoke.contains("FUZZ_MAX_TOTAL_TIME") && smoke.contains("-max_total_time=$MAX_TIME"),
        "fuzz-smoke.sh must support the deep time-budgeted campaign mode"
    );
}

#[test]
fn shared_memory_frames_are_validated_before_read() {
    // Shared memory is writable by every process in the session, and the
    // writer runs inside the captured game — an untrusted host. The readers
    // must therefore validate the frame header beyond the magic: geometry,
    // pixel format, data/geometry agreement, an allocation cap and the true
    // mapping size (never a compile-time constant).
    let channel = read("rivulet-core/src/capture_channel.rs");
    assert!(
        channel.contains("pub fn is_plausible"),
        "FrameHeader must expose the plausibility check"
    );
    assert!(
        channel.contains("MAX_PIXEL_BYTES"),
        "pixel payload must be capped before allocation"
    );
    assert!(
        channel.contains("checked_mul"),
        "geometry math must not overflow silently"
    );
    assert!(
        channel.contains("mapped_region_size") && channel.contains("VirtualQuery"),
        "the Windows reader must query the real mapping size"
    );
    assert!(
        channel.contains("libc::fstat"),
        "the Linux reader must bound the mapping by the object size"
    );
    assert!(
        channel.contains("UnmapViewOfFile"),
        "the Windows mapping must be released on drop"
    );
    assert!(
        !channel.contains("data_offset + data_len > self.size"),
        "the old constant-size bounds check must not return"
    );

    // Both readers go through the check.
    let opengl = read("rivulet-core/src/opengl_hook.rs");
    assert!(
        opengl.contains("is_plausible"),
        "the OpenGL hook reader must validate headers too"
    );

    // The writers must not emit headers whose data_size disagrees with the
    // geometry (checked multiplication instead of release-mode wrapping).
    let layer = read("rivulet-vulkan-layer/src/capture_channel.rs");
    assert!(
        layer.contains("checked_mul") && layer.contains("u32::try_from"),
        "the Vulkan layer writer must compute data_size with checked math"
    );
    let dll = read("rivulet-opengl-hook-dll/src/lib.rs");
    assert!(
        dll.contains("checked_mul"),
        "the OpenGL hook writer must refuse geometry/data disagreement"
    );
}

#[test]
fn alpha_release_runs_are_serialized() {
    // Two pushes in quick succession used to race for the same next version
    // number, release branch and GitHub release object (tag-push rejections,
    // asset-upload Not Found). The workflow must serialize itself.
    let workflow = read(".github/workflows/release.yml");
    assert!(
        workflow.contains("concurrency:")
            && workflow.contains("group: release-alpha")
            && workflow.contains("cancel-in-progress: false"),
        "release.yml must serialize runs via a concurrency group"
    );
}

#[test]
fn alpha_release_gates_on_ci_conclusion_and_auto_resumes() {
    // The alpha release must be triggered by the CI workflow completing, not
    // by the push itself: a flaky/failed CI run must leave the release
    // skipped (not cancelled), and rerunning CI green must fire this
    // workflow again automatically — no manual release rerun after flakes.
    let workflow = read(".github/workflows/release.yml");
    assert!(
        workflow.contains("workflow_run:")
            && workflow.contains("workflows: [CI]")
            && workflow.contains("types: [completed]")
            && workflow.contains("branches: [develop]"),
        "release.yml must trigger on CI completion on develop (workflow_run)"
    );
    assert!(
        workflow.contains("CONCLUSION")
            && workflow.contains("!= \"success\"")
            && workflow.contains("rerun CI green to auto-resume"),
        "release.yml must gate on the CI conclusion (success only) and \
         document the rerun-green auto-resume path"
    );
    assert!(
        workflow.contains("github.event.workflow_run.head_sha"),
        "release.yml must checkout the CI-tested commit (workflow_run.head_sha), \
         not the default-branch tip, so a release always builds the tested SHA"
    );
    assert!(
        !workflow.contains("on:\n  push:\n    branches: [ develop ]"),
        "release.yml must no longer run on push directly — it must wait for CI"
    );
}

#[test]
fn alpha_release_notes_are_generated_from_commits_since_last_tag() {
    // The GitHub release body must be derived from the ACTUAL commits since
    // the previous tag (grouped by conventional-commit type) rather than
    // GitHub's PR-based auto-notes, which are sparse for a workflow that
    // pushes commits directly to develop.
    let release = read(".github/workflows/release.yml");
    assert!(
        release.contains("Generate release notes from commits since last tag")
            && release.contains("scripts/generate-release-notes.sh")
            && release.contains("body_path: release-notes.md"),
        "the release workflow must generate its notes body from the commits since the last tag"
    );
    assert!(
        !release.contains("generate_release_notes: true"),
        "GitHub's PR-based auto-notes must be replaced by the commit-derived body"
    );
    assert!(
        release.contains("fetch-depth: 0"),
        "the release checkout must fetch full history + tags for the notes generator"
    );

    let notes = read("scripts/generate-release-notes.sh");
    assert!(
        notes.contains("--self-test")
            && notes.contains("describe --tags --abbrev=0")
            && notes.contains("chore(release): prepare "),
        "the notes generator must ship a self-test and exclude release-prep commits"
    );
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("generate-release-notes.sh --self-test"),
        "the notes generator self-test must run in CI"
    );
}

#[test]
fn obs_upstream_candidates_doc_keeps_both_generation_markers() {
    // The OBS upstream workflow rewrites the candidates doc between its
    // START/END markers on every run. An earlier version dropped the END
    // marker when writing, so the next run failed with "candidate markers
    // missing" and — masked by continue-on-error — silently emptied the
    // weekly report artifact and step summary. The doc must carry both
    // markers and the writer must write the END marker back.
    let doc = read("docs/obs-vision-candidates.md");
    assert!(
        doc.contains("<!-- OBS-VISION-CANDIDATES:START -->")
            && doc.contains("<!-- OBS-VISION-CANDIDATES:END -->"),
        "the candidates doc must keep both generation markers"
    );
    let script = read("scripts/check-obs-upstream.py");
    assert!(
        script.contains("+ end + text.split(end, 1)[1]"),
        "update_candidate_doc must write the END marker back"
    );
    let workflow = read(".github/workflows/obs-upstream.yml");
    assert!(
        workflow.contains("--update-doc") && workflow.contains("obs-upstream-report"),
        "the weekly workflow must generate the candidates doc and publish the report artifact"
    );
    assert!(
        !workflow.contains("continue-on-error"),
        "the OBS workflow must not mask check failures with continue-on-error"
    );
    assert!(
        workflow.contains("report is empty or malformed")
            && workflow.contains("## OBS upstream check"),
        "the OBS workflow must fail loudly when the report is empty or malformed"
    );
}

#[test]
fn obs_upstream_check_persists_checked_release_tag_across_runs() {
    // Delta tracking needs the last-checked release to survive between
    // weekly runs. The checker records it in a gitignored state file
    // (scripts/.obs-upstream-state.json) that the workflow restores from
    // and saves back to the actions cache — never a repo commit, so the
    // workflow keeps its contents:read-only permission.
    let workflow = read(".github/workflows/obs-upstream.yml");
    assert!(
        workflow.contains("actions/cache@")
            && workflow.contains("scripts/.obs-upstream-state.json")
            && workflow.contains("obs-upstream-state-${{ github.run_id }}")
            && workflow.contains("restore-keys")
            && workflow.contains("obs-upstream-state-"),
        "the OBS workflow must persist the checked-release state via the actions cache"
    );
    let script = read("scripts/check-obs-upstream.py");
    assert!(
        script.contains("def persist_state(")
            && script.contains("def previous_tag_from(")
            && script.contains("persist_state(release)"),
        "the checker must write the checked tag and read it back on the next run"
    );
    let gitignore = read(".gitignore");
    assert!(
        gitignore.contains("/scripts/.obs-upstream-state.json"),
        "the state file must stay a gitignored runtime artifact"
    );
}

#[test]
fn obs_upstream_weekly_check_also_reviews_rivulet_open_issues() {
    // The weekly sweep covers Rivulet's own open issues with the same vision
    // scoring and publishes them in the same report, so maintainers review
    // OBS candidates and community wishes in one place.
    let workflow = read(".github/workflows/obs-upstream.yml");
    assert!(
        workflow.contains("issues: write") && workflow.contains("sweep Rivulet issues"),
        "the workflow must request issue write access and run the issue sweep"
    );
    assert!(
        workflow.contains("## Rivulet open issues")
            && workflow.contains("community-wish-candidates.md"),
        "the common report must carry the issue section and the community doc artifact"
    );
    assert!(
        workflow.contains("weekly-vision-review")
            && workflow.contains("gh issue create")
            && workflow.contains("--checklist-file")
            && workflow.contains("## Review checklist"),
        "the weekly run must publish the merged report as a labeled issue with a checklist"
    );
    let script = read("scripts/check-obs-upstream.py");
    assert!(
        script.contains("def fetch_open_issues(")
            && script.contains("def analyze_issues(")
            && script.contains("def issues_report(")
            && script.contains("def update_community_doc(")
            && script.contains("def review_checklist(")
            && script.contains("COMMUNITY-WISH-CANDIDATES:START")
            && script.contains("vision_fit(text, vision)"),
        "the checker must fetch, score, report, checklist, and persist Rivulet issue wishes"
    );
    assert!(
        script.contains("ROADMAP_LABELS")
            && script.contains("def is_roadmap_tracked(")
            && script.contains("### Roadmap-tracked"),
        "enhancement/epic milestone work must be excluded from the wish review"
    );
    let doc = read("docs/community-wish-candidates.md");
    assert!(
        doc.contains("<!-- COMMUNITY-WISH-CANDIDATES:START -->")
            && doc.contains("<!-- COMMUNITY-WISH-CANDIDATES:END -->")
            && doc.contains("Review policy")
            && doc.contains("Label taxonomy"),
        "the community doc must keep both generation markers and the review/label policy"
    );
}

#[test]
fn ndi_output_is_wired_into_the_engine_and_gui() {
    // M5 #77: the NDI contract must actually publish — the engine embeds the
    // `ndisink` feed into every session pipeline and the GUI exposes the
    // destination in Settings, applied before each session start.
    let core = read("rivulet-core/src/lib.rs");
    assert!(
        core.contains("ndi_output: Option<NdiOutput>")
            && core.contains("pub fn set_ndi_output(")
            && core.contains("fn ndi_feed_chain(")
            && core.contains("fn video_tail_fragment("),
        "the engine must own the NDI configuration and its pipeline branches"
    );
    assert!(
        core.contains("ndi_vtee") || core.contains("video_tee. ! queue ! h264parse ! ndisink"),
        "recording/streaming pipelines must fan the encoded video into an NDI sink"
    );
    let gui = read("rivulet-gui/src/app.rs");
    let production = gui
        .split_once("#[cfg(test)]")
        .map(|(head, _)| head)
        .unwrap_or(&gui);
    assert!(
        production.contains("fn apply_ndi_output")
            && production.contains("ndi_output_enabled")
            && production.contains("self.tr(\"ndi_section\")"),
        "the GUI must expose the NDI destination in Settings via apply_ndi_output"
    );
    let i18n = read("rivulet-core/src/i18n.rs");
    assert!(
        i18n.matches("\"ndi_section\"").count() == 2
            && i18n.matches("\"ndi_plugin_missing\"").count() == 2,
        "NDI labels must exist in both locales (EN + DE)"
    );
}

#[test]
fn tag_based_release_attaches_checksums_and_generated_notes() {
    // The beta/rc/stable tag path must ship the same release hygiene as the
    // alpha channel: a SHA256SUMS manifest over the attached assets (the
    // updater fails closed on releases without one) and the commit-derived
    // notes body instead of GitHub's PR-based auto-notes, verified for
    // completeness before the release is created.
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("Generate SHA256SUMS manifest")
            && ci.contains("sha256sum) > SHA256SUMS.tmp")
            && ci.contains("release-assets/SHA256SUMS"),
        "the tag-based release path must generate and attach SHA256SUMS"
    );
    assert!(
        ci.contains("Generate release notes from commits since last tag")
            && ci.contains("scripts/generate-release-notes.sh")
            && ci.contains("body_path: release-notes.md"),
        "the tag-based release path must use the commit-derived notes generator"
    );
    assert!(
        ci.contains("check-release-notes.py --notes-file release-notes.md"),
        "the tag-based release path must verify notes completeness before publishing"
    );
    assert!(
        !ci.contains("generate_release_notes: true"),
        "GitHub's PR-based auto-notes must be gone from ci.yml"
    );
    assert!(
        ci.contains("fetch-depth: 0"),
        "the tag-based release checkout must fetch full history + tags for the notes generator"
    );
}

#[test]
fn release_notes_completeness_is_checked_in_ci() {
    // The generated notes must cover every non-prepare commit since the
    // previous tag and contain no release-prep commits. The checker runs in
    // two places: as a regression self-test in the Lints job (every push)
    // and — decisively — in the release workflow against the exact body that
    // is about to be published, before the release is created.
    let checker = read("scripts/check-release-notes.py");
    assert!(
        checker.contains("--self-test")
            && checker.contains("--notes-file")
            && checker.contains("chore(release): prepare "),
        "the completeness checker must ship a self-test and detect prepare-commit leaks"
    );
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("check-release-notes.py --self-test"),
        "the completeness checker self-test must run in the Lints job"
    );
    let release = read(".github/workflows/release.yml");
    assert!(
        release.contains("Verify release notes completeness")
            && release.contains("check-release-notes.py --notes-file release-notes.md"),
        "the release workflow must verify the published notes body for completeness"
    );
}

#[test]
fn updater_declares_sha2_dependency() {
    // sha2 is the only crypto the updater needs; pin it through the workspace
    // so cargo-deny and dependabot track a single version.
    let manifest = read("rivulet-updater/Cargo.toml");
    assert!(
        manifest.contains("sha2 = { workspace = true }"),
        "the updater must hash with the workspace sha2 crate"
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

#[test]
fn daily_logging_defaults_to_info_not_empty_filter() {
    // Regression: logging init used EnvFilter::from_default_env(), which with
    // RUST_LOG unset filters out everything — the daily crash log stayed empty
    // and Discord/engine diagnostics were invisible. The init must resolve a
    // user-friendly default (info) and the fallback must be unit-tested.
    let logging = read("rivulet-gui/src/logging.rs");
    // Only the *production* init code must not use from_default_env; a doc
    // comment mentioning the old bug is fine and expected.
    let production = logging
        .split_once("pub fn init(")
        .map(|(_, rest)| {
            rest.split_once("#[cfg(test)]")
                .map(|(head, _)| head)
                .unwrap_or(rest)
        })
        .expect("init() must exist");
    assert!(
        !production.contains("EnvFilter::from_default_env()"),
        "unset RUST_LOG must not silently disable all logging"
    );
    assert!(
        production.contains("resolve_filter_spec(std::env::var(\"RUST_LOG\").ok())"),
        "init must resolve the filter from the env with an info fallback"
    );
    assert!(
        logging.contains("filter_spec_defaults_to_info_when_rust_log_unset"),
        "the fallback rule must be covered by a unit test"
    );
}

#[test]
fn cancelled_file_dialog_is_logged_at_info_level() {
    // Regression: cancelling the recording save dialog was logged at debug
    // level, which the daily log (default filter `info`) never showed. A user
    // who pressed Record but cancelled the dialog saw nothing in the log and
    // concluded the recording silently failed — while the real reason was a
    // cancelled dialog. The cancellation must be visible in the daily log.
    let app = read("rivulet-gui/src/app.rs");
    let occurrences = app
        .matches("tracing::info!(\"File selection cancelled\")")
        .count();
    assert!(
        occurrences >= 3,
        "every recording save-dialog path must log the cancellation at info \
         level (found {occurrences}, expected >= 3)"
    );
    assert!(
        !app.contains("tracing::debug!(\"File selection cancelled\")"),
        "no recording save-dialog path may keep the debug-level cancellation log"
    );
}

#[test]
fn twitch_chat_dock_is_wired_and_covered() {
    // M5 community dock: the chat lives inside the Stream workspace (one
    // Meld-style broadcast page with stream start/stop, chat, stream status
    // and compact audio), no longer as its own sidebar entry. The core worker
    // must keep its local-listener smoke test (deterministic in CI without
    // real network), and the feature must be documented.
    let app = read("rivulet-gui/src/app.rs");
    assert!(
        !app.contains("AppView::Chat") && !app.contains("nav_chat"),
        "the chat must be part of the Stream workspace, not a sidebar view"
    );
    assert!(
        app.contains("fn draw_chat_dock") && app.contains("fn reconcile_chat"),
        "the chat dock must have a draw + reconcile path"
    );
    assert!(
        app.contains("self.draw_chat_dock(&mut cols[0], chat_list_height)"),
        "the Stream workspace must embed the chat dock next to the stream info"
    );
    let core = read("rivulet-core/src/twitch_chat.rs");
    assert!(
        core.contains("worker_connects_and_delivers_messages_to_local_listener"),
        "the Twitch worker must keep its local-listener smoke test"
    );
    let i18n = read("rivulet-core/src/i18n.rs");
    assert!(
        i18n.contains("(\"chat_title\", \"Chat dock\")")
            && i18n.contains("(\"chat_title\", \"Chat-Dock\")"),
        "the chat view must be localized in DE and EN"
    );
    let docs = read("docs/twitch-chat.md");
    assert!(
        docs.contains("# Chat Dock") && docs.contains("parse_irc_line"),
        "the chat dock must be documented with its architecture"
    );
    // Sending replies: the worker must expose a non-blocking send that is
    // covered by the local-listener smoke, and the GUI must gate the input on
    // an authenticated connection (OAuth token) because Twitch rejects
    // PRIVMSG from the anonymous nick.
    let core = read("rivulet-core/src/twitch_chat.rs");
    assert!(
        core.contains("pub fn send_message") && core.contains("Msg::SendMessage"),
        "the worker must expose a non-blocking send_message"
    );
    assert!(
        core.contains("PRIVMSG {channel} :{text}"),
        "sending must write a PRIVMSG to the joined channel"
    );
    let app = read("rivulet-gui/src/app.rs");
    assert!(
        app.contains("fn send_chat_message") && app.contains("ChatAction::Send"),
        "the GUI must wire a send path into the chat worker"
    );
    assert!(
        app.contains("chat_send_locked"),
        "the UI must show a lock hint when sending is unavailable"
    );
}

#[test]
fn chat_dock_supports_kick_and_youtube() {
    // M5 community dock: besides Twitch IRC the chat dock can connect to
    // Kick (Pusher WebSocket) and YouTube (Innertube polling). Each platform
    // worker must keep a deterministic local-listener smoke test, the GUI
    // must offer a platform selector and gate sending correctly (YouTube is
    // read-only), and the feature must be localized and documented.
    let core = read("rivulet-core/src/lib.rs");
    assert!(
        core.contains("pub mod kick_chat") && core.contains("pub mod youtube_chat"),
        "the chat modules must be exported from core"
    );
    let kick = read("rivulet-core/src/kick_chat.rs");
    assert!(
        kick.contains("pub fn parse_kick_event")
            && kick.contains("pub fn kick_chatroom_id")
            && kick.contains("worker_connects_and_delivers_messages_to_local_ws_server"),
        "the Kick worker must have a pure parser, chatroom resolution and a local WS smoke test"
    );
    let youtube = read("rivulet-core/src/youtube_chat.rs");
    assert!(
        youtube.contains("pub fn parse_youtube_payload")
            && youtube.contains("pub fn youtube_initial_continuation")
            && youtube.contains("worker_connects_and_delivers_messages_to_local_http_listener"),
        "the YouTube worker must have pure parsers and a local HTTP smoke test"
    );
    let chat = read("rivulet-core/src/chat.rs");
    assert!(
        chat.contains("pub enum ChatPlatform")
            && chat.contains("pub struct ChatConfig")
            && chat.contains("pub fn can_send"),
        "the chat facade must expose a platform enum, config and send capability"
    );
    let app = read("rivulet-gui/src/app.rs");
    assert!(
        app.contains("chat_platform") && app.contains("ChatPlatform::all()"),
        "the GUI must offer a chat platform selector"
    );
    assert!(
        app.contains("rivulet_core::Chat::new(&cfg)")
            && app.contains("rivulet_core::ChatConfig::new"),
        "the GUI reconcile must build the platform dispatch config"
    );
    assert!(
        app.contains("chat_read_only") && app.contains("ChatPlatform::YouTube"),
        "YouTube chat must be marked read-only in the GUI"
    );
    let i18n = read("rivulet-core/src/i18n.rs");
    assert!(
        i18n.contains("(\"chat_note_kick\", ")
            && i18n.contains("(\"chat_note_youtube\", ")
            && i18n.contains("(\"chat_channel_hint_kick\", ")
            && i18n.contains("(\"chat_channel_hint_youtube\", "),
        "Kick/YouTube chat keys must exist in both locales"
    );
    let docs = read("docs/twitch-chat.md");
    assert!(
        docs.contains("Kick") && docs.contains("YouTube"),
        "the chat dock documentation must cover Kick and YouTube"
    );
}

#[test]
fn chat_outbound_is_rate_limited_per_platform() {
    // The bot must never burst against a platform limit: every outbound chat
    // send passes through a shared token-bucket limiter in core. Twitch's
    // documented global ceiling is 20 messages / 30 s for non-privileged
    // accounts; Kick has no published limits (undocumented API) so its default
    // throttles harder; YouTube's official insert costs ~200 quota units, so
    // its default is quota-bounded (1/day burst). All defaults are
    // configurable per platform.
    let limiter = read("rivulet-core/src/rate_limit.rs");
    for marker in [
        "pub struct RateLimitConfig",
        "pub struct RateLimiter",
        "pub fn try_acquire",
        "pub fn with_clock",
        "pub const fn twitch_default",
        "pub const fn kick_default",
        "pub const fn youtube_default",
        "burst_is_limited_to_capacity",
        "tokens_refill_over_the_window",
    ] {
        assert!(
            limiter.contains(marker),
            "rate_limit.rs must provide {marker}"
        );
    }
    // The defaults must encode the documented/conservative ceilings: Twitch
    // 20/30s, Kick lower than Twitch, YouTube capacity 1 (serialized sends).
    assert!(limiter.contains("capacity: 20") && limiter.contains("window_secs: 30"));
    assert!(limiter.contains("capacity: 10") && limiter.contains("window_secs: 30"));
    assert!(limiter.contains("capacity: 1") && limiter.contains("window_secs: 86_400"));
    // The chat facade must apply the limiter before enqueueing and expose the
    // remaining budget for the status line.
    let chat = read("rivulet-core/src/chat.rs");
    assert!(
        chat.contains("limiter: Mutex<RateLimiter>")
            && chat.contains("pub fn send_message")
            && chat.contains("pub fn rate_limit_config")
            && chat.contains("pub fn rate_limit_remaining")
            && chat.contains("platform rate limit exhausted"),
        "the chat facade must gate sends through the limiter and expose the budget"
    );
    assert!(
        chat.contains("ChatPlatform::Twitch => RateLimitConfig::twitch_default()")
            && chat.contains("ChatPlatform::Kick => RateLimitConfig::kick_default()")
            && chat.contains("ChatPlatform::YouTube => RateLimitConfig::youtube_default()"),
        "the facade must pick the platform default when no explicit limit is set"
    );
    assert!(
        chat.contains("send_message_drops_when_custom_rate_limit_is_exhausted"),
        "the facade-level rate-limit rejection must be covered by a test"
    );
    // The dock must surface the live budget above the input so throttling is
    // visible before a send is silently dropped (budget line + pause notice).
    let gui = read("rivulet-gui/src/app.rs");
    assert!(
        gui.contains("fn chat_rate_budget")
            && gui.contains("rate_limit_remaining()")
            && gui.contains("chat_rate_budget")
            && gui.contains("chat_rate_limited"),
        "the chat dock must read and render the send budget"
    );
    // The budget line carries a tooltip with the platform + window the limit
    // applies over (e.g. “20 messages per 30 s on Twitch”), so the bare
    // numbers stay compact but nothing is hidden.
    assert!(
        gui.contains("fn chat_rate_limit_detail")
            && gui.contains("chat_rate_window")
            && gui.contains("on_hover_text(tooltip)"),
        "the budget tooltip must expose platform and window"
    );
    assert!(
        gui.contains("chat_rate_budget_reports_limiter_state")
            && gui.contains("chat_rate_limit_detail_reports_platform_and_window"),
        "the budget accessors must be covered by GUI behavior tests"
    );
    let i18n = read("rivulet-core/src/i18n.rs");
    assert!(
        i18n.contains("chat_rate_budget")
            && i18n.contains("chat_rate_limited")
            && i18n.contains("chat_rate_window"),
        "the budget strings must be translated"
    );
    // The platform compliance contract (M10 issue #100) stays documented.
    let readme = read("README.md");
    assert!(readme.contains("20 messages/30 s"));
    let gates = read("docs/milestone-quality-gates.md");
    assert!(gates.contains("20-messages/30-s"));
}

#[test]
fn local_pre_push_hook_mirrors_the_ci_lints_job() {
    // Commit 06792c6 shipped four GUI tests that were clean under a plain
    // `cargo clippy` but failed CI's `-- -D warnings` Lints job
    // (clippy::field_reassign_with_default). The committed `.githooks/pre-
    // push` hook must keep mirroring the CI Lints job (fmt + workspace
    // clippy with -D warnings) and stay documented in CONTRIBUTING so the
    // failure happens before the push, not after.
    let hook = read(".githooks/pre-push");
    for marker in [
        "core.hooksPath .githooks",
        "cargo fmt --all --check",
        "clippy --workspace --all-targets -- -D warnings",
        "RIVULET_SKIP_PRE_PUSH",
        "RIVULET_PRE_PUSH_FAST_TESTS",
        "cargo test -p rivulet-core --test ci_pinning",
        "generate-action-pins.py --check",
        "check-parity-checklist.py --self-test",
        "check-release-notes.py --self-test",
        "generate-release-notes.sh --self-test",
        "check-theme-contrast.py",
        "06792c6",
    ] {
        assert!(
            hook.contains(marker),
            ".githooks/pre-push must mention {marker}"
        );
    }
    let contributing = read("CONTRIBUTING.md");
    assert!(
        contributing.contains("Local pre-push checks")
            && contributing.contains("git config core.hooksPath .githooks")
            && contributing.contains("-D warnings")
            && contributing.contains("RIVULET_PRE_PUSH_FAST_TESTS=0 git push")
            && contributing.contains("RIVULET_SKIP_PRE_PUSH=1 git push"),
        "CONTRIBUTING.md must document the pre-push hook, its -D warnings flags and the toggles"
    );
}

/// Bash to run the hook syntax check with. On Unix `bash` on PATH is fine
/// (CI ubuntu enforces the check there). On Windows, `bash` on PATH may
/// resolve to the WSL launcher (`C:\Windows\System32\bash.exe`), which
/// cannot read Windows-style paths and takes seconds to boot — so prefer an
/// explicit Git for Windows bash and skip locally when none is installed
/// (the CI Pinning-Tests job on ubuntu keeps the check enforced).
fn bash_program() -> Option<std::ffi::OsString> {
    if !cfg!(windows) {
        return Some("bash".into());
    }
    const GIT_BASH: &[&str] = &[
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
        r"C:\Program Files (x86)\Git\usr\bin\bash.exe",
    ];
    GIT_BASH
        .iter()
        .map(Path::new)
        .find(|path| path.exists())
        .map(|path| path.as_os_str().to_owned())
}

#[test]
fn pre_push_hook_is_valid_bash() {
    // A syntactically broken pre-push hook would fail silently on every git
    // push (git does not run an unparsable hook and reports nothing), which
    // would defeat the fmt/clippy/guard checks the hook provides. `bash -n`
    // parses the script without executing it.
    let Some(program) = bash_program() else {
        eprintln!(
            "pre-push hook syntax check skipped: no bash available (Windows without Git Bash)"
        );
        return;
    };
    let hook = repo_file(".githooks/pre-push");
    let output = std::process::Command::new(&program)
        .arg("-n")
        .arg(&hook)
        .output()
        .expect("run bash -n");
    assert!(
        output.status.success(),
        "bash -n failed for .githooks/pre-push — the hook has a shell syntax error:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn twitch_replies_thread_the_parent_message_id_and_surface_phone_verification() {
    // Twitch replies must use the IRCv3 `@reply-parent-msg-id` tag so the bot
    // answers the exact line a viewer asked about, and the worker must turn
    // the `msg_requires_verified_phone_number` NOTICE into a visible flag so
    // the streamer learns why sending fails (M10 issue #100 reply/phone part).
    let twitch = read("rivulet-core/src/twitch_chat.rs");
    for marker in [
        "pub id: Option<String>",
        "pub fn send_reply",
        "Msg::SendReply",
        "@reply-parent-msg-id=",
        "pub enum TwitchNotice",
        "pub fn parse_notice",
        "msg_requires_verified_phone_number",
        "pub fn phone_verification_required",
    ] {
        assert!(
            twitch.contains(marker),
            "twitch_chat.rs must provide {marker}"
        );
    }
    assert!(
        twitch.contains("CAP REQ :twitch.tv/tags"),
        "replies require the tags capability"
    );
    // The facade must forward replies (rate-limited like plain sends) and
    // expose the phone-verification flag; the GUI dock must arm a reply target
    // per message and surface the server notice.
    let chat = read("rivulet-core/src/chat.rs");
    assert!(
        chat.contains("pub fn send_reply") && chat.contains("pub fn phone_verification_required"),
        "the chat facade must forward replies and the phone flag"
    );
    let gui = read("rivulet-gui/src/app.rs");
    assert!(
        gui.contains("chat_reply_target") && gui.contains("ChatAction::SendReply"),
        "the chat dock must arm threaded replies"
    );
    assert!(
        gui.contains("phone_verification_required()") && gui.contains("chat_phone_verification"),
        "the chat dock must surface the phone-verification requirement"
    );
    let i18n = read("rivulet-core/src/i18n.rs");
    assert!(
        i18n.contains("chat_reply_to") && i18n.contains("chat_phone_verification"),
        "reply/notice UI strings must be translated"
    );
}

#[test]
fn m10_platform_compliance_bullets_are_pinned_in_docs() {
    // M10 issue #100 (platform-compliance baseline): the bot must satisfy
    // each platform's hard constraints, and that contract is specified in
    // BOTH the README roadmap section and the M10 quality gate. Pinning the
    // key markers here makes a silent edit to either document fail CI, so the
    // two sources cannot drift apart or lose a platform.
    let readme = read("README.md");
    assert!(
        readme.contains("**Platform compliance**"),
        "README M10 must carry the Platform compliance bullet"
    );
    // Twitch: scopes, global rate limit, PING/PONG, reply threading, phone
    // verification.
    for marker in [
        "chat:read` + `chat:edit",
        "20 messages/30 s for non-broadcaster/mod/VIP",
        "`PONG` on every `PING`",
        "`reply-parent-msg-id`",
        "`twitch.tv/tags`",
        "phone-verification hint",
    ] {
        assert!(
            readme.contains(marker),
            "README M10 Twitch bullet must mention {marker}"
        );
    }
    // Kick: session-token sending, conservative self-throttle, read-only
    // degradation for the undocumented API.
    for marker in [
        "session token",
        "undocumented",
        "self-throttles conservatively",
        "degrade to read-only/observer mode",
    ] {
        assert!(
            readme.contains(marker),
            "README M10 Kick bullet must mention {marker}"
        );
    }
    // YouTube: official Live Streaming API + OAuth, parentId, quota
    // accounting, read-only Innertube fallback.
    for marker in [
        "Live Streaming API",
        "`youtube.force-ssl`/`youtube` scope",
        "`parentId`",
        "`insert` ≈ 200 units",
        "read-only Innertube poller",
    ] {
        assert!(
            readme.contains(marker),
            "README M10 YouTube bullet must mention {marker}"
        );
    }

    let gates = read("docs/milestone-quality-gates.md");
    assert!(
        gates.contains("Per-platform chat compliance is implemented and verified")
            && gates.contains("The auth/scope matrix is explicit and masked"),
        "the M10 gate must review compliance behavior and the masked auth/scope matrix"
    );
    for marker in [
        "20-messages/30-s",
        "`PING` with `PONG`",
        "`reply-parent-msg-id`",
        "degrades to read-only",
        "`insert` ≈ 200 units",
        "read-only Innertube poller",
        "`chat:read`+`chat:edit`",
        "Kick session token",
        "`youtube.force-ssl`",
        "ever logged, exported, or screenshotted",
        "per-platform compliance test matrix",
    ] {
        assert!(gates.contains(marker), "M10 gate must review {marker}");
    }
}

#[test]
fn stream_workspace_controls_stay_reachable_on_narrow_windows() {
    // Responsive contract of the Meld-style Stream page: below the narrow
    // width threshold the action bar and every control row wrap and the
    // chat/info columns stack, so start/stop, connect, send and mixer stay
    // reachable when the window is shrunk (regression: buttons used to clip
    // off-screen in a plain ui.horizontal / columns(2) layout).
    let app = read("rivulet-gui/src/app.rs");
    assert!(
        app.contains("const STREAM_WORKSPACE_NARROW_WIDTH: f32 = 720.0;"),
        "the stream workspace must define a narrow-width threshold"
    );
    assert!(
        app.contains("action_bar_wraps") && app.contains("horizontal_wrapped"),
        "the action bar and control rows must wrap on narrow windows"
    );
    assert!(
        app.contains("available_width() >= STREAM_WORKSPACE_NARROW_WIDTH")
            && app.contains("self.draw_chat_dock(ui, chat_list_height)"),
        "the chat/info columns must stack instead of clipping on narrow windows"
    );
    assert!(
        app.contains("available_width() - 70.0).max(120.0)"),
        "the chat send input must never get a negative width"
    );
    // The send-budget indicator above the chat input must survive narrow
    // windows too: it renders as an explicitly wrapping Label (.wrap(), never
    // clipped at the right edge of the dock column) and stacks between the
    // reply banner and the input row; the dock itself lives in the page's
    // vertical scroll area (auto_shrink false), so short windows scroll to it.
    let chat = app
        .split_once("fn draw_chat_dock")
        .map(|(_, rest)| rest)
        .expect("draw_chat_dock must exist");
    assert!(
        chat.contains("chat_rate_budget")
            && chat.contains("chat_rate_limited")
            && chat.contains("Label::new")
            && chat.contains(".wrap()"),
        "the send-budget indicator must wrap inside the dock on narrow windows"
    );
    let budget_pos = chat
        .find("chat_rate_budget")
        .expect("budget marker in dock");
    let input_pos = chat
        .find("submit_chat_input")
        .expect("input marker in dock");
    assert!(
        budget_pos < input_pos,
        "the send budget must render above the chat input"
    );
}

#[test]
fn alert_overlay_import_is_wired_through_browser_source() {
    // M5 community dock: alert overlays are imported as browser-source widget
    // URLs (Streamlabs/StreamElements), exactly like OBS. The core must know
    // the provider URL shapes and validate tokens, the GUI must offer the
    // import (provider picker + token + custom URL) and keep it tested, and
    // the feature must be documented.
    let core = read("rivulet-core/src/alerts.rs");
    assert!(core.contains("pub enum AlertProvider"));
    assert!(core.contains("streamlabs.com/alert-box/v2/"));
    assert!(core.contains("streamelements.com/overlay/"));
    assert!(core.contains("pub fn build_overlay_url"));
    assert!(core.contains("pub fn validate_overlay_url"));
    // The generated URLs must keep being accepted by the browser source.
    assert!(core.contains("browser_source_accepts_the_generated_url"));
    let gui = read("rivulet-gui/src/app.rs");
    assert!(gui.contains("fn import_alert_overlay"));
    assert!(gui.contains("alert_provider"));
    assert!(gui.contains("alert_overlay_title"));
    assert!(gui.contains("alert_import_loads_provider_widget_url_into_browser_source"));
    let i18n = read("rivulet-core/src/i18n.rs");
    for key in ["alert_import", "alert_token", "alert_overlay_title"] {
        let k = format!("\"{key}\"");
        assert!(
            i18n.matches(&k).count() >= 2,
            "{key} must exist in EN and DE"
        );
    }
    let docs = read("docs/alerts.md");
    assert!(
        docs.contains("# Stream Alerts") && docs.contains("streamelements.com/overlay/"),
        "alert import must be documented"
    );
}
