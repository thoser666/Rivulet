#!/usr/bin/env bash
# Local smoke run of the fuzz targets — the same commands the CI
# "Fuzz smoke (regression corpus)" job executes on Linux.
#
# This is NOT an acceptance fuzzing campaign: it builds each target with
# libFuzzer (nightly toolchain, as cargo-fuzz requires) and runs a fixed,
# short number of executions per target so parser regressions surface in
# minutes, not hours. Full campaigns stay a manual task:
#
#   cargo +nightly fuzz run parse_irc_line -- -max_total_time=600
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
  echo "-- fuzz smoke: $target ($RUNS runs)"
  if ! (cd "$FUZZ_DIR" && cargo +nightly fuzz run --sanitizer address "$target" \
        -- -runs="$RUNS" -max_len=4096); then
    echo "FAIL: fuzz target $target found a crash" >&2
    status=1
  fi
done

if [ "$status" -eq 0 ]; then
  echo "fuzz smoke passed (${RUNS} runs per target)"
fi
exit "$status"
