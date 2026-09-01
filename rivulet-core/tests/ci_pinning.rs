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
    // Regression: Discord rejected a SET_ACTIVITY whose `details` was an empty
    // string with `4000: "details" is not allowed to be empty` (verified
    // live), silently dropping the whole status update. The payload validator
    // plus the exhaustive wire-contract test must stay wired so such 4000
    // rejections surface locally in CI instead of on a live Discord client.
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
    assert!(discord.contains("empty details on the wire"));
    assert!(discord.contains("text.len() <= 128"));
    assert!(discord.contains("implausible large_image on the wire"));
    // CI runs the contract check as a dedicated, named step.
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("Discord payload contract check")
            && ci.contains("cargo test -p rivulet-core --lib payload"),
        "CI must expose a dedicated Discord payload contract check"
    );
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
fn discord_presence_uses_obs_style_assets_and_no_duplicate_name() {
    // Like OBS, the activity card should render the app artwork (large image
    // from the Discord Developer Portal) instead of the generic placeholder
    // icon, and the payload must not duplicate the app name into state or
    // details (Discord renders the name from the application registration).
    let presence = read("rivulet-core/src/presence.rs");
    assert!(
        presence.contains("let details = match game"),
        "the game name must live in details (state stays the plain label)"
    );
    assert!(
        presence.contains("let state = label.to_owned()"),
        "state must be the plain localized label without the app name"
    );
    let discord = read("rivulet-core/src/discord.rs");
    assert!(discord.contains("large_image_key: Option<String>"));
    assert!(discord.contains("struct ActivityAssets"));
    assert!(discord.contains("#[serde(rename = \"large_image\")]"));
    // Wire-level coverage: assets attached when configured, absent otherwise.
    assert!(discord.contains("set_activity_attaches_large_image_when_configured"));
    // Discord rejects an empty details string (4000: "details" is not allowed
    // to be empty, verified live), so the details field must be omitted when
    // there is no game name instead of being sent empty.
    assert!(
        discord.contains("details: Option<String>")
            && discord.contains("!status.details.trim().is_empty()"),
        "empty details must be omitted, never sent as an empty string"
    );
    assert!(discord.contains("empty_details_is_omitted_not_sent_empty"));
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
            && script.contains("git push origin \"HEAD:$BRANCH\"")
            && script.contains("git push --force-with-lease origin \"HEAD:$BRANCH\""),
        "release-branch.sh must publish current HEAD onto the release branch, \
         fast-forwarding when the remote is at/behind HEAD and force-with-lease \
         overwriting a divergent/stale tip so a release always builds current source"
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
                "needs: [lints, beta_gate, build_and_test, pinning_tests, srt_receiver_smoke, rist_receiver_smoke, obs_websocket_smoke]"
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
        "rivulet-obs-websocket/Cargo.toml",
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
    // M5 community dock: the Chat view must be a first-class sidebar entry
    // (nav key + heading i18n), the core worker must keep its local-listener
    // smoke test (deterministic in CI without real network), and the feature
    // must be documented.
    let app = read("rivulet-gui/src/app.rs");
    assert!(
        app.contains("AppView::Chat") && app.contains("nav_chat"),
        "the Chat view must be a first-class AppView with a nav key"
    );
    assert!(
        app.contains("fn draw_chat_view") && app.contains("fn reconcile_twitch_chat"),
        "the Chat view must have a draw + reconcile path"
    );
    let core = read("rivulet-core/src/twitch_chat.rs");
    assert!(
        core.contains("worker_connects_and_delivers_messages_to_local_listener"),
        "the Twitch worker must keep its local-listener smoke test"
    );
    let i18n = read("rivulet-core/src/i18n.rs");
    assert!(
        i18n.contains("(\"chat_title\", \"Twitch Chat\")")
            && i18n.contains("(\"chat_title\", \"Twitch-Chat\")"),
        "the chat view must be localized in DE and EN"
    );
    let docs = read("docs/twitch-chat.md");
    assert!(
        docs.contains("# Twitch Chat Dock") && docs.contains("parse_irc_line"),
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
