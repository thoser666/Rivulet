# Contributing to Rivulet

## Commit Convention

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

feat(core): add recording preset management
fix(gui): correct hotkey handling on macOS
docs(readme): update M1 checklist
chore(deps): upgrade ureq from 2.12 to 3.4
```

**Allowed types:** `feat`, `fix`, `docs`, `chore`, `refactor`, `test`, `ci`, `build`, `perf`

Scopes are optional but encouraged: `core`, `gui`, `audio`, `capture`, `streaming`, `updater`, `plugins`, `i18n`, `signing`, `ci`, `deps`.

### Developer Certificate of Origin (DCO)

Every commit MUST carry a `Signed-off-by` trailer matching the commit author:

```
fix(gui): keep the action bar reachable in narrow windows

Signed-off-by: Ada Example <ada@example.com>
```

The trailer asserts that the author is legally entitled to make the
contribution under the project's MIT license and agrees to the Developer
Certificate of Origin (<https://developercertificate.org>). It is enforced by
CI (`scripts/check-dco.py` runs on every pull request) and locally by
`git commit -s`. Community rules, escalation, and enforcement follow the
[Code of Conduct](CODE_OF_CONDUCT.md) and the
[governance model](GOVERNANCE.md).

---

## Release Strategy

### Two-Channel Architecture

Rivulet uses a **two-channel** release model:

| Channel | Tag format | Trigger | Example |
| --- | --- | --- | --- |
| **Alpha** | `v0.X.Y-alpha.N` | Every `feat:`, `fix:`, `build:`, `chore:` or `ci:` push to `develop` **whose CI run concludes successfully** | `v0.21.0-alpha.1` |
| **Beta / RC / Stable** | `vX.Y.Z` | Manually set tag (no suffix) | `v1.0.0` |

### Alpha Channel (Automatic, CI-gated)

Every releasable commit pushed to `develop` automatically produces an alpha
release — **after the CI workflow for that commit concluded successfully**.
The release workflow is triggered by `workflow_run` on the CI workflow
(`types: [completed]`, branch `develop`), so a failed or cancelled CI run
simply skips the release instead of starting one. Rerunning CI green fires
the release workflow again automatically, so a transient runner flake never
requires a manual release rerun.

| Commit type | SemVer bump | Example |
| --- | --- | --- |
| `fix(...)` | Patch | `0.20.0` → `0.20.1-alpha.1` |
| `feat(...)` | Minor | `0.20.0` → `0.21.0-alpha.1` |
| `feat!(...)` or `BREAKING CHANGE` | Major | `0.x.y` → `1.0.0-alpha.1` |
| `build(...)`, `chore(...)` or `ci(...)` | Patch | `0.20.0` → `0.20.1-alpha.1` |

The workflow (`release.yml`) handles everything:
0. Waits for the CI workflow of the commit to complete; only proceeds when it concluded `success` (failed/cancelled CI skips; rerun CI green to auto-resume)
1. Detects `feat:`, `fix:`, `build:`, `chore:` or `ci:` commits (skips Dependabot)
2. Computes the next SemVer version from commit history
3. Bumps `Cargo.toml` and `CHANGELOG.md`
4. Commits, tags, and pushes
5. Builds platform binaries (Linux, Windows, macOS)
6. Creates a GitHub pre-release whose notes are generated automatically from
the commits since the previous tag (grouped by conventional-commit type,
release-prep commits excluded) and whose assets include the `SHA256SUMS`
checksum manifest the updater verifies against — the notes body is
additionally verified for completeness before publishing (every non-prepare
commit since the previous tag must appear, see `scripts/check-release-notes.py`)

`build:`, `chore:` and `ci:` commits release with a patch bump so packaging,
housekeeping and CI changes (installer layouts, workflow fixes, dependency
housekeeping) ship automatically instead of needing a manual *Actions → Release
(Alpha) → Run workflow* dispatch afterwards. Purely editorial commits (`docs:`,
`test:`, `style:`) do not trigger a release.

### Beta / RC / Stable Channel (Manual)

When a milestone is ready for a non-alpha release:

```bash
# 1. Ensure the version in Cargo.toml is correct (e.g. 0.3.0)
# 2. Tag without the alpha suffix:
git tag -a v0.3.0 -m "Release v0.3.0"
git push origin v0.3.0
```

The CI workflow (`ci.yml`) picks up the tag, runs the full test suite, builds binaries, and creates a GitHub release — with the same release hygiene as the alpha channel: the notes body is generated from the commits since the previous tag (grouped by conventional-commit type, release-prep commits excluded), verified for completeness, and a `SHA256SUMS` manifest over all attached assets is published so the updater can verify stable installers too. Beta and RC tags are marked as pre-releases; stable tags are full releases.

**Prerequisites for beta/stable** (checked by `scripts/check-beta-gate.py`):
- All M1 checklist items verified
- CI fully green on `develop`
- Code-signing secrets configured
- No known release-blocking bugs

### When to Use Which

| Situation | Action | Result |
| --- | --- | --- |
| Bug fix | `fix(core): ...` → push to `develop` | Auto alpha patch release |
| New feature | `feat(gui): ...` → push to `develop` | Auto alpha minor release |
| Breaking change | `feat!(core): ...` → push to `develop` | Auto alpha major release |
| Milestone complete | Tag `vX.Y.Z` manually | Beta / RC / Stable release |
| Hotfix on stable | `fix(...)` → cherry-pick to `main` → tag `vX.Y.Z+1` | Patch release |

### Version History

All releases are alpha until the first beta. The version in `Cargo.toml` is the single source of truth — it is automatically updated by the release workflow on every alpha release.

### Release Verification

Every release — alpha, beta, RC, or stable — is verified before and after it
is published (this is also what satisfies the OpenSSF Best Practices
`release_notes` / `release_notes_vulns` / `delivery_mitm` criteria):

- **Built from the tested commit.** Releases only proceed when the CI run of
  the release commit concluded `success`; the workflow checks out that exact
  SHA (`workflow_run.head_sha`), so the artifact always matches the tested
  source.
- **Notes are complete.** The release-notes body is generated from the actual
  commits since the previous tag and `scripts/check-release-notes.py`
  verifies that every non-prepare commit appears before the release is
  created — the notes are a human-readable, grouped summary, never a raw
  `git log` dump.
- **Fixed vulnerabilities are identified.** Security fixes use the
  `fix(security):` prefix and MUST carry the CVE/GHSA/RUSTSEC identifier in
  the commit subject, so the generated notes and `CHANGELOG.md` identify
  every publicly known run-time vulnerability fixed in the release (the
  `release_notes_vulns` criterion).
- **Artifacts are integrity-checked.** A `SHA256SUMS` manifest over every
  attached asset is generated and published with the release; the updater
  downloads the manifest and verifies the installer digest before install,
  failing closed on mismatch. Assets are distributed over HTTPS
  (GitHub Releases), which counters MITM tampering.
- **Stable/beta gating.** Non-alpha releases additionally require the
  beta-gate criteria (see above) to be met before they are cut.

---

## Development Workflow

1. Create a feature branch from `develop`
2. Make changes with conventional commits
3. Push and open a PR against `develop`
4. CI runs: lint, test suite, actionlint, ShellCheck
5. Merge (squash or rebase)
6. If the commit is `feat:` or `fix:`, an alpha release is automatically built and published

### Local pre-push checks

A committed git hook (`.githooks/pre-push`) mirrors the CI **Lints (Fmt &
Clippy)** job locally, so pushes cannot break the required checks. Enable it
once per clone:

```bash
git config core.hooksPath .githooks
```

The hook runs `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets -- -D warnings` with the exact CI
flags. The `-D warnings` part matters: a plain `cargo clippy` only *warns*
on lints such as `clippy::field_reassign_with_default`, which is how commit
`06792c6` shipped GUI tests that were clean locally but failed the CI Lints
job.

An optional **fast-guard stage** (default on) then runs the repo-internal
guards that CI would check later: the CI pinning tests
(`cargo test -p rivulet-core --test ci_pinning`) plus the Lints-job script
self-tests (`generate-action-pins.py --check`, parity-checklist/release-
notes/theme-contrast checks, `generate-release-notes.sh --self-test`).
Disable just that stage with `RIVULET_PRE_PUSH_FAST_TESTS=0 git push`; skip
the whole hook for a single push with `RIVULET_SKIP_PRE_PUSH=1 git push`
(or `git push --no-verify`).

Edits to the hook itself are guarded too: the ci_pinning suite runs
`bash -n` on `.githooks/pre-push`, so a hook with a shell syntax error
fails CI (and the local pre-push run) instead of being silently skipped by
git on every push. On Windows the check prefers an explicit Git for
Windows bash (a `bash` on PATH may be the WSL launcher) and is skipped
locally only when no Git Bash is installed — the CI Pinning-Tests job on
ubuntu keeps it enforced.

---

## Code Review Policy

Every change to `develop` passes a review before it lands:

1. **Pull requests for contributors.** The `develop` ruleset requires a pull
   request plus the required status checks (`CI`, `Security`,
   `Pinning-Tests`, `OpenSSF Scorecard`) before a merge; contributors never
   push to `develop` directly. See the ruleset description in
   `docs/security.md`.
2. **Maintainer direct pushes.** Direct pushes to `develop` are no longer
   possible: the branch ruleset lists **no bypass actors**, so even repository
   administrators are rejected. Every change — maintainer or contributor —
   lands as a pull request whose required status checks must pass before the
   merge. The committed pre-push hook (fmt + clippy with `-D warnings`) and
   the fast-guard stage still run locally before any push, so the CI round-trip
   on a pull request is the safety net, not the first line of defence.
3. **Sensitive changes always get a PR.** Changes to workflows, packaging,
   signing, the release pipeline, security policy, and shared-memory/capture
   security code are reviewed as pull requests with an approving review
   before merge whenever a second reviewer is available.
4. **Automated review is part of review.** Because much of this repository is
   single-maintainer, the executable checks are treated as reviewers too:
   fmt, clippy `-D warnings`, the full test suite, the ci_pinning supply-
   chain guards, actionlint, and ShellCheck all run on every push and must be
   green before the change is considered reviewed. Release artifacts are
   additionally verified (notes completeness, `SHA256SUMS`) as described in
   the [Release Strategy](#release-strategy).

---

## Definition of Done (Acceptable Contributions)

A contribution is acceptable when it meets **all** of the following:

- **Tests for new functionality.** Major new functionality MUST add
   automated tests to the relevant suite (this is the project's formal test
   policy — see the OpenSSF Best Practices `test_policy` criterion). Tests
   for pure-wiring and release-automation changes live in
   `rivulet-core/tests/ci_pinning.rs` as content guards.
- **Regression test with every bug fix.** A `fix:` change MUST add or update a
   regression test that fails on the unfixed code (policy and exceptions in
   [`docs/regression-testing.md`](docs/regression-testing.md)).
- **i18n parity.** Any user-visible string must exist in **both** the EN and
   DE locale tables (`rivulet-core/src/i18n.rs`) in the same order.
- **Documentation.** User-visible behavior is reflected in `README.md`, the
   relevant `docs/*.md` page, and the `CHANGELOG.md` entry for the change.
- **Checks are green.** `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`, and the test
   suites pass.
- **Commit convention.** The commit uses the Conventional Commits format
   described above; `fix(security): ...` commits that close a publicly known
   vulnerability MUST reference its CVE/GHSA/RUSTSEC identifier in the commit
   subject so it surfaces in the release notes and changelog.
- **No secrets.** Stream keys, tokens, certificates, and personal paths never
   appear in commits, logs, docs, or issue text (see `SECURITY.md`).

## Testing

```bash
# Run the full test suite
cargo test

# Requires GStreamer dev packages + plugins
# On Linux: export LIBCLANG_PATH=/usr/lib/llvm-14/lib
```

## Code Style

- `#![allow(unused_imports)]` is intentional in `app.rs` (cross-platform)
- All i18n keys must exist in **both** EN and DE locale tables in the same order
- Hardware-encoding tests are gated on GPU availability
- Prefer `AtomicBool`/`AtomicU64` for cross-thread state

---

## Questions?

Open an issue at https://github.com/thoser666/rivulet/issues
