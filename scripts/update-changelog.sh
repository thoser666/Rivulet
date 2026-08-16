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
  # Keep the existing content without the header and prepend the new section.
  REST="$(sed '1{/^# Changelog/d;}' "$CHANGELOG" | sed '/./,$!d' | sed 's/^/\n/')"
  printf '%s\n\n%s\n%s\n' "$HEADER" "$ENTRY" "$REST" > "$CHANGELOG"
else
  printf '%s\n\n%s\n' "$HEADER" "$ENTRY" > "$CHANGELOG"
fi

echo "CHANGELOG.md updated (version $VERSION)."
