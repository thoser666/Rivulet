# Rivulet Governance

This document describes how Rivulet is governed: how decisions are made, which
roles exist, who currently holds them, and how the project stays healthy.

## Governance model

Rivulet uses a **maintainer model with open contribution**:

- The project is led by one or more **maintainers** who own the repository,
  the roadmap, and the release pipeline.
- Anyone may contribute through the normal fork-and-pull-request flow described
  in [`CONTRIBUTING.md`](CONTRIBUTING.md); acceptance is gated by the review
  policy and the Definition of Done, not by membership.
- Day-to-day decisions follow the documented roadmap (see
  [`README.md`](README.md#-roadmap)) and issue labels; large direction changes
  are discussed in issues before they are scheduled into a milestone.

## Roles and responsibilities

### Maintainers

Maintainers have write access to the repository and are responsible for:

- Keeping `develop` green and the release pipeline healthy (alpha releases are
  automatic; beta/RC/stable tags are a manual maintainer decision, see
  [`CONTRIBUTING.md`](CONTRIBUTING.md#release-strategy)).
- Triaging issues, assigning milestones/labels, and closing decisions.
- Reviewing and merging pull requests.
- Enforcing the [Code of Conduct](CODE_OF_CONDUCT.md) and the
  [security policy](SECURITY.md).
- Guarding the branch-protection ruleset (see
  [`docs/security.md`](docs/security.md)) — no one, including maintainers,
  pushes to `develop` directly.

**Current maintainers:** `thoser666` (https://github.com/thoser666).

### Contributors

Contributors submit changes via pull requests. Their responsibilities are
documented in [`CONTRIBUTING.md`](CONTRIBUTING.md) (commit convention,
Definition of Done, code review policy, DCO).

### Reviewers

Reviewers are contributors or maintainers who review pull requests. Because the
project is largely single-maintainer, the executable CI checks (fmt, clippy
`-D warnings`, the full test suite, the ci_pinning supply-chain guards,
actionlint, ShellCheck) act as automated reviewers and must be green before any
merge — see [`CONTRIBUTING.md`](CONTRIBUTING.md#code-review-policy).

### Users

Users report bugs and request features through the issue tracker. The
[`SECURITY.md`](SECURITY.md) policy describes how security issues must be
reported privately instead.

## Decision making

1. **Roadmap.** The milestone list in `README.md` is the plan of record.
   Milestone quality gates are defined in
   [`docs/milestone-quality-gates.md`](docs/milestone-quality-gates.md).
2. **Bugs and enhancements.** Issues are triaged by maintainers. Bug fixes
   follow the Definition of Done (including regression tests — see
   [`docs/regression-testing.md`](docs/regression-testing.md)).
3. **Design changes.** Material design or architecture changes are discussed in
   an issue first; the outcome is recorded in `docs/` so decisions have a
   written trail.
4. **Disagreements.** Maintainers make the final call. Disputes are resolved
   by reference to the roadmap, the quality gates, and the Code of Conduct.

## Release process

Release channels (alpha automatic / beta / RC / stable) are described in
[`CONTRIBUTING.md`](CONTRIBUTING.md#release-strategy) and
[`docs/release-platforms.md`](docs/release-platforms.md). Only maintainers tag
non-alpha releases.

## Succession and bus factor

- Every maintainer should document, in a private succession note, how
  repository, package, and signing access can be transferred if they become
  unavailable (keys, credentials, and rights).
- The project explicitly welcomes a second maintainer; onboarding happens via
  the issue tracker / discussion so it is a public, reviewable decision.

## Scope and non-goals

Governance covers the `thoser666/Rivulet` repository and its releases. The
wiki (`thoser666/Rivulet.wiki`) is maintained as a documentation companion and
follows the content policy in
[`docs/wiki-content-policy.md`](docs/wiki-content-policy.md).
