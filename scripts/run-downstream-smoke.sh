#!/usr/bin/env bash
# M8 #297 exit evidence: build and run the downstream-consumer smoke crate
# against the artifact `cargo publish` would upload, never against the tree.
#
# The point of this script is that a *foreign* crate — no workspace
# membership, no internal source paths, no dev-dependencies — can use
# `rivulet-core` as a normal library. Everything it builds against comes out
# of `cargo package`, so a public item that only exists in the working tree
# fails here instead of at the next release.
#
# Usage:
#   bash scripts/run-downstream-smoke.sh
#
# Environment:
#   CARGO_TARGET_DIR                    shared target dir (default: <repo>/target),
#                                       so the consumer reuses the workspace's
#                                       compiled dependencies instead of
#                                       rebuilding them
#   RIVULET_DOWNSTREAM_SMOKE_DIR        build dir (default:
#                                       <repo>/target/downstream-smoke)
#   RIVULET_DOWNSTREAM_REQUIRE_AUDIO=1  fail if the audio leg cannot run instead
#                                       of accepting the typed refusal (CI sets
#                                       this; see downstream-smoke/README.md)
#   CARGO                               cargo binary to use (default: cargo)
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_bin="${CARGO:-cargo}"
consumer_src="$repo_root/downstream-smoke"
work_dir="${RIVULET_DOWNSTREAM_SMOKE_DIR:-$repo_root/target/downstream-smoke}"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"

if ! command -v "$cargo_bin" >/dev/null 2>&1; then
  echo "downstream-smoke: no cargo on PATH (tried '$cargo_bin')" >&2
  exit 1
fi

echo "downstream-smoke: packaging rivulet-core (the artifact cargo publish would upload)"
rm -rf "$work_dir"
mkdir -p "$work_dir/pkg" "$work_dir/consumer"

"$cargo_bin" package -p rivulet-core --no-verify --allow-dirty \
  --target-dir "$work_dir/package-target"

crate_file="$(ls "$work_dir"/package-target/package/rivulet-core-*.crate | head -n 1)"
crate_name="$(basename "$crate_file" .crate)"
echo "downstream-smoke: packaged $crate_name ($(wc -c <"$crate_file" | tr -d ' ') bytes)"

tar xzf "$crate_file" -C "$work_dir/pkg"
packaged="$work_dir/pkg/$crate_name"
test -f "$packaged/src/lib.rs" ||
  { echo "downstream-smoke: $crate_name has no src/lib.rs" >&2; exit 1; }

# The consumer gets a copy of the sources and a manifest whose only dependency
# is the unpacked artifact, addressed relatively so the build works from any
# checkout location (including Git Bash on Windows).
cp -R "$consumer_src/src" "$work_dir/consumer/src"
sed "s|__RIVULET_CORE_PATH__|../pkg/$crate_name|" \
  "$consumer_src/Cargo.toml.in" >"$work_dir/consumer/Cargo.toml"
if grep -q "__RIVULET_CORE_PATH__" "$work_dir/consumer/Cargo.toml"; then
  echo "downstream-smoke: the manifest template placeholder was not substituted" >&2
  exit 1
fi

# A manifest without the placeholder would make this a working-tree test.
grep -q "path = \"../pkg/$crate_name\"" "$work_dir/consumer/Cargo.toml" ||
  { echo "downstream-smoke: the consumer manifest does not point at the package" >&2; exit 1; }

# The consumer must depend on rivulet-core and nothing else: a second entry
# would mean the build proves less than it claims.
dependencies="$(sed -n '/^\[dependencies\]/,/^\[/p' "$work_dir/consumer/Cargo.toml" |
  grep -c "rivulet-core" | tr -d ' ')"
if [ "$dependencies" != "1" ]; then
  echo "downstream-smoke: expected exactly one dependency entry, found $dependencies" >&2
  exit 1
fi

echo "downstream-smoke: building and running the consumer against $crate_name"
(
  cd "$work_dir/consumer"
  CARGO_TARGET_DIR="$target_dir" "$cargo_bin" run --quiet
)

echo "downstream-smoke: OK — an external crate drives the packaged artifact end to end"
