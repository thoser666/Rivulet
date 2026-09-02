#!/usr/bin/env bash
# Unit tests for scripts/release-branch.sh.
#
# The script reconciles an alpha release branch onto the current source HEAD.
# These tests simulate a remote + working clone (as the GitHub Actions version
# job sees them) and assert that after running the script the remote release
# branch ALWAYS points at the current local HEAD — including the case that
# previously failed: a release branch left on stale code after it had diverged.
#
# Usage: bash scripts/test-release-branch.sh
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-branch.sh"
WORK="$(mktemp -d)"
REMOTE_BARE="${WORK}/remote.git"
CLONE="${WORK}/clone"
cleanup() {
  rm -rf "$WORK"
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# --- helpers --------------------------------------------------------------

# Reset the fixture: a bare remote with an initial commit, plus a fresh clone.
setup_fixture() {
  rm -rf "$WORK"
  mkdir -p "$WORK"
  git init --bare -q "$REMOTE_BARE"

  git clone -q "$REMOTE_BARE" "$CLONE"
  pushd "$CLONE" >/dev/null
  git config user.email "test@example.com"
  git config user.name "Test"
  echo "base" > file.txt
  git add file.txt
  git commit -q -m "base"
  git checkout -q -b main
  git push -q origin main
  popd >/dev/null
}

# Create N commits on top of the current branch, tagged with a label file.
make_commits() {
  pushd "$CLONE" >/dev/null
  local label="$1"; shift
  local n="$1"
  for _ in $(seq 1 "$n"); do
    echo "$label $(date +%s%N)" >> "$label.txt"
    git add "$label.txt"
    git commit -q -m "chore($label): bump"
  done
  popd >/dev/null
}

# Publish the current "$CLONE" main HEAD to a remote branch WITHOUT touching
# the local checkouts that release-branch.sh will operate on.
publish_remote_branch() {
  pushd "$CLONE" >/dev/null
  local branch="$1"
  git push -q origin "HEAD:$branch"
  popd >/dev/null
}

# Run release-branch.sh from "$CLONE" against the given remote branch.
run_reconcile() {
  pushd "$CLONE" >/dev/null
  bash "$SCRIPT" "$1"
  popd >/dev/null
}

remote_branch_sha() {
  git --git-dir="$REMOTE_BARE" rev-parse "refs/heads/$1" 2>/dev/null || true
}

# --- tests ----------------------------------------------------------------

echo "T1: fresh branch is created and points at current HEAD"
setup_fixture
local_head_before="$(git -C "$CLONE" rev-parse HEAD)"
run_reconcile "release/v0.65.0-alpha.1"
[ "$(remote_branch_sha "release/v0.65.0-alpha.1")" = "$local_head_before" ] \
  || fail "fresh branch did not point at current HEAD"

echo "T2: branch behind HEAD fast-forwards to current HEAD"
setup_fixture
make_commits stale 1
# Publish the older main as the release branch, then advance main on top.
publish_remote_branch "release/v0.65.0-alpha.2"
make_commits newer 2
new_head="$(git -C "$CLONE" rev-parse HEAD)"
run_reconcile "release/v0.65.0-alpha.2"
[ "$(remote_branch_sha "release/v0.65.0-alpha.2")" = "$new_head" ] \
  || fail "behind branch did not fast-forward to current HEAD"

# Build a truly DIVERGED release branch: the release branch carries the version
# bump on top of an OLD source snapshot, while the working clone's HEAD has NEW
# source commits that the release branch does not contain (neither is an
# ancestor of the other) — the exact scenario that reproduced the stale-code
# build bug.
setup_diverged_release_branch() {
  local branch="$1"
  pushd "$CLONE" >/dev/null
  # Side branch: "old source + version bump" -> published as the release branch.
  git checkout -q -b "tmp-old-$branch"
  echo "old-snapshot $(date +%s%N)" > old.txt
  git add old.txt
  git commit -q -m "chore(release): prepare $branch"
  git push -q origin "HEAD:$branch"
  git checkout -q main
  # Now advance main with NEW source (the fixed current code).
  echo "new-source $(date +%s%N)" > new.txt
  git add new.txt
  git commit -q -m "fix: apply latest fix"
  git branch -qD "tmp-old-$branch"
  # Sanity: truly diverged — main is NOT an ancestor of the release branch.
  if git merge-base --is-ancestor HEAD "refs/remotes/origin/$branch"; then
    echo "fixture error: main is ancestor of $branch" >&2
    exit 1
  fi
  popd >/dev/null
}

echo "T3 (regression): truly diverged/stale branch is overwritten with current HEAD"
setup_fixture
setup_diverged_release_branch "release/v0.65.0-alpha.55"
new_head="$(git -C "$CLONE" rev-parse HEAD)"
run_reconcile "release/v0.65.0-alpha.55"
[ "$(remote_branch_sha "release/v0.65.0-alpha.55")" = "$new_head" ] \
  || fail "diverged branch was NOT overwritten with current HEAD (the staleness bug)"

echo "T4: ahead branch is overwritten with current HEAD"
setup_fixture
# Simulate a release branch that is AHEAD of main (carries an extra commit).
make_commits ahead 1
publish_remote_branch "release/v0.65.0-alpha.9"
# Rewind main so it is behind the remote release branch.
git -C "$CLONE" reset -q --hard HEAD~1
main_head="$(git -C "$CLONE" rev-parse HEAD)"
# Sanity: main is an ancestor of the (ahead) release branch but not equal.
[ "$(remote_branch_sha "release/v0.65.0-alpha.9")" != "$main_head" ] \
  || fail "fixture did not create an ahead branch as expected"
git -C "$CLONE" merge-base --is-ancestor HEAD "refs/remotes/origin/release/v0.65.0-alpha.9" \
  || fail "fixture error: main is not an ancestor of the ahead branch"
run_reconcile "release/v0.65.0-alpha.9"
[ "$(remote_branch_sha "release/v0.65.0-alpha.9")" = "$main_head" ] \
  || fail "ahead branch was not overwritten with current HEAD"

echo "T5: missing argument exits non-zero"
if bash "$SCRIPT" >/dev/null 2>&1; then
  fail "script with no args should exit non-zero"
fi

echo "ALL PASS: release-branch.sh reconciliation is robust."
