#!/usr/bin/env bash
# Coverage gate for the OpenSSF silver criterion `test_statement_coverage80`.
#
# Measures statement (line) coverage of `rivulet-core` — the crate that holds
# the actual streaming/chat/capture logic — and fails the build if total line
# coverage drops below 80%.
#
# Why rivulet-core and not the whole workspace?
# The workspace also contains platform-injected hook shims (Vulkan/OpenGL/DXGI
# capture layers), GUI shell crates, and the launcher; those cannot be
# exercised headlessly and pull the workspace total below 80% (~69%). The
# coverage claim in the OpenSSF entry is therefore scoped to the core library,
# which is the meaningful, unit-testable surface. See
# docs/security/assurance-case.md §6.
#
# Why serial tests?
# GStreamer request-pad linking and the plugin registry are exercised by many
# tests in parallel; under llvm-cov instrumentation (2-3x slower, freshly
# scanned registry on CI) that parallelism made deterministic-looking parses
# fail sporadically ("could not link … to mux") and starved real-time pipeline
# tests of CPU. Running the suite serially removes that contention — the run
# takes longer but is deterministic. Override with RUST_TEST_THREADS for local
# experimentation.
#
# Usage: scripts/coverage-gate.sh   (expects cargo-llvm-cov + llvm-tools)

set -euo pipefail

MIN="${COVERAGE_MIN:=80}"
CRATE="${COVERAGE_CRATE:=rivulet-core}"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "::error::cargo-llvm-cov is not installed (run: cargo install cargo-llvm-cov --locked)" >&2
  exit 2
fi

export RUST_TEST_THREADS="${RUST_TEST_THREADS:-1}"

echo "==> Coverage gate: ${CRATE} >= ${MIN}% statement coverage (RUST_TEST_THREADS=${RUST_TEST_THREADS})"
cargo llvm-cov --version >/dev/null 2>&1 || true

LOG="$(mktemp)"
trap 'rm -f "$LOG"' EXIT
set +e
# Keep the FULL log so a failing run is diagnosable — a `tail` pipe would have
# discarded the very failure details this gate must surface.
cargo llvm-cov -p "$CRATE" --fail-under-lines "$MIN" --summary-only >"$LOG" 2>&1
rc=$?
set -e

tail -n 25 "$LOG"
if [[ $rc -ne 0 ]]; then
  echo "::group::coverage failure details"
  grep -E "failures:|^---- |panicked at|test result:|coverage:|TOTAL" "$LOG" | tail -n 80 || true
  echo "::endgroup::"
fi
exit $rc
