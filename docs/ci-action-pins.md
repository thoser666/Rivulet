# CI Action Pins

Human-readable reference for the third-party GitHub Actions used in
`.github/workflows`. Every action is pinned to a full 40-character commit SHA
(plus a version comment inline in the workflow) so a repointed tag or branch
cannot swap in a malicious commit.

Dependabot keeps the pins current: when an action releases a new version it
opens a PR that updates both the SHA and the inline version comment. The table
below is generated from the workflows by `scripts/generate-action-pins.py`
(do not edit it by hand) and is the review aid for those PRs:

- **Action** — the upstream repository.
- **Version** — the upstream release (or branch) this SHA corresponds to.
- **Pinned SHA** — the immutable commit the workflows actually run.
- **Used in** — which workflows reference the action.

<!-- action-pins-table:start -->
| Action | Version | Pinned SHA | Used in |
| --- | --- | --- | --- |
| `EmbarkStudios/cargo-deny-action` | `v2.1.1` | `3c6349835b2b7b196a839186cb8b78e02f7b5f25` | security.yml |
| `actions-rust-lang/audit` | `v1.2.7` | `72c09e02f132669d52284a3323acdb503cfc1a24` | security.yml |
| `actions/cache` | `v6.1.0` | `55cc8345863c7cc4c66a329aec7e433d2d1c52a9` | build-package.yml, ci.yml, nightly.yml |
| `actions/checkout` | `v7.0.1` | `3d3c42e5aac5ba805825da76410c181273ba90b1` | build-package.yml, ci.yml, distribution-readiness.yml, nightly.yml, obs-upstream.yml, release.yml, scorecard.yml, security.yml, signing-e2e.yml, wiki-translations.yml |
| `actions/dependency-review-action` | `v5.0.0` | `a1d282b36b6f3519aa1f3fc636f609c47dddb294` | security.yml |
| `actions/download-artifact` | `v8.0.1` | `3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c` | ci.yml, release.yml |
| `actions/upload-artifact` | `v7.0.1` | `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` | build-package.yml, obs-upstream.yml, scorecard.yml |
| `dtolnay/rust-toolchain` | `stable` | `4360b52568e2003a75bf9bc1d59f33a8e3fc893c` | build-package.yml, ci.yml, nightly.yml, security.yml — the fuzz_smoke job overrides `toolchain: nightly` (cargo-fuzz requirement), still at the same pinned SHA |
| `github/codeql-action/analyze` | `v4.37.9` | `cdf488f595d80d6e07e03d4674febd5ab45fa938` | security.yml |
| `github/codeql-action/autobuild` | `v4.37.9` | `cdf488f595d80d6e07e03d4674febd5ab45fa938` | security.yml |
| `github/codeql-action/init` | `v4.37.9` | `cdf488f595d80d6e07e03d4674febd5ab45fa938` | security.yml |
| `github/codeql-action/upload-sarif` | `v4.37.9` | `cdf488f595d80d6e07e03d4674febd5ab45fa938` | scorecard.yml |
| `ossf/scorecard-action` | `v2.4.4` | `2d1146689b8cda280b9bc96326124645441f03bc` | scorecard.yml |
| `softprops/action-gh-release` | `v3.0.3` | `efb35369e0ad2afab669f228072c1b0d510eae64` | ci.yml, release.yml |
<!-- action-pins-table:end -->

> `dtolnay/rust-toolchain` is pinned to the `stable` **branch** (not a release
> tag): the SHA pins the action *code*, while the action's default
> `toolchain: stable` input still installs the latest stable Rust. It therefore
> has no semver comment.
>
> The **compiler version itself is pinned by `rust-toolchain.toml`** (repo
> root, channel `1.98.0`, components rustfmt + clippy): every `cargo`
> invocation in any checkout resolves to that exact toolchain, so local and CI
> `cargo fmt --check` can never diverge the way they did in the 0.65.0-alpha.100
> window (rustfmt 1.9.0 vs the runner's older rustfmt flip-flopped on a long
> `contains()` line). Bump the channel deliberately — a new compiler version
> can change rustfmt output and redden the Lints job until the tree is
> reformatted with the same version.

## Reviewing a Dependabot update

When Dependabot opens a PR that bumps a pin, verify that the new SHA really is
the commit behind the advertised release before merging:

```bash
# For a release tag (most actions):
git ls-remote https://github.com/<owner>/<repo>.git refs/tags/<version>
# e.g.:
git ls-remote https://github.com/actions/checkout.git refs/tags/v5.1.0

# Same via the GitHub API:
gh api repos/<owner>/<repo>/commits/<version> --jq .sha

# For the rolling `stable` branch (dtolnay/rust-toolchain only):
git ls-remote https://github.com/dtolnay/rust-toolchain.git refs/heads/stable
```

The printed commit must equal the SHA in the PR. Then run
`scripts/generate-action-pins.py` to refresh this table and update the matching
entry in `rivulet-core/tests/ci_pinning.rs` so the reference and the
enforcement stay in sync (the test suite fails otherwise).

## Auto-merge

`.github/workflows/dependabot-auto-merge.yml` approves Dependabot PRs and
enables auto-merge, so a bump is merged automatically once the required CI
checks pass — those checks re-run `rivulet-core/tests/ci_pinning.rs` (the
pinning tests) as part of the `Build & Test` matrix. Two repo settings make
this take effect (they cannot be committed):

1. **Settings → General → Pull Requests → "Allow auto-merge"** must be on.
2. **Branch protection on `develop`** must require the `Build & Test` status
   checks. Auto-merge needs at least one required status check; GitHub then
   merges only after those checks are green.

Without those settings, the workflow's `gh pr merge --auto` step fails on the
next Dependabot PR (visible in the Actions tab) — the intended signal that the
repository is not configured yet.

## Enforcement

- `rivulet-core/tests/ci_pinning.rs` — fails if any third-party `uses:` is not
  a full 40-char SHA, if the reviewed pins are not the ones in use, or if this
  table drifts from those pins.
- `scripts/check-action-pins.py` — compares every pinned SHA against upstream
  (run daily by the nightly workflow): fails on same-major updates we have not
  taken. The nightly workflow invokes it with `--fail-on-major`, so **newer
  major versions also fail the job** instead of only being reported — a major
  gap therefore surfaces as a red Nightly run rather than a warning.
  `--json` emits a machine-readable result; `--comment` emits a compact Markdown
  notification that the nightly workflow publishes to the run's step summary.
  Compound actions (`owner/repo/path/to/action`) are resolved against the
  `owner/repo` repository that owns the tags, so the sub-path is stripped before
  comparing SHAs.
- `.github/dependabot.yml` — the `github-actions` entry that proposes the
  updates in the first place.
