#!/usr/bin/env bash
# Detect and backfill orphaned alpha release tags.
#
# An "orphaned" tag is a version tag (vX.Y.Z-alpha.N) that the release
# automation (release.yml, "Determine version & tag" job) created and pushed,
# but that never got a formal GitHub Release: the "Create Alpha GitHub
# Release" job is skipped whenever at least one platform build failed.
#
# Usage:
#   scripts/backfill-releases.sh --check            # list orphaned tags, no changes
#   scripts/backfill-releases.sh                    # backfill all orphaned alpha tags
#   scripts/backfill-releases.sh v0.18.0-alpha.1    # backfill specific tag(s)
#   scripts/backfill-releases.sh --dry-run [tags]   # show the plan, change nothing
#
# The script is idempotent and safe to re-run: already-published tags are
# skipped, draft releases are resumed, downloads are gated on complete file
# sets and uploads use --clobber. A lock file prevents concurrent instances
# (two parallel runs once corrupted a draft release).
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${RIVULET_BACKFILL_WORK:-/tmp/backfill-releases}"
LOCK="$WORK/.lock"
LOG="$WORK/backfill.log"
mkdir -p "$WORK"

# --- lock (flock where available, mkdir fallback for Git Bash) ---------------
if command -v flock >/dev/null 2>&1; then
  exec 9>"$LOCK"
  if ! flock -n 9; then
    echo "Another backfill instance is already running (lock: $LOCK)." >&2
    exit 1
  fi
else
  LOCKDIR="$WORK/.lockdir"
  if ! mkdir "$LOCKDIR" 2>/dev/null; then
    echo "Another backfill instance is already running (lock: $LOCKDIR)." >&2
    exit 1
  fi
  trap 'rmdir "$LOCKDIR" 2>/dev/null || true' EXIT
fi

log() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$LOG"; }

CHECK=false
DRY=false
TAGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) CHECK=true ;;
    --dry-run) DRY=true ;;
    --help|-h)
      grep '^#' "$0" | sed 's/^# \{0,1\}//' | sed -n '2,15p'
      exit 0
      ;;
    -*) echo "Unknown option: $1" >&2; exit 2 ;;
    *) TAGS+=("$1") ;;
  esac
  shift
done

# --- tag discovery ------------------------------------------------------------
alpha_tags() {
  git ls-remote --tags origin 2>/dev/null \
    | grep -v '\^{}' \
    | sed 's|.*refs/tags/||' \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+-alpha\.[0-9]+$' || true
}

published_tags() {
  gh release list --limit 200 --json tagName,isDraft -q \
    '.[] | select(.isDraft == false) | .tagName' 2>/dev/null || true
}

draft_tags() {
  gh release list --limit 200 --json tagName,isDraft -q \
    '.[] | select(.isDraft == true) | .tagName' 2>/dev/null || true
}

# Resolve the workflow run that produced a tag. The tag points at the
# "chore(release): prepare vX" commit; its parent is the head commit of the
# triggering push, which equals the run's headSha.
run_for_tag() {
  local tag="$1"
  local feat_sha
  feat_sha="$(git rev-parse "$tag^" 2>/dev/null || echo '')"
  if [[ -z "$feat_sha" ]]; then
    echo ""
    return
  fi
  gh run list --workflow=release.yml --limit 200 --json databaseId,headSha \
    -q ".[] | select(.headSha==\"$feat_sha\") | .databaseId" 2>/dev/null | head -1
}

# --- artifact handling ---------------------------------------------------------
# Each line: <artifact-name>:<file1>:<file2>:...
ARTIFACTS=(
  "rivulet-linux-x86_64-alpha:rivulet-gui:rivulet-linux-x86_64.AppImage"
  "rivulet-windows-x86_64-alpha:rivulet-gui.exe:rivulet-windows-x86_64-portable.zip:rivulet-windows-x86_64.msi"
  "rivulet-macos-aarch64-alpha:rivulet-macos-aarch64.dmg"
)

# Download one artifact unless all its expected files are already complete.
# Returns 0 when the files are complete afterwards (missing artifact in the
# run is expected and not an error — that platform simply never built).
download_artifact() {
  local dir="$1" run="$2" art="$3"
  shift 3
  local missing=() still=() f
  for f in "$@"; do [[ -f "$dir/$f" ]] || missing+=("$f"); done
  if [[ ${#missing[@]} -eq 0 ]]; then
    log "  $art: files already complete"
    return 0
  fi
  log "  $art: downloading (missing: ${missing[*]})"
  if ! gh run download "$run" -n "$art" -D "$dir" >/dev/null 2>&1; then
    log "  $art: no artifact in run (platform did not build)"
    return 0
  fi
  for f in "$@"; do [[ -f "$dir/$f" ]] || still+=("$f"); done
  if [[ ${#still[@]} -gt 0 ]]; then
    log "  WARNING: $art still missing after download: ${still[*]} (partial download?)"
    return 1
  fi
  return 0
}

# --- --check mode ---------------------------------------------------------------
if [[ "$CHECK" == true ]]; then
  PUBLISHED="$(published_tags)"
  DRAFTS="$(draft_tags)"
  echo "=== Orphaned alpha tags (no published GitHub release) ==="
  local_orphans=()
  while IFS= read -r tag; do
    [[ -z "$tag" ]] && continue
    if grep -qx "$tag" <<<"$PUBLISHED"; then
      continue
    fi
    if grep -qx "$tag" <<<"$DRAFTS"; then
      echo "  $tag  (draft — upload in progress)"
    else
      echo "  $tag"
    fi
  done < <(alpha_tags)
  echo "(end of list)"
  exit 0
fi

# --- backfill mode ---------------------------------------------------------------
if [[ ${#TAGS[@]} -eq 0 ]]; then
  mapfile -t TAGS < <(alpha_tags)
fi
PUBLISHED="$(published_tags)"
DRAFTS="$(draft_tags)"

for tag in "${TAGS[@]}"; do
  version="${tag#v}"
  dir="$WORK/$tag"
  log "================================================"
  log "TAG $tag"

  if grep -qx "$tag" <<<"$PUBLISHED" && ! grep -qx "$tag" <<<"$DRAFTS"; then
    log "  already published — skip"
    continue
  fi
  if grep -qx "$tag" <<<"$DRAFTS"; then
    log "  draft exists — will resume the upload"
  fi

  run="$(run_for_tag "$tag")"
  if [[ -z "$run" ]]; then
    log "  no release workflow run found for this tag — skip"
    continue
  fi
  log "  original run: https://github.com/thoser666/rivulet/actions/runs/$run"

  mkdir -p "$dir"
  for entry in "${ARTIFACTS[@]}"; do
    IFS=: read -r art files <<< "$entry"
    # shellcheck disable=SC2206
    flist=(${files//:/ })
    if [[ "$DRY" == true ]]; then
      # Dry run: only report what a real run would download.
      missing_dry=()
      for f in "${flist[@]}"; do [[ -f "$dir/$f" ]] || missing_dry+=("$f"); done
      if [[ ${#missing_dry[@]} -gt 0 ]]; then
        log "  $art: would download (missing: ${missing_dry[*]})"
      fi
      continue
    fi
    download_artifact "$dir" "$run" "$art" "${flist[@]}" || true
  done

  # Determine which platforms are present to build the release notes.
  present=()
  for f in "$dir/rivulet-linux-x86_64.AppImage"; do [[ -f "$f" ]] && present+=("Linux"); done
  for f in "$dir/rivulet-windows-x86_64.msi"; do [[ -f "$f" ]] && present+=("Windows"); done
  for f in "$dir/rivulet-macos-aarch64.dmg"; do [[ -f "$f" ]] && present+=("macOS"); done
  absent=()
  for p in Linux Windows macOS; do
    [[ " ${present[*]} " == *" $p "* ]] || absent+=("$p")
  done

  if [[ ${#present[@]} -eq 0 ]]; then
    log "  no artifacts available for this tag — skip"
    continue
  fi

  log "  platforms available: ${present[*]}"

  if [[ "$DRY" == true ]]; then
    log "  DRY RUN — would create release with:"
    for f in "$dir"/rivulet-* "$REPO_DIR"/docs/opengraph.png "$REPO_DIR"/docs/social-preview.png; do
      [[ -f "$f" ]] && log "    - $(basename "$f")"
    done
    continue
  fi

  notes=$(cat <<EOF
Backfilled release for tag \`$tag\`.

The original release build did not publish a GitHub Release (at least one
platform build failed). This release was created retroactively from the
artifacts that **did** build successfully in run [$run](https://github.com/thoser666/rivulet/actions/runs/$run).

**Missing platforms:** ${absent[*]:-none}

- Tag: \`$tag\`
- Original workflow run: https://github.com/thoser666/rivulet/actions/runs/$run
EOF
)

  if ! gh release view "$tag" >/dev/null 2>&1; then
    log "  creating draft release"
    gh release create "$tag" --draft --prerelease \
      --title "Rivulet $version" --notes "$notes" || log "  create failed"
  fi

  for f in \
    "$dir/rivulet-gui" \
    "$dir/rivulet-linux-x86_64.AppImage" \
    "$dir/rivulet-gui.exe" \
    "$dir/rivulet-windows-x86_64-portable.zip" \
    "$dir/rivulet-windows-x86_64.msi" \
    "$dir/rivulet-macos-aarch64.dmg" \
    "$REPO_DIR/docs/opengraph.png" \
    "$REPO_DIR/docs/social-preview.png"; do
    if [[ -f "$f" ]]; then
      log "  uploading $(basename "$f")"
      gh release upload "$tag" "$f" --clobber || log "  upload failed: $f"
    fi
  done

  log "  publishing $tag"
  gh release edit "$tag" --draft=false || log "  publish failed for $tag"
  log ">>> $tag done"
done

log "ALL DONE"
