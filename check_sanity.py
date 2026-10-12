#!/usr/bin/env python3
"""Smoke check for the rivulet-gstreamer builder.

CLI entry point from CS-001 C55 (m6/p8 safeguards):
  python check_sanity.py

Exit codes:
  0 - environment is sane. A missing pytest is a documented graceful skip
      (this is a sanity check, not CI), and pytest exit code 5 ("no tests
      collected") is also OK: the repository has no Python test suite yet.
  * - the first failing step's exit code (cargo check failure or a real
      pytest failure).
"""

import subprocess
import sys

PYTHON = sys.executable

# pytest exit status 5: no tests were collected. Not a failure for a sanity
# check of a repository that has no Python test suite.
PYTEST_NO_TESTS_COLLECTED = 5


def _run(cmd: list[str]):
    return subprocess.run(cmd, capture_output=True, text=True)


def main() -> int:
    print(f"Python: {PYTHON} ({sys.version.splitlines()[0]})")

    # 1. lint check - cargo check failing means the workspace does not build,
    # which is exactly what a sanity check exists to catch, so its exit code
    # is folded into the result. This step runs even when pytest is missing
    # below, and always prints the stderr tail marker so the wrapper's output
    # contract is stable for callers that parse it.
    print("\n> lint (cargo check) ...")
    r = _run(["cargo", "check", "--quiet"])
    print("cargo check", "OK" if r.returncode == 0 else f"FAIL rc={r.returncode}")
    # Raw compiler diagnostics are the first thing a developer needs when the
    # check fails; print them verbatim (the tail keeps the output bounded).
    print("STDERR (tail):", (r.stderr or "").strip()[-400:])
    lint_rc = r.returncode

    # 2. pytest availability
    r = _run([PYTHON, "-m", "pytest", "--version"])
    print("pytest", "OK" if r.returncode == 0 else f"NOT FOUND (rc={r.returncode})")
    if r.returncode != 0:
        print("SKIP: pytest not installed, install with pip install pytest.")
        return lint_rc  # graceful skip of the test step only, lint still counts

    # 3. tests. Build and scratch trees hold no tests and only make a
    # root-level run crawl, so they are excluded from collection.
    print("\n> tests ...")
    pytest = _run([PYTHON, "-m", "pytest", "-q", "--ignore=target", "--ignore=tmp"])
    if pytest.returncode == PYTEST_NO_TESTS_COLLECTED:
        print("pytest OK (no tests collected - this repo has no Python tests)")
        tests_rc = 0
    else:
        print("pytest", "OK" if pytest.returncode == 0 else f"FAIL rc={pytest.returncode}")
        tests_rc = pytest.returncode
    print(pytest.stdout)

    return lint_rc or tests_rc


if __name__ == "__main__":
    sys.exit(main())
