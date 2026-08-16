#!/usr/bin/env bash
# Determines the next release version according to Semantic Versioning from
# the Conventional Commits since the last tag, and generates the version
# string for the next release (beta/rc/stable).
#
# Output: <next-version>
set -euo pipefail

# Determine the last tag (v[0-9]*); without an existing tag we start at
# 0.0.0 and analyze the complete history.
LAST_TAG="$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)"

if [[ -n "$LAST_TAG" ]]; then
  echo "Last tag: $LAST_TAG" >&2
  BASE_VERSION="${LAST_TAG#v}"
  BASE_VERSION="${BASE_VERSION%%-*}"
  LOG_RANGE="$LAST_TAG..HEAD"
else
  echo "No previous tag found - analyzing the complete history." >&2
  BASE_VERSION="0.0.0"
  LOG_RANGE="HEAD"
fi

IFS='.' read -r MAJOR MINOR PATCH <<< "$BASE_VERSION"
MAJOR="${MAJOR:-0}"; MINOR="${MINOR:-0}"; PATCH="${PATCH:-0}"

# Analyze the commits in the relevant range by conventional type.
# A `!!` prefix in the subject marks a breaking change.
HAS_BREAKING=false
HAS_FEATURE=false
HAS_FIX=false
while IFS= read -r line; do
  case "$line" in
    *'!!'*|*'BREAKING CHANGE'*) HAS_BREAKING=true ;;
  esac
  case "$line" in
    feat!*|feat\(*\)!*) HAS_BREAKING=true ;;
    feat:*|feat\(*\):*) HAS_FEATURE=true ;;
    fix:*|fix\(*\):*) HAS_FIX=true ;;
  esac
done < <(git log "$LOG_RANGE" --pretty=format:'%s%n%b' 2>/dev/null || true)

# If there are no commits in the range, the version stays unchanged.
if [[ -z "$(git log "$LOG_RANGE" --oneline 2>/dev/null || true)" ]]; then
  echo "No new commits since $LAST_TAG - version stays $BASE_VERSION" >&2
  echo "$BASE_VERSION"
  exit 0
fi

if [[ "$HAS_BREAKING" == true ]]; then
  MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0
  REASON="breaking change(s)"
elif [[ "$HAS_FEATURE" == true ]]; then
  MINOR=$((MINOR + 1)); PATCH=0
  REASON="feature commits"
else
  PATCH=$((PATCH + 1))
  REASON="fix commits"
fi

NEXT="$MAJOR.$MINOR.$PATCH"
echo "Next version: $NEXT (basis: $REASON)" >&2
echo "$NEXT"
