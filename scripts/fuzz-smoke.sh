#!/usr/bin/env bash
# Local smoke run of the fuzz targets — the same commands the CI
# "Fuzz smoke (regression corpus)" job executes on Linux.
#
# This is NOT an acceptance fuzzing campaign: it builds each target with
# libFuzzer (nightly toolchain, as cargo-fuzz requires) and runs a fixed,
# short number of executions per target so parser regressions surface in
# minutes, not hours. Set FUZZ_MAX_TOTAL_TIME (e.g. 600) for the deep,
# time-budgeted campaign mode that the weekly workflow uses:
#
#   FUZZ_MAX_TOTAL_TIME=600 bash scripts/fuzz-smoke.sh
#
# Requires: rustup nightly + cargo-fuzz (cargo install cargo-fuzz --locked).
#
# Sanitizer note: libFuzzer builds use AddressSanitizer. On Linux the ASan
# runtime ships with rustup's rust-std, so this works out of the box. On
# Windows the runtime comes from the Visual Studio "C++ AddressSanitizer"
# component; without it the build links but fails to launch
# (STATUS_DLL_NOT_FOUND). WSL or CI are the supported paths on Windows.
set -euo pipefail

cd "$(dirname "$0")/.."
FUZZ_DIR="fuzz"
RUNS="${FUZZ_SMOKE_RUNS:-256}"
# Deep-campaign mode: when FUZZ_MAX_TOTAL_TIME is set, each target runs for
# that many seconds instead of a fixed number of executions. The scheduled
# "Deep fuzz (weekly)" workflow uses this with the corpus persisted via the
# actions cache, so coverage accumulates across runs.
MAX_TIME="${FUZZ_MAX_TOTAL_TIME:-}"

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    echo "SKIP: the fuzz smoke needs the ASan runtime; on Windows use WSL" >&2
    echo "      or install the VS 'C++ AddressSanitizer' component." >&2
    echo "      CI runs this gate on Linux for every push." >&2
    exit 2
    ;;
esac

if ! command -v cargo >/dev/null 2>&1; then
  echo "FAIL: cargo not found" >&2
  exit 2
fi

if ! cargo +nightly fuzz --version >/dev/null 2>&1; then
  echo "cargo-fuzz is not available for the nightly toolchain." >&2
  echo "Install it with: cargo install cargo-fuzz --locked" >&2
  exit 2
fi

status=0
for target in parse_irc_line sdp_offer_endpoint parse_latest_release parse_checksums; do
  if [ -n "$MAX_TIME" ]; then
    echo "-- fuzz deep: $target (max ${MAX_TIME}s)"
    budget_args="-max_total_time=$MAX_TIME"
  else
    echo "-- fuzz smoke: $target ($RUNS runs)"
    budget_args="-runs=$RUNS"
  fi
  if ! (cd "$FUZZ_DIR" && cargo +nightly fuzz run --sanitizer address "$target" \
        -- "$budget_args" -max_len=4096); then
    echo "FAIL: fuzz target $target found a crash" >&2
    status=1
  fi
done

if [ "$status" -eq 0 ]; then
  if [ -n "$MAX_TIME" ]; then
    echo "fuzz deep passed (${MAX_TIME}s per target)"
  else
    echo "fuzz smoke passed (${RUNS} runs per target)"
  fi
fi
exit "$status"
