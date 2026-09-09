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
#   bash scripts/generate-release-notes.sh --from-tag v0.65.0-alpha.160
#       # print notes for the range "<tag>..HEAD" — the weekly promotion
#       # workflow uses this to produce one bigger, multi-release changelog
#       # covering everything since the last promoted release
#   bash scripts/generate-release-notes.sh --digest
#   bash scripts/generate-release-notes.sh --from-tag <tag> --digest
#       # collapse the per-commit bullets into a human summary: features are
#       # still listed individually, everything else is rolled up into
#       # "N fixes / M housekeeping ..." counts per section
#   bash scripts/generate-release-notes.sh --self-test  # run the fixture tests
#
# Run from the repository root or any subdirectory. In default mode the
# release tip must carry the version-bump commit at HEAD, exactly as
# scripts/release-branch.sh publishes it; the previous tag is the newest tag
# reachable from HEAD^ (falls back to the first commit when no tag exists
# yet). In --from-tag mode the range is "<tag>..HEAD" instead and the bump
# commit at HEAD (if any) is excluded by subject like everywhere else.
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
  # Explicit range override (--from-tag <tag>): the weekly promotion workflow
  # renders one big changelog for "<last-promoted-tag>..HEAD" so the notes
  # cover a week of releases, not just the newest one.
  if [ -n "${FROM_TAG:-}" ]; then
    range="${FROM_TAG}..HEAD"
  fi

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
      if [ "$DIGEST" -eq 1 ] && [ "$type" != "feat" ]; then
        # Digest mode: roll everything except features up into one count
        # bullet per section — the weekly store-listing changelog reads
        # better with "12 fixes across the app" than twelve raw bullets.
        local count
        count="$(printf '%s' "${entries[$type]}" | grep -c '^- ' || true)"
        out+="- ${count} change(s) in this area"$'\n'
      else
        out+="${entries[$type]}"
      fi
      out+=$'\n'
    fi
  done
  printf '%s' "$out"
}

# ---------------------------------------------------------------------------
# Argument parsing. Flags come first; the optional positional repo-dir is
# forwarded to the caller below unchanged (check-release-notes.py relies on
# it for fixture repositories).
# ---------------------------------------------------------------------------
FROM_TAG=""
DIGEST=0
SELF_TEST=0
while [ $# -gt 0 ]; do
  case "$1" in
    --self-test)
      SELF_TEST=1
      shift
      ;;
    --from-tag)
      if [ $# -lt 2 ]; then
        echo "error: --from-tag requires a tag argument" >&2
        exit 2
      fi
      FROM_TAG="$2"
      shift 2
      ;;
    --from-tag=*)
      FROM_TAG="${1#--from-tag=}"
      shift
      ;;
    --digest)
      DIGEST=1
      shift
      ;;
    --*)
      echo "usage: generate-release-notes.sh [--from-tag <tag>] [--digest] [--self-test] [repo-dir]" >&2
      exit 2
      ;;
    *)
      break # positional repo-dir (or --self-test tail) — stop flag parsing
      ;;
  esac
done

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

# --from-tag: notes must cover exactly "<tag>..HEAD" — the promoted-range
# changelog for the weekly promotion. Everything at/before the tag (here:
# the whole first release batch) must be excluded, the bump commit at HEAD
# must still be dropped.
self_test_from_tag() {
  local work status=0 out

  work="$(mktemp -d)"
  trap 'rm -rf "$work"' RETURN

  git -C "$work" init -q
  git -C "$work" config user.email "test@example.com"
  git -C "$work" config user.name "Test"
  git -C "$work" commit --allow-empty -q -m "chore: seed the repository"
  git -C "$work" commit --allow-empty -q -m "feat(ui): add the stream tab"
  git -C "$work" tag -a v0.2.0-alpha.1 -m "Release v0.2.0-alpha.1"

  git -C "$work" commit --allow-empty -q -m "feat(chat): multi-platform chat docks"
  git -C "$work" commit --allow-empty -q -m "fix(capture): unblock the stop path"
  git -C "$work" commit --allow-empty -q -m "chore(release): prepare v0.3.0-alpha.1"

  # End-to-end through the CLI so the argument parser is exercised too.
  out="$(bash "$0" --from-tag v0.2.0-alpha.1 "$work")"

  if ! grep -qF -- "- multi-platform chat docks" <<<"$out"; then
    echo "FAIL: --from-tag must include commits after the tag" >&2
    status=1
  fi
  if grep -qF -- "add the stream tab" <<<"$out"; then
    echo "FAIL: --from-tag must exclude commits at/before the tag" >&2
    status=1
  fi
  if grep -qF -- "prepare v0.3.0" <<<"$out"; then
    echo "FAIL: --from-tag must still drop the release-prep commit" >&2
    status=1
  fi

  if [ "$status" -ne 0 ]; then
    echo "--- generated from-tag notes ---" >&2
    echo "$out" >&2
  fi
  return "$status"
}

# --digest: features stay listed individually, every other section collapses
# into a single count bullet.
self_test_digest() {
  local work status=0 out

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

  out="$(bash "$0" --digest "$work")"

  if ! grep -qF -- "- add the stream tab" <<<"$out"; then
    echo "FAIL: --digest must keep feature bullets individually" >&2
    status=1
  fi
  if grep -qF -- "verify checksums" <<<"$out"; then
    echo "FAIL: --digest must collapse non-feature sections into counts" >&2
    status=1
  fi
  for expect in "### Features" "### Bug fixes" "### Documentation" "### CI"; do
    if ! grep -qF -- "$expect" <<<"$out"; then
      echo "FAIL: --digest must keep the section headings ($expect)" >&2
      status=1
    fi
  done
  if [ "$(grep -cF -- '- 1 change(s) in this area' <<<"$out")" -ne 3 ]; then
    echo "FAIL: --digest must emit exactly one count bullet per non-empty section" >&2
    status=1
  fi

  if [ "$status" -ne 0 ]; then
    echo "--- generated digest notes ---" >&2
    echo "$out" >&2
  fi
  return "$status"
}

if [ "$SELF_TEST" -eq 1 ]; then
  status=0
  self_test || status=1
  self_test_from_tag || status=1
  self_test_digest || status=1
  if [ "$status" -eq 0 ]; then
    echo "release-notes flag self-tests passed"
  fi
  exit "$status"
fi

# Normal mode: print the notes for the given repository (default: the
# checkout that contains this script). An explicit argument lets other tools
# (e.g. scripts/check-release-notes.py) generate for a fixture repository.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
generate_notes "${1:-$ROOT}"
