# OpenSSF Best Practices Badge — Evidence Map

Rivulet targets the [OpenSSF Best Practices badge](https://www.bestpractices.dev)
(passing level, metal series). This page records, for the criteria that live
**in this repository**, where the evidence is and how it is kept honest by CI.
The final self-certification happens on `bestpractices.dev` with the
maintainer's GitHub account (badge claims are self-assertions presented for
public scrutiny); everything the badge asks for that the repo can own is owned
here and pinned by tests so it cannot silently drift away.

Criteria that depend on human behaviour (responding to bug reports within
2–12 months, vulnerability response within 14 days, a maintainer who knows
secure-design principles) cannot be satisfied by a file and are answered
during the online self-certification.

## Repository status

| Item | Status |
| --- | --- |
| Repository | Public (`thoser666/Rivulet`), HTTPS, FLOSS (MIT) `LICENSE` |
| Version control | Git on GitHub; interim versions on `develop` between releases |
| Versioning | Semantic versions, git tags per release (alpha/beta/RC/stable) |
| Issue tracking | GitHub Issues + labels + milestones |
| Vulnerability reporting | `SECURITY.md` (private advisory, coordinated disclosure) |
| CI | GitHub Actions: build/test on 3 OS, Lints, Security, Pinning, smokes |
| Static analysis | CodeQL, Cargo Audit, Cargo Deny, OpenSSF Scorecard, clippy `-D warnings` |
| Dynamic analysis | cargo-fuzz targets (CI smoke + weekly deep campaign) |
| Release hygiene | Generated notes + completeness check + `SHA256SUMS` + HTTPS |
| Codebases | Primary `thoser666/Rivulet` + `Rivulet.wiki` docs companion (README **Repositories** section) |
| Automation proposals | `.bestpractices.json` (baseline-1 24/24 + metal passing subset, schema-validated by CI) |

## Contribution process (`contribution`, `contribution_requirements`)

Evidence: [`CONTRIBUTING.md`](../CONTRIBUTING.md)

- Contribution process: feature branch → PR against `develop` → CI runs →
  merge (see **Development Workflow**).
- Requirements for acceptable contributions: the **Definition of Done**
  section lists what every contribution must meet: tests for new
  functionality, EN+DE i18n parity, documentation + CHANGELOG updates,
  green fmt/clippy/tests, Conventional Commits, and a no-secrets rule.

## Code review policy

Evidence: [`CONTRIBUTING.md`](../CONTRIBUTING.md) (**Code Review Policy**),
[`docs/security.md`](security.md) (branch ruleset).

- Every change — contributor **and** maintainer — merges via a pull request
  against `develop`; the branch ruleset lists **no bypass actors**, so direct
  pushes to `develop` are rejected for every actor, administrators included
  (`osps_ac_03_01`). The ruleset requires the status checks (`CI`,
  `Security`, `Pinning-Tests`, `OpenSSF Scorecard`) before merging.
- Because this is a single-maintainer repository the ruleset does not require
  a second human approval; the committed pre-push hook (fmt + clippy
  `-D warnings`) and the fast-guard stage still run before every push.
- Workflow/packaging/signing/security changes get an approving PR review
  whenever a second reviewer is available.
- Automated checks are treated as reviewers for the single-maintainer case
  and run on every push.

## Release verification (`release_notes`, `release_notes_vulns`, `delivery_mitm`, `version_*`)

Evidence: [`CONTRIBUTING.md`](../CONTRIBUTING.md) (**Release Verification**),
[`.github/workflows/release.yml`](../.github/workflows/release.yml),
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml),
[`docs/security.md`](security.md) (release asset integrity).

- Every release has a unique semantic version and a git tag.
- The release-notes body is generated from the actual commits since the
  previous tag and verified for completeness before publishing (no raw
  `git log`, no release-prep commits).
- `fix(security):` commits must carry their CVE/GHSA/RUSTSEC identifier in
  the subject, so each release's notes identify fixed publicly known
  vulnerabilities.
- A `SHA256SUMS` manifest is generated over all attached assets, published
  with the release, and verified by the updater before install — transport is
  HTTPS, so the update path counters MITM.
- The `LICENSE` file is copied into the release asset folder before the
  manifest is generated (release.yml alpha path and ci.yml beta/RC/stable tag
  path), so the MIT license ships alongside every release and is covered by
  `SHA256SUMS` (`osps_le_03_02`).

## Automation proposals (`.bestpractices.json`)

Evidence: [`.bestpractices.json`](../.bestpractices.json)

When a project is registered at `bestpractices.dev`, the badge site reads a
`.bestpractices.json` file from the repository top level and turns its fields
into 🤖 automation proposals the maintainer reviews while answering the
questionnaire (field naming per the
[automation-proposals](https://github.com/ossf/best-practices-badge/blob/main/docs/automation-proposals.md)
and [bestpractices-json](https://github.com/ossf/best-practices-badge/blob/main/docs/bestpractices-json.md)
docs). The file proposes `Met` for **all 24** baseline-1 controls; the former
gaps were closed as follows:

- `osps_ac_03_01` — the `develop` ruleset lists no bypass actors, so direct
  commits are prevented for every actor (PR + required checks only).
- `osps_le_03_02` — the release workflows ship the `LICENSE` file with every
  release (covered by `SHA256SUMS`).
- `osps_qa_04_01` — the README **Repositories** section lists the primary
  codebase and the wiki companion with status and intent.

Statuses the site understands: `Met`, `Unmet`, `N/A`, `?` and `unknown`
(`?`/`unknown` are ignored and safe as placeholders).

Beyond baseline-1, the file also carries **metal-series passing** proposals
for the criteria the repo demonstrably meets and that a file can vouch for:
`contribution`, `contribution_requirements`, `floss_license`,
`license_location`, `documentation_basics`, `sites_https`, `discussion`,
`english`, `repo_public`, `repo_track`, `repo_interim`, `version_unique`,
`release_notes`, `report_process`, `report_tracker`, `report_archive`,
`vulnerability_report_process`, `vulnerability_report_private`, `build`,
`test`, `test_invocation`, `test_policy`, `warnings`, `warnings_fixed`,
`static_analysis`, `no_leaked_credentials`, `delivery_mitm`,
`dynamic_analysis`, `description_good` and `interact`. Behavioural criteria
(`maintained`, `report_responses`, `know_secure_design`, the `crypto_*`
family, `vulnerabilities_fixed_60_days`, …) are deliberately **not** claimed
from a file — they are answered truthfully during the online
self-certification.

## How this page stays true

The following are pinned by `rivulet-core/tests/ci_pinning.rs` so the
evidence cannot drift out of existence:

- `contributing_defines_code_review_policy_and_definition_of_done` — the
  Code Review Policy, Definition of Done, and Release Verification sections
  stay present with their key markers.
- The existing release-notes guards (`release_notes_completeness_is_checked_in_ci`,
  `alpha_release_notes_are_generated_from_commits_since_last_tag`,
  `tag_based_release_attaches_checksums_and_generated_notes`) and the
  security-policy guard (`security_policy_is_linked_from_readme_and_docs`)
  keep the underlying automation in place.
- `bestpractices_json_claims_are_schema_valid_and_canonical` —
  `.bestpractices.json` must stay valid JSON, may only use canonical
  baseline-1 criterion keys, may only carry legal status values
  (`Met`/`Unmet`/`N/A`/`?`/`unknown`), and every one of the 24 canonical
  criteria must have a status key (complete coverage); every concrete claim
  needs a non-empty justification.
- `ossf_baseline_1_gap_closures_are_pinned` — the three former gaps stay
  closed: `.bestpractices.json` claims `osps_ac_03_01`, `osps_le_03_02` and
  `osps_qa_04_01` as `Met`, the README keeps the multi-repo list, both
  release workflows ship the license, and the docs keep the no-bypass
  ruleset wording.
- `develop_ruleset_guard_proves_direct_pushes_are_blocked` — `osps_ac_03_01`
  is verified live on every pull request and after every push to `develop`:
  the Ruleset Guard workflow (`ruleset-guard.yml` +
  `scripts/check-develop-ruleset.py`) asserts via the read-only rulesets API
  that the `develop` ruleset stays active, lists no bypass actors, and keeps
  its `pull_request` and required-status-check rules.
- `bestpractices_metal_passing_claims_are_pinned` — the claimed metal
  passing criteria stay exactly in sync: every listed criterion is `Met`
  with evidence, no other metal passing criterion gains a claim without a
  deliberate list change, and the schema guard accepts the full canonical
  passing key set (`METAL_PASSING_CRITERIA`).

## Silver gap closures (repo-side)

The repo-side artifacts that make the *silver* criteria claimable are pinned
by `ossf_silver_gap_closures_are_pinned`:

- **`governance` / `roles_responsibilities`** — [`GOVERNANCE.md`](../GOVERNANCE.md)
  documents the governance model, maintainer/contributor/reviewer/user roles,
  decision-making, release process, and succession.
- **`code_of_conduct`** — [`CODE_OF_CONDUCT.md`](../CODE_OF_CONDUCT.md)
  (Contributor Covenant 2.1) sits at the repository root, the standard
  location GitHub surfaces on the front page.
- **`documentation_achievements`** — the README front page carries an
  [`Achievements`](../README.md#-achievements) section that hyperlinks the
  OpenSSF best-practices badge (`bestpractices.dev/projects/14447`), the
  license, and the community-rule documents.
- **`dco`** — `CONTRIBUTING.md` documents the Developer Certificate of Origin;
  [`scripts/check-dco.py`](../scripts/check-dco.py) verifies that every PR
  commit carries a `Signed-off-by` trailer matching its author. The check runs
  in CI (`ci.yml`, job `DCO (Signed-off-by)`) and in the local pre-push hook,
  and the script has a `--self-test`.
- **`regression_tests_added50`** — [`docs/regression-testing.md`](../docs/regression-testing.md)
  is the formal policy and tracking mechanism (every bug fix ships a
  regression test; exceptions must be named); it is wired into the
  Definition of Done and the pull-request template.
- **`access_continuity` / `bus_factor`** — [`GOVERNANCE.md`](../GOVERNANCE.md)
  now documents a concrete **access-continuity plan**: a backup-maintainer
  onboarding path, a private credential-contingency procedure (key/signing/DNS
  handover), and a one-week recovery window. The current honest status (bus
  factor 1, single maintainer) is stated explicitly rather than overclaimed.
- **`assurance_case`** — [`docs/security/assurance-case.md`](../docs/security/assurance-case.md)
  is the written security assurance case: security requirements, threat model,
  trust boundaries, the secure-design argument, and the table of common
  implementation weaknesses counteracted (fuzzing, secret-redaction,
  fail-closed updates, memory safety).
- **`test_statement_coverage80`** — [`scripts/coverage-gate.sh`](../scripts/coverage-gate.sh)
  measures `rivulet-core` statement coverage with `cargo-llvm-cov` and
  **fails CI below 80 %** (job `Statement Coverage (rivulet-core >= 80%)`).
  Scope is the core library because the platform hook/launcher shims cannot be
  exercised headlessly; measured locally at ~88 %.
- **`build_preserve_debug`** — the release profile in [`Cargo.toml`](../Cargo.toml)
  deliberately preserves debug info (no `strip`, no `install -s`,
  RUSTFLAGS `-C debuginfo` passes through) and documents that choice inline.

## Remaining (maintainer action, not repo files)

1. Answer the remaining behavioural criteria and the structural items that a
   file cannot fix alone: `bus_factor` (needs a second maintainer),
   `signed_releases` / `version_tags_signed` (needs a trusted signing key +
   user-verification documentation; currently only self-signed smoke tests).
2. Accept the 🤖 automation proposals from `.bestpractices.json` as you go;
   the coverage gate above is what makes `test_statement_coverage80` honest.
3. Embed the badge markdown in `README.md` (already present) and keep the
   badge status current.
