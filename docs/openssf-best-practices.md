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

- Contributors merge via pull requests against `develop`; the branch ruleset
  requires the status checks (`CI`, `Security`, `Pinning-Tests`, `OpenSSF
  Scorecard`) before merging.
- Maintainer direct pushes are the exception and must first pass the
  committed pre-push hook (fmt + clippy `-D warnings`) and the fast-guard
  stage; CI re-runs the same checks.
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

## Remaining (maintainer action, not repo files)

1. Register the project at <https://www.bestpractices.dev> with the GitHub
   account and answer the questionnaire (baseline-1 series is the fastest
   start; the metal "passing" series is the badge referenced above).
2. Answer the behavioural criteria truthfully (bug/vulnerability response
   history, maintainer security knowledge).
3. Embed the badge markdown in `README.md` once the project has an ID.
