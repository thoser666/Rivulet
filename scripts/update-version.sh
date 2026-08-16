#!/usr/bin/env bash
# Sets the workspace version in Cargo.toml and all crates.
#
# The crates use `version.workspace = true` (see [workspace.package]);
# therefore it is enough to update the central version.
#
# Usage: scripts/update-version.sh <version>
set -euo pipefail

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  echo "Usage: $0 <version>" >&2
  exit 1
fi

# Validate conformance: SemVer with an optional pre-release suffix.
if ! grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' <<< "$VERSION"; then
  echo "Invalid version number: $VERSION" >&2
  exit 1
fi

MANIFEST="Cargo.toml"
if [[ ! -f "$MANIFEST" ]]; then
  echo "Cargo.toml not found - please run from the workspace root." >&2
  exit 1
fi

# Replace [workspace.package] version = "x.y.z".
if grep -q '^version = ' "$MANIFEST"; then
  sed -i -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?\"/version = \"$VERSION\"/" "$MANIFEST"
else
  echo "No version field found in $MANIFEST." >&2
  exit 1
fi

# Make sure all crates use the workspace version.
# Only replace the version in the [package] section (dependency versions like
# "windows-capture = { version = ... }" or "version = "2.0.0"" under
# [target.'...'.dependencies] must not be touched). Important: the
# [package] line itself must be preserved (no `next` in awk!).
for manifest in rivulet-*/Cargo.toml; do
  tmp="${manifest}.tmp"
  awk '
    /^\[package\]/ { in_package = 1 }
    /^\[/ && $0 != "[package]" { in_package = 0 }
    in_package && /^version = / {
      sub(/^version = ".*"/, "version.workspace = true")
    }
    { print }
  ' "$manifest" > "$tmp"
  mv "$tmp" "$manifest"
done

echo "Version set to $VERSION."
