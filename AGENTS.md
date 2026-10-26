# Agent Instructions

Guidance for AI agents (and helpful for human contributors) working in this
repository.

## Language

**Write code comments, commit messages, PR titles and PR bodies in English.**
Code is read by a global audience; commit history and PRs are the project's
public record (see `CONTRIBUTING.md` for the Conventional Commits format).

Two deliberate exceptions:

- **`CHANGELOG.md` is written in English** — all entries follow the English convention so the changelog stays consistent.
- **Issue discussions** may be written in whichever language the thread is
  already in; do not switch an existing German thread to English.

Merged commit subjects are history — never rewrite them to translate.

## Branching & reviews

- Open pull requests against `develop` (not `main`/`master`).
- `develop` is protected by a ruleset: required status checks include
  `CI`, `Security`, `Pinning-Tests` and `Stale Action Pins (gate)`.
  A PR with an outdated action pin fails the gate by design — take the bump
  (see `docs/ci-action-pins.md`) instead of bypassing it.
- Commits need a `Signed-off-by` line (DCO): use `git commit -s`.

## CI & pinning rules the tests enforce

- All third-party GitHub Actions are pinned to full 40-char SHAs with a
  `# vX.Y.Z` comment; `rivulet-core/tests/ci_pinning.rs` fails otherwise.
  After bumping a pin, run `scripts/generate-action-pins.py` and update the
  `PINNED_ACTIONS` table in the same PR.
- `CHANGELOG.md` gets an entry for every user-visible change (English, under
  `## [Unreleased]`); tests assert core surfaces stay documented.
- Renovate owns Cargo updates; Dependabot owns GitHub Actions pins only.
  Do not add a `cargo` ecosystem to `.github/dependabot.yml`.

## Local verification

- `scripts/run.cmd` (Windows) drives the usual check loop; CI mirrors it.
- `python3 scripts/check-action-pins.py --fail-on-major` reproduces the
  gate locally.
