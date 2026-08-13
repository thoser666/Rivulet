#!/usr/bin/env bash
# Setzt die Workspace-Version in Cargo.toml und allen Crates.
#
# Die Crates verwenden `version.workspace = true` (siehe [workspace.package]);
# daher reicht es, die zentrale Version zu aktualisieren.
#
# Verwendung: scripts/update-version.sh <version>
set -euo pipefail

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  echo "Usage: $0 <version>" >&2
  exit 1
fi

# Konformität prüfen: SemVer mit optionalem Pre-Release-Suffix.
if ! grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' <<< "$VERSION"; then
  echo "Ungültige Versionsnummer: $VERSION" >&2
  exit 1
fi

MANIFEST="Cargo.toml"
if [[ ! -f "$MANIFEST" ]]; then
  echo "Cargo.toml nicht gefunden - bitte im Workspace-Root ausführen." >&2
  exit 1
fi

# [workspace.package] version = "x.y.z" ersetzen.
if grep -q '^version = ' "$MANIFEST"; then
  sed -i -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?\"/version = \"$VERSION\"/" "$MANIFEST"
else
  echo "Kein version-Feld in $MANIFEST gefunden." >&2
  exit 1
fi

# Sicherstellen, dass alle Crates die Workspace-Version verwenden.
# Nur die Version im [package]-Abschnitt ersetzen (Dependency-Versionen wie
# "windows-capture = { version = ... }" oder "version = "2.0.0"" unter
# [target.'...'.dependencies] dürfen nicht angetastet werden).
for manifest in rivulet-*/Cargo.toml; do
  tmp="${manifest}.tmp"
  awk '
    /^\[package\]/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && /^version = / {
      sub(/^version = ".*"/, "version.workspace = true")
    }
    { print }
  ' "$manifest" > "$tmp"
  mv "$tmp" "$manifest"
done

echo "Version auf $VERSION gesetzt."
