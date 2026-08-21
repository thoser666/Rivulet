#!/usr/bin/env bash
# Generates/updates CHANGELOG.md from the Conventional Commits since the
# last tag.
#
# Usage: scripts/update-changelog.sh <version>
set -euo pipefail

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  echo "Usage: $0 <version>" >&2
  exit 1
fi

LAST_TAG="$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || echo '')"

SECTION="## [$VERSION] - $(date -u +%Y-%m-%d)"
BODY=""
if [[ -n "$LAST_TAG" ]]; then
  BODY="$(git log "$LAST_TAG"..HEAD --pretty=format:'- %s' 2>/dev/null || true)"
fi
if [[ -z "$BODY" ]]; then
  BODY="- Initial release or no new commits."
fi

CHANGELOG="CHANGELOG.md"
HEADER="# Changelog"
ENTRY="$SECTION
$BODY"

if [[ -f "$CHANGELOG" ]]; then
  # Keep the existing content without the header and leading blank lines,
  # then prepend the new section separated by a blank line.
  #
  # NOTE: never run `sed 's/^/\n/'` on the rest — it prepends a newline to
  # *every* line, doubling the file on each release (fixed in v0.25.0).
  REST="$(sed '1{/^# Changelog/d;}' "$CHANGELOG" | sed '/./,$!d')"
  printf '%s\n\n%s\n\n%s\n' "$HEADER" "$ENTRY" "$REST" > "$CHANGELOG"
else
  printf '%s\n\n%s\n' "$HEADER" "$ENTRY" > "$CHANGELOG"
fi

# Defensive guard: the generated file must stay small. This catches
# regressions like the pre-v0.25.0 doubling bug before a >100 MB file
# breaks the push to GitHub (hard limit 100 MB).
SIZE_BYTES="$(wc -c < "$CHANGELOG")"
MAX_BYTES=5242880 # 5 MB
if (( SIZE_BYTES > MAX_BYTES )); then
  echo "ERROR: $CHANGELOG is $SIZE_BYTES bytes (>${MAX_BYTES}) — aborting." >&2
  echo "The changelog generation has a bug; fix it instead of pushing a huge file." >&2
  exit 1
fi

echo "CHANGELOG.md updated (version $VERSION, $SIZE_BYTES bytes)."
