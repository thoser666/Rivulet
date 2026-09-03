#!/usr/bin/env bash
# Generate Markdown release notes from the commits since the previous tag.
#
# The alpha Release workflow builds the notes body from the ACTUAL commits
# between the previous tag and the release tip (excluding the
# "chore(release): prepare vX.Y.Z-alpha.N" bump commit that sits at HEAD),
# grouped by Conventional Commit type:
#
#   feat      → ### Features
#   fix       → ### Bug fixes
#   perf      → ### Performance
#   docs      → ### Documentation
#   build     → ### Build & packaging
#   ci        → ### CI
#   test      → ### Tests
#   refactor  → ### Refactoring
#   chore     → ### Housekeeping
#   anything else → ### Other changes
#
# Usage:
#   bash scripts/generate-release-notes.sh              # print notes to stdout
#   bash scripts/generate-release-notes.sh --self-test  # run the fixture tests
#
# Run from the repository root or any subdirectory. The release tip must
# carry the version-bump commit at HEAD, exactly as scripts/release-branch.sh
# publishes it; the previous tag is the newest tag reachable from HEAD^
# (falls back to the first commit when no tag exists yet).
set -euo pipefail

# ---------------------------------------------------------------------------
# Core generator. generate_notes <repo-dir> prints the grouped notes for
# "<previous-tag>..HEAD^" of the given git repository to stdout.
# ---------------------------------------------------------------------------
generate_notes() {
  local repo="$1"
  local prev range type rest subject out
  local -A headings
  local -A entries

  if git -C "$repo" describe --tags --abbrev=0 HEAD^ >/dev/null 2>&1; then
    prev="$(git -C "$repo" describe --tags --abbrev=0 HEAD^)"
  else
    # No previous tag yet (very first release): start at the first commit.
    prev="$(git -C "$repo" rev-list --max-parents=0 HEAD)"
  fi
  # Up to HEAD, not HEAD^: the release tip only carries the version-bump
  # commit when the version job found changes, so pinning the range to HEAD^
  # would drop the triggering commit on retries/manual runs. The bump commit
  # itself is excluded by subject below.
  range="${prev}..HEAD"

  headings=(
    [feat]="### Features"
    [fix]="### Bug fixes"
    [perf]="### Performance"
    [docs]="### Documentation"
    [build]="### Build & packaging"
    [ci]="### CI"
    [test]="### Tests"
    [refactor]="### Refactoring"
    [chore]="### Housekeeping"
    [other]="### Other changes"
  )

  # Collect one bullet per commit under its type section. The conventional
  # prefix (type, optional scope and the colon) is stripped for a clean
  # bullet; the type section already groups the change, so e.g.
  # "fix(updater): verify checksums" becomes "- verify checksums" under
  # "### Bug fixes".
  # Held in a variable so bash parses the ERE operators literally inside the
  # [[ =~ ]] conditional (bare parens/pipes there would be shell syntax).
  local conventional_re='^(feat|fix|perf|docs|build|ci|test|refactor|chore)(\([^)]*\))?!?:[[:space:]]*(.*)$'
  # `--pretty=format:'%s'` emits no trailing newline, which would make the
  # final `read` fail at EOF and silently drop the oldest commit in range —
  # append '%n' so every subject line is terminated (the empty trailing line
  # is skipped below).
  while IFS= read -r subject; do
    [ -n "$subject" ] || continue
    # Skip the version-bump commit(s) the release pipeline itself creates
    # ("chore(release): prepare vX.Y.Z-alpha.N") — regardless of whether
    # they sit at HEAD (normal path) or further down (retry/manual runs).
    case "$subject" in
      "chore(release): prepare "*) continue ;;
    esac
    type="other"
    rest="$subject"
    if [[ "$subject" =~ $conventional_re ]]; then
      type="${BASH_REMATCH[1]}"
      rest="${BASH_REMATCH[3]}"
    fi
    entries["$type"]+="- ${rest}"$'\n'
  done < <(git -C "$repo" log --no-merges --pretty=format:'%s%n' "$range" 2>/dev/null || true)

  out=""
  for type in feat fix perf docs build ci test refactor chore other; do
    if [[ -n "${entries[$type]:-}" ]]; then
      out+="${headings[$type]}"$'\n'
      out+="${entries[$type]}"
      out+=$'\n'
    fi
  done
  printf '%s' "$out"
}

# ---------------------------------------------------------------------------
# Self-test: builds a fixture repository with one commit per section and
# asserts the generator groups them correctly and excludes the tagged
# baseline as well as the release-prep commit at HEAD.
# ---------------------------------------------------------------------------
self_test() {
  local work status=0 out expect

  work="$(mktemp -d)"
  trap 'rm -rf "$work"' RETURN

  git -C "$work" init -q
  git -C "$work" config user.email "test@example.com"
  git -C "$work" config user.name "Test"
  git -C "$work" commit --allow-empty -q -m "chore: seed the repository"
  git -C "$work" tag -a v0.1.0-alpha.1 -m "Release v0.1.0-alpha.1"

  git -C "$work" commit --allow-empty -q -m "feat(ui): add the stream tab"
  git -C "$work" commit --allow-empty -q -m "fix(updater): verify checksums before install"
  git -C "$work" commit --allow-empty -q -m "docs: document the release notes"
  git -C "$work" commit --allow-empty -q -m "ci(release): serialize alpha runs"
  git -C "$work" commit --allow-empty -q -m "chore(release): prepare v0.2.0-alpha.1"

  out="$(generate_notes "$work")"

  for expect in \
    "### Features" \
    "- add the stream tab" \
    "### Bug fixes" \
    "- verify checksums before install" \
    "### Documentation" \
    "- document the release notes" \
    "### CI" \
    "- serialize alpha runs"; do
    if ! grep -qF -- "$expect" <<<"$out"; then
      echo "FAIL: generated notes miss '$expect'" >&2
      status=1
    fi
  done
  # The release-prep commit at HEAD and everything at/before the previous
  # tag must never appear in the notes.
  if grep -qF -- "prepare v0.2.0" <<<"$out"; then
    echo "FAIL: the release-prep commit must not appear in the notes" >&2
    status=1
  fi
  if grep -qF -- "seed the repository" <<<"$out"; then
    echo "FAIL: commits at/before the previous tag must not appear" >&2
    status=1
  fi

  if [ "$status" -eq 0 ]; then
    echo "release-notes self-test passed"
  else
    echo "--- generated notes ---" >&2
    echo "$out" >&2
  fi
  return "$status"
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit $?
fi

# Normal mode: print the notes for the given repository (default: the
# checkout that contains this script). An explicit argument lets other tools
# (e.g. scripts/check-release-notes.py) generate for a fixture repository.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
generate_notes "${1:-$ROOT}"
