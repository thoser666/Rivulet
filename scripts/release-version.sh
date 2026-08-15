#!/usr/bin/env bash
# Ermittelt die nächste Release-Version nach Semantic Versioning anhand der
# Conventional-Commits seit dem letzten Tag, und generiert daraus die
# Versions-String für den nächsten Release (beta/rc/stable).
#
# Ausgabe: <next-version>
set -euo pipefail

# Letzten Tag ermitteln (v[0-9]*); ohne existierenden Tag starten wir bei
# 0.0.0 und analysieren den kompletten Verlauf.
LAST_TAG="$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)"

if [[ -n "$LAST_TAG" ]]; then
  echo "Letzter Tag: $LAST_TAG" >&2
  BASE_VERSION="${LAST_TAG#v}"
  BASE_VERSION="${BASE_VERSION%%-*}"
  LOG_RANGE="$LAST_TAG..HEAD"
else
  echo "Kein vorheriger Tag gefunden - kompletter Verlauf wird analysiert." >&2
  BASE_VERSION="0.0.0"
  LOG_RANGE="HEAD"
fi

IFS='.' read -r MAJOR MINOR PATCH <<< "$BASE_VERSION"
MAJOR="${MAJOR:-0}"; MINOR="${MINOR:-0}"; PATCH="${PATCH:-0}"

# Commits im relevanten Bereich nach konventionellen Typen analysieren.
# `!!`-Präfix in der Subject kennzeichnet einen Breaking Change.
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

# Wenn keine Commits im Bereich existieren, bleibt die Version unverändert.
if [[ -z "$(git log "$LOG_RANGE" --oneline 2>/dev/null || true)" ]]; then
  echo "Keine neuen Commits seit $LAST_TAG - Version bleibt $BASE_VERSION" >&2
  echo "$BASE_VERSION"
  exit 0
fi

if [[ "$HAS_BREAKING" == true ]]; then
  MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0
  REASON="Breaking Change(s)"
elif [[ "$HAS_FEATURE" == true ]]; then
  MINOR=$((MINOR + 1)); PATCH=0
  REASON="Feature-Commits"
else
  PATCH=$((PATCH + 1))
  REASON="Fix-Commits"
fi

NEXT="$MAJOR.$MINOR.$PATCH"
echo "Nächste Version: $NEXT (Basis: $REASON)" >&2
echo "$NEXT"
