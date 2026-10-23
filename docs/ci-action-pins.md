# CI Action Pins

Human-readable reference for the third-party GitHub Actions used in
`.github/workflows`. Every action is pinned to a full 40-character commit SHA
(plus a version comment inline in the workflow) so a repointed tag or branch
cannot swap in a malicious commit.

Dependabot keeps the pins current: when an action releases a new version it
opens a PR that updates both the SHA and the inline version comment. It runs
**daily** and opens **one grouped PR** for all stale action pins (see the
`github-actions` entry in `.github/dependabot.yml`): the blocking `stale_pins`
gate reddens every open PR whose pins are behind upstream, so a per-action PR
would still carry the other stale pins and could never go green, and a weekly
schedule would leave the fleet stale-gated for up to seven days. The table
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
| `actions/cache` | `v6.1.0` | `55cc8345863c7cc4c66a329aec7e433d2d1c52a9` | build-package.yml, ci.yml, fuzz-deep.yml, nightly.yml, obs-upstream.yml |
| `actions/checkout` | `v7.0.1` | `3d3c42e5aac5ba805825da76410c181273ba90b1` | build-package.yml, ci.yml, distribution-readiness.yml, flatpak-build.yml, fuzz-deep.yml, nightly.yml, obs-upstream.yml, release.yml, ruleset-guard.yml, scorecard.yml, security.yml, signing-e2e.yml, weekly-promotion.yml, wiki-translations.yml |
| `actions/dependency-review-action` | `v5.0.0` | `a1d282b36b6f3519aa1f3fc636f609c47dddb294` | security.yml |
| `actions/download-artifact` | `v8.0.2` | `9000827ccba6bdab643e8b6fd33ac0654aef8333` | ci.yml, release.yml |
| `actions/upload-artifact` | `v7.0.2` | `cf430e030ddbb5b0abf93d22962f4752f3646cd9` | build-package.yml, ci.yml, fuzz-deep.yml, obs-upstream.yml, scorecard.yml, weekly-promotion.yml |
| `dtolnay/rust-toolchain` | `stable` | `89b12181fb390509a0842a86cc55eeb8eb928c1d` | build-package.yml, ci.yml, fuzz-deep.yml, nightly.yml, security.yml |
| `github/codeql-action/analyze` | `v4.39.1` | `9b3254085494b55a0f7c4c2b3d2d52cc712ce370` | security.yml |
| `github/codeql-action/autobuild` | `v4.39.1` | `9b3254085494b55a0f7c4c2b3d2d52cc712ce370` | security.yml |
| `github/codeql-action/init` | `v4.39.1` | `9b3254085494b55a0f7c4c2b3d2d52cc712ce370` | security.yml |
| `github/codeql-action/upload-sarif` | `v4.39.1` | `9b3254085494b55a0f7c4c2b3d2d52cc712ce370` | scorecard.yml |
| `ossf/scorecard-action` | `v2.5.0` | `fa7117db83c52d787ae5a89f350999701505d78e` | scorecard.yml |
| `signpath/github-action-submit-signing-request` | `v3.0` | `f6d04783b4569d051e0c80105fe66e82819d0092` | build-package.yml |
| `softprops/action-gh-release` | `v3.0.3` | `efb35369e0ad2afab669f228072c1b0d510eae64` | ci.yml, release.yml |
<!-- action-pins-table:end -->

> `dtolnay/rust-toolchain` is pinned to the `stable` **branch** (not a release
> tag): the SHA pins the action *code*, while the action's default
> `toolchain: stable` input still installs the latest stable Rust. It therefore
> has no semver comment.
>
> Because that pin tracks a rolling branch, it is the one pin **Dependabot
> cannot keep current**: Dependabot proposes updates for released versions,
> and the `stable` branch has no release, so the nightly stale-pin checker is
> the only thing that notices the tip moving. Upstream pushed `6bed0761` →
> `89b12181` on 2026-10-01 ("retry release-server checksum failures",
> `--force-non-host`), which reddened the nightly for several days until the
> bump landed — the second time in three weeks, the first costing 8+ days
> (#173). The checker now reports that as `branch-drift` and keeps it
> **informational**, because nothing is actually stale: the pinned SHA still
> fixes the action code that runs, and the compiler version comes from
> `rust-toolchain.toml` below, not from the branch. Bump it when convenient;
> pass `--fail-on-branch-drift` if you ever want the old strict behaviour back.
>
> When the checker reports drift on this action, verify the diff
>
> ```bash
> git clone -q --filter=blob:none --branch stable --single-branch \
>   https://github.com/dtolnay/rust-toolchain.git /tmp/rtc
> git -C /tmp/rtc log --oneline <old-sha>..<new-sha>
> git -C /tmp/rtc diff <old-sha> <new-sha> -- action.yml
> ```
>
> and confirm that no `inputs:` surface changed — the action is called with
> `components:` in every job, so an input rename would break all of them.
> Runtime changes to the `runs:` steps (retry loops, extra flags) are the
> normal reason a bump exists.
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
  (run by the blocking `stale_pins` job in `ci.yml` on every push/PR, and daily
  by the nightly workflow): fails on same-major updates we have not taken.
  Both invoke it with `--fail-on-major`, so **newer major versions also fail
  the check** instead of only being reported — a major gap surfaces on the open
  PR (and as a red Nightly run) rather than as a quiet warning. For a
  branch pin such as `dtolnay/rust-toolchain@stable` the comparison is against
  the branch tip, which moves on every upstream push; that is reported as its
  own `branch-drift` status and stays informational, because the pinned SHA
  still fixes the action code and Dependabot cannot cover a branch. Add
  `--fail-on-branch-drift` to make it fatal — the nightly deliberately does
  not, so an upstream action commit cannot redden the run on its own. The
  "is this fatal?" decision lives in `fatal_count()`, shared by the exit code
  and the Markdown renderer, so the step summary can never claim green while
  the job fails.
  `--json` emits a machine-readable result; `--comment` emits a compact Markdown
  notification that the nightly workflow publishes to the run's step summary.

### The checker runs as a blocking gate in both workflows

| Workflow | Job / step | Flags | Failure |
| --- | --- | --- | --- |
| `ci.yml` | `stale_pins` — `Compare pinned SHAs against upstream tags/branches` | `--fail-on-major --comment` | **fails the PR** (outdated within major, newer major, unresolvable ref) |
| `nightly.yml` | `Check for stale action pins` | `--fail-on-major --comment` | **fails the job** on schedule/dispatch |

`ci.yml` runs on every push and pull request, so an outdated pin reddens the
PR *while it is still open* instead of surfacing only at the next 02:17 UTC
nightly — this closed the gap where a release cut between Dependabot's last
run and the merge (e.g. `github/codeql-action` v4.39.1) landed on `develop`
before the nightly noticed. To make the gate enforce merges, mark the
`Stale Action Pins (gate)` check as a **required status check** on `develop`'s
branch protection.

Branch drift stays informational in both places: a rolling branch pin such as
`dtolnay/rust-toolchain@stable` moving upstream is reported but passes, because
the pinned SHA still fixes the action code and the compiler is pinned by
`rust-toolchain.toml`. Neither invocation passes `--fail-on-branch-drift`.

Both invocations use `set -o pipefail` with a `| tee -a "$GITHUB_STEP_SUMMARY"`
pipeline, so the checker's exit status survives the pipe and the step summary
is written even when the checker itself fails.

`rivulet-core/tests/ci_pinning.rs` asserts the gate
(`stale_pin_checker_blocks_prs_via_a_dedicated_gate_job`): the `stale_pins`
job must exist, run `check-action-pins.py --fail-on-major`, keep
`set -o pipefail` + the step summary, take no `continue-on-error` (so it can
redden a PR), never opt into `--fail-on-branch-drift`, and the retired
advisory step must be gone.
  Compound actions (`owner/repo/path/to/action`) are resolved against the
  `owner/repo` repository that owns the tags, so the sub-path is stripped before
  comparing SHAs.
- `.github/dependabot.yml` — the `github-actions` entry that proposes the
  updates in the first place.
