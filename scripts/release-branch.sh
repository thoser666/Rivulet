#!/usr/bin/env bash
# Reconcile an alpha release branch so it ALWAYS points at the current source
# HEAD (which already carries the freshly-committed version bump) — never at a
# stale remote tip.
#
# Why: the alpha Release workflow builds a versioned package from a temporary
# branch `release/$TAG` (consumed by build-package.yml at `ref: release_branch`).
# An earlier implementation reused a stale remote tip whenever that branch had
# diverged from HEAD. That made the build compile OUTDATED code that predated a
# fix on develop — for example a non-Windows compile error that had already been
# repaired on `develop`, but the release branch still contained the broken bin.
# To guarantee a release always reflects the *current* develop source, this
# script publishes the freshly-built HEAD (source + version bump) onto the
# release branch, overwriting any divergent/stale tip. `--force-with-lease`
# keeps concurrent runs safe: an overwrite only applies when the remote tip is
# still the one we just fetched.
#
# Usage: scripts/release-branch.sh <release-branch>
set -euo pipefail

BRANCH="${1:-}"
if [[ -z "$BRANCH" ]]; then
  echo "Usage: $0 <release-branch>" >&2
  exit 2
fi

# Refresh the remote-tracking ref so --force-with-lease compares against the
# true current remote tip. A missing branch simply leaves no tracking ref.
git fetch origin "$BRANCH" 2>/dev/null || true

if git show-ref --verify --quiet "refs/remotes/origin/$BRANCH"; then
  REMOTE_TIP="$(git rev-parse "origin/$BRANCH")"
  if git merge-base --is-ancestor "$REMOTE_TIP" HEAD; then
    # Remote is at or behind our HEAD: a normal push fast-forwards it. This is
    # the common "same version, branch already exists" retry path.
    echo "release branch $BRANCH is at or behind HEAD; fast-forwarding." >&2
    git push origin "HEAD:$BRANCH"
  else
    # Remote has diverged (or is ahead) with code that is not our current
    # source. The version job is authoritative: it always carries the latest
    # develop plus a fresh version bump, so the stale tip must be overwritten.
    echo "release branch $BRANCH diverged; overwriting with current HEAD." >&2
    git push --force-with-lease origin "HEAD:$BRANCH"
  fi
else
  echo "creating new release branch $BRANCH." >&2
  git push origin "HEAD:$BRANCH"
fi
