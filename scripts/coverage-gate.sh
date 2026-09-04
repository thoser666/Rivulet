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
# Usage: scripts/coverage-gate.sh   (expects cargo-llvm-cov + llvm-tools)

set -euo pipefail

MIN="${COVERAGE_MIN:=80}"
CRATE="${COVERAGE_CRATE:=rivulet-core}"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "::error::cargo-llvm-cov is not installed (run: cargo install cargo-llvm-cov --locked)" >&2
  exit 2
fi

echo "==> Coverage gate: ${CRATE} >= ${MIN}% statement coverage"
# --fail-under-lines exits 1 below MIN; tee the summary for the log.
cargo llvm-cov --version >/dev/null 2>&1 || true
cargo llvm-cov -p "$CRATE" --fail-under-lines "$MIN" --summary-only 2>&1 | tail -n 15