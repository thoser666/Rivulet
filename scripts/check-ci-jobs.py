#!/usr/bin/env python3
"""Evaluate the `needs` context of the required CI aggregate job.

The `CI` job in `.github/workflows/ci.yml` is a required status check on
`develop`, so it has to judge every job it depends on. It used to compare a
hand-maintained list of `${{ needs.<job>.result }}` values: adding a job to
`needs` without adding it to that list made the aggregate silently ignore it
(`render_smoke` was missing for exactly that reason), and a job that never got a
runner was reported as the opaque

    A required CI job finished with: abandoned

with no job name and no recovery path. This script reads `toJSON(needs)` as a
whole, so a new dependency is covered automatically, and it explains *which* job
ended in *which* state and what to do about it.

Usage (inside the workflow):

    env:
      CI_NEEDS_JSON: ${{ toJSON(needs) }}
    run: python3 scripts/check-ci-jobs.py

Exit codes: 0 when every required job succeeded, 1 otherwise.
`--self-test` runs the built-in cases and needs no network or repository state.
"""

import json
import os
import sys

# Results that mean "this job verified what it was supposed to verify".
PASSING = {"success"}

# GitHub cancels a job that waits for a runner it never gets (for example a
# macOS runner during a capacity shortage). `needs` then reports `cancelled`,
# and for a job that was cancelled while still queued it reports the
# undocumented `abandoned`. Neither says anything about the code under test, so
# the two get their own explanation instead of a bare "failed".
INFRASTRUCTURE = {"abandoned", "cancelled"}

# A real problem in the code, the tests or the workflow itself.
FAILING = {"failure", "timed_out"}

# A skipped required job means the check the ruleset promises did not run.
SKIPPED = {"skipped"}


def describe(job, result):
    """Return (hint, is_problem) for one `needs` entry."""
    if result in PASSING:
        return "", False
    if result in INFRASTRUCTURE:
        return (
            f"{job} was cancelled before it could run "
            f"(runner capacity or superseded run, not a test failure); "
            f"rerun the failed jobs with `gh run rerun <run-id> --failed`",
            True,
        )
    if result in FAILING:
        return f"{job} failed; open its job log before rerunning", True
    if result in SKIPPED:
        return f"{job} was skipped, so its check never verified anything", True
    return (
        f"{job} reported the unexpected result {result!r}; treat it as a failure",
        True,
    )


def evaluate(needs):
    """Return (problems, lines) for a decoded `toJSON(needs)` mapping.

    `problems` holds one explanation per job that is not a success; `lines`
    holds a `job: result` row per job for the step summary.
    """
    problems = []
    lines = []
    for job in sorted(needs):
        entry = needs[job] or {}
        result = entry.get("result", "missing")
        lines.append(f"- `{job}`: {result}")
        hint, is_problem = describe(job, result)
        if is_problem:
            problems.append(hint)
    return problems, lines


def load(payload):
    """Decode the needs JSON; return (needs, error)."""
    if not payload or not payload.strip():
        return None, "CI_NEEDS_JSON is empty; the workflow must pass toJSON(needs)"
    try:
        decoded = json.loads(payload)
    except json.JSONDecodeError as exc:
        return None, f"CI_NEEDS_JSON is not valid JSON: {exc}"
    if not isinstance(decoded, dict):
        return None, "CI_NEEDS_JSON must decode to an object of job results"
    return decoded, None


def report(needs):
    """Print the evaluation; return the process exit code."""
    problems, lines = evaluate(needs)
    print("Required CI jobs:")
    for line in lines:
        print(line)
    if not problems:
        print(f"All {len(needs)} required CI jobs succeeded.")
        return 0
    print("", file=sys.stderr)
    for problem in problems:
        print(f"::error::{problem}", file=sys.stderr)
    return 1


def run_self_test():
    """Verify the classification; needs neither network nor repository state."""
    failures = 0

    def case(name, ok):
        nonlocal failures
        print(("PASS" if ok else "FAIL") + f" [{name}]")
        if not ok:
            failures += 1

    all_green = {job: {"result": "success"} for job in ("lints", "build_and_test")}
    case("all jobs green", evaluate(all_green)[0] == [])

    failed = {"lints": {"result": "success"}, "pinning_tests": {"result": "failure"}}
    problems, _ = evaluate(failed)
    case(
        "failure names the job",
        len(problems) == 1
        and "pinning_tests" in problems[0]
        and "job log" in problems[0],
    )

    never_started = {"build_and_test": {"result": "abandoned"}}
    problems, _ = evaluate(never_started)
    case(
        "abandoned is infrastructure, not a code failure",
        len(problems) == 1
        and "cancelled before it could run" in problems[0]
        and "gh run rerun" in problems[0],
    )

    cancelled = {"render_smoke": {"result": "cancelled"}}
    case(
        "cancelled points at a rerun",
        "rerun" in evaluate(cancelled)[0][0],
    )

    timed_out = {"coverage": {"result": "timed_out"}}
    case("timeout counts as a failure", "coverage" in evaluate(timed_out)[0][0])

    skipped = {"coverage": {"result": "skipped"}}
    case("skipped required job is a problem", "never verified" in evaluate(skipped)[0][0])

    unknown = {"lints": {"result": "something-new"}}
    case("unknown result fails loudly", "unexpected result" in evaluate(unknown)[0][0])

    # A job added to `needs` must be evaluated without editing any list here.
    case(
        "new needs entry is covered automatically",
        "brand_new_job" in evaluate({"brand_new_job": {"result": "failure"}})[0][0],
    )

    case(
        "empty needs is rejected",
        load("")[1] is not None and load("  ")[1] is not None,
    )
    case("invalid JSON is rejected", load("{not json")[1] is not None)
    case("non-object JSON is rejected", load("[]")[1] is not None)
    case("valid JSON decodes", load('{"lints": {"result": "success"}}')[1] is None)

    if failures:
        print(f"{failures} self-test case(s) FAILED")
        return 1
    print("All self-test cases passed.")
    return 0


def main():
    args = sys.argv[1:]
    if "--help" in args or "-h" in args:
        print(__doc__.strip())
        return 0
    if "--self-test" in args:
        return run_self_test()

    payload = os.environ.get("CI_NEEDS_JSON")
    if payload is None and len(args) == 1 and not args[0].startswith("-"):
        payload = args[0]
    needs, error = load(payload)
    if error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return report(needs)


if __name__ == "__main__":
    sys.exit(main())
