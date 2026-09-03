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

---

## Release Strategy

### Two-Channel Architecture

Rivulet uses a **two-channel** release model:

| Channel | Tag format | Trigger | Example |
| --- | --- | --- | --- |
| **Alpha** | `v0.X.Y-alpha.N` | Every `feat:`, `fix:`, `build:`, `chore:` or `ci:` push to `develop` | `v0.21.0-alpha.1` |
| **Beta / RC / Stable** | `vX.Y.Z` | Manually set tag (no suffix) | `v1.0.0` |

### Alpha Channel (Automatic)

Every releasable commit pushed to `develop` automatically produces an alpha release:

| Commit type | SemVer bump | Example |
| --- | --- | --- |
| `fix(...)` | Patch | `0.20.0` → `0.20.1-alpha.1` |
| `feat(...)` | Minor | `0.20.0` → `0.21.0-alpha.1` |
| `feat!(...)` or `BREAKING CHANGE` | Major | `0.x.y` → `1.0.0-alpha.1` |
| `build(...)`, `chore(...)` or `ci(...)` | Patch | `0.20.0` → `0.20.1-alpha.1` |

The workflow (`release.yml`) handles everything:
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
