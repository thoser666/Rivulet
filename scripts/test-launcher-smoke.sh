#!/usr/bin/env bash
set -euo pipefail

# The launcher has no GUI/GStreamer dependency. Override the child path and log
# directory so this test is hermetic and never touches a developer's logs.
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

cargo build -p rivulet-launcher --quiet
launcher="$repo_root/target/debug/rivulet-launcher"
log_dir="$tmp_dir/logs"
missing_gui="$tmp_dir/missing/rivulet-gui"

set +e
RIVULET_LAUNCHER_LOG_DIR="$log_dir" \
RIVULET_GUI_PATH="$missing_gui" \
  "$launcher"
exit_code=$?
set -e

if [[ "$exit_code" -eq 0 ]]; then
  echo "launcher unexpectedly succeeded with a missing GUI binary" >&2
  exit 1
fi

log_file="$(find "$log_dir" -maxdepth 1 -type f -name 'rivulet-*.log' -print -quit)"
[[ -n "$log_file" ]] || { echo "launcher did not create a daily log" >&2; exit 1; }
grep -Fq '===== RIVULET PRE-RUST DIAGNOSTIC =====' "$log_file"
grep -Fq 'context: launcher' "$log_file"
grep -Fq '===== END RIVULET PRE-RUST DIAGNOSTIC =====' "$log_file"

echo "Launcher smoke test passed: $log_file"
