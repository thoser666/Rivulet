#!/usr/bin/env python3
"""Validate resource-efficiency reports for milestone CI gates.

Two layers decide whether a resource number is acceptable:

- **This script** validates the *schema* and the absolute targets the goal doc
  states for any machine: idle CPU, memory growth, RSS slope, positive startup
  and frame time, and that a `blocked`/`n/a`/`fail` record carries a reason.
  Since M9 #363 it also validates the fields a real measurement produces
  (`idle_cpu_percent`, `rss_slope_mb_per_min`, `startup_ms`, `verdicts`, ...),
  while still accepting reports that only carry the original four fields -- the
  M6 harness and the deterministic fixture do, and a schema change must not
  retroactively invalidate them.
- **`scripts/resource-metrics.py`** compares a measurement against a committed
  per-profile baseline, because "is this a regression" can only be answered
  relative to real recorded numbers.

Keeping the split explicit is what stops a synthetic fixture from being
mistaken for hardware evidence: the fixture proves this validator works, the
baseline proves the application did not get worse, and everything that needs a
GPU, a game or a battery stays manual evidence.
"""
import argparse
import json
import math
import sys

MAX_CPU_DELTA_PERCENT = 2.0
MAX_MEMORY_GROWTH_MB = 64.0
MAX_FRAME_TIME_REGRESSION_PERCENT = 1.0

#: M9 #363. The goal doc states these as absolute targets ("<= 2 % additional
#: process CPU after startup settles", no unbounded growth), so they are checked
#: here rather than only relative to a baseline.
MAX_IDLE_CPU_PERCENT = 2.0
MAX_RSS_SLOPE_MB_PER_MIN = 4.0

REQUIRED_FIELDS = (
    "cpu_delta_percent",
    "memory_growth_mb",
    "frame_time_regression_percent",
    "p99_frame_time_ms",
)

#: Fields a measurement may add. Validated when present, never required, so an
#: older report stays valid.
OPTIONAL_NUMERIC_FIELDS = (
    "idle_cpu_percent",
    "rss_slope_mb_per_min",
    "startup_ms",
    "engine_init_ms",
    "peak_rss_mb",
    "mean_frame_time_ms",
    "max_frame_time_ms",
    "capture_cpu_percent",
    "capture_cpu_overhead_percent",
)

STATUS_VALUES = ("pass", "fail", "blocked", "n/a")

#: A status that is not a plain pass has to say why. "No data" without a reason
#: is how an unmeasured gate turns into a green check.
STATUS_NEEDS_REASON = ("fail", "blocked", "n/a")


def validate(report):
    """Return a list of human-readable errors; empty means the report is valid."""
    profiles = report.get("profiles")
    if not isinstance(profiles, list) or not profiles:
        return ["report contains no profiles"]

    errors = []
    for sample in profiles:
        errors.extend(_validate_profile(sample))
    return errors


def _validate_profile(sample):
    if not isinstance(sample, dict):
        return [f"profile must be an object, got {type(sample).__name__}"]
    name = sample.get("name", "unknown")
    errors = []

    missing = [field for field in REQUIRED_FIELDS if field not in sample]
    if missing:
        return [f"{name}: missing fields: {', '.join(missing)}"]
    for field in REQUIRED_FIELDS:
        if not _is_number(sample[field]):
            errors.append(f"{name}: {field} must be a finite number")
    if errors:
        # The budget checks below would produce a traceback or a meaningless
        # comparison on a missing/non-numeric field.
        return errors

    if sample["cpu_delta_percent"] > MAX_CPU_DELTA_PERCENT:
        errors.append(f"{name}: CPU delta exceeds {MAX_CPU_DELTA_PERCENT:.1f}%")
    if sample["memory_growth_mb"] > MAX_MEMORY_GROWTH_MB:
        errors.append(f"{name}: memory growth exceeds {MAX_MEMORY_GROWTH_MB:.0f} MiB")
    if (
        sample["frame_time_regression_percent"] > MAX_FRAME_TIME_REGRESSION_PERCENT
        and not sample.get("verdicts")
    ):
        # The 1 % rule is a *fixture* rule: a controlled report states its
        # regression against a controlled reference, so 1 % is the budget. A
        # measured report carries its own `verdicts`, computed against a
        # committed baseline with a runner-noise-aware budget; applying both
        # would gate the same number twice, the second time at a threshold
        # shared runners cannot hold.
        errors.append(
            f"{name}: frame-time regression exceeds "
            f"{MAX_FRAME_TIME_REGRESSION_PERCENT:.1f}%"
        )
    if sample["p99_frame_time_ms"] <= 0:
        errors.append(f"{name}: p99_frame_time_ms must be positive")

    errors.extend(_validate_optional_fields(name, sample))
    errors.extend(_validate_verdicts(name, sample))
    errors.extend(_validate_status(name, sample))
    return errors


def _validate_optional_fields(name, sample):
    errors = []
    for field in OPTIONAL_NUMERIC_FIELDS:
        if field in sample and not _is_number(sample[field]):
            errors.append(f"{name}: {field} must be a finite number when present")
    if "p95_frame_time_ms" in sample:
        p95 = sample["p95_frame_time_ms"]
        if not _is_number(p95) or p95 <= 0:
            errors.append(f"{name}: p95_frame_time_ms must be positive")
    if "one_percent_low_fps" in sample:
        low = sample["one_percent_low_fps"]
        if not _is_number(low) or low <= 0:
            errors.append(f"{name}: one_percent_low_fps must be positive")
    if "gpu_utilization_percent" in sample:
        gpu = sample["gpu_utilization_percent"]
        if not _is_number(gpu) or not 0 <= gpu <= 100:
            errors.append(f"{name}: gpu_utilization_percent must be between 0 and 100")
    if (
        "idle_cpu_percent" in sample
        and _is_number(sample["idle_cpu_percent"])
        and sample["idle_cpu_percent"] > MAX_IDLE_CPU_PERCENT
    ):
        errors.append(
            f"{name}: idle CPU {sample['idle_cpu_percent']:.2f}% exceeds the "
            f"{MAX_IDLE_CPU_PERCENT:.1f}% target"
        )
    if (
        "rss_slope_mb_per_min" in sample
        and _is_number(sample["rss_slope_mb_per_min"])
        and sample["rss_slope_mb_per_min"] > MAX_RSS_SLOPE_MB_PER_MIN
    ):
        errors.append(
            f"{name}: RSS slope {sample['rss_slope_mb_per_min']:.2f} MiB/min exceeds "
            f"the {MAX_RSS_SLOPE_MB_PER_MIN:.1f} MiB/min budget"
        )
    if "startup_ms" in sample and _is_number(sample["startup_ms"]) and sample["startup_ms"] <= 0:
        errors.append(f"{name}: startup_ms must be positive")
    if "peak_rss_mb" in sample and _is_number(sample["peak_rss_mb"]) and sample["peak_rss_mb"] <= 0:
        errors.append(f"{name}: peak_rss_mb must be positive")
    if "threads_max" in sample:
        threads = sample["threads_max"]
        if not isinstance(threads, int) or isinstance(threads, bool) or threads <= 0:
            errors.append(f"{name}: threads_max must be a positive integer")
    return errors


def _validate_verdicts(name, sample):
    if "verdicts" not in sample:
        return []
    verdicts = sample["verdicts"]
    if not isinstance(verdicts, list) or not verdicts:
        return [f"{name}: verdicts must be a non-empty list"]
    errors = []
    for verdict in verdicts:
        if not isinstance(verdict, dict):
            errors.append(f"{name}: every verdict must be an object")
            continue
        metric = verdict.get("metric", "unknown metric")
        status = verdict.get("status")
        if status not in STATUS_VALUES:
            errors.append(
                f"{name}: verdict {metric} has status {status!r} "
                f"(expected one of {', '.join(STATUS_VALUES)})"
            )
        elif status in STATUS_NEEDS_REASON and not _has_text(verdict.get("reason")):
            errors.append(
                f"{name}: verdict {metric} is {status} without a reason"
            )
    return errors


def _validate_status(name, sample):
    if "status" not in sample:
        return []
    status = sample["status"]
    errors = []
    if status not in STATUS_VALUES:
        return [
            f"{name}: status {status!r} (expected one of {', '.join(STATUS_VALUES)})"
        ]
    if status in STATUS_NEEDS_REASON and not _has_text(sample.get("reason")):
        errors.append(f"{name}: status {status} without a reason")
    verdicts = sample.get("verdicts") or []
    if status in ("pass", "blocked", "n/a") and any(
        verdict.get("status") == "fail" for verdict in verdicts
    ):
        errors.append(f"{name}: status {status} but a verdict failed")
    if status == "fail" and not any(verdict.get("status") == "fail" for verdict in verdicts):
        errors.append(f"{name}: status fail but no verdict failed")
    return errors


def _is_number(value):
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
    )


def _has_text(value):
    return isinstance(value, str) and value.strip() != ""


# --------------------------------------------------------------------------
# Self-test
# --------------------------------------------------------------------------


def _base_profile(**overrides):
    profile = {
        "name": "self-test",
        "cpu_delta_percent": 0.1,
        "memory_growth_mb": 0.5,
        "frame_time_regression_percent": 0.0,
        "p99_frame_time_ms": 5.0,
    }
    profile.update(overrides)
    return profile


def _report(*profiles):
    return {"schema_version": 1, "profiles": list(profiles)}


def run_self_test():
    """Policy tests: each threshold must fail on a regression and pass on health."""
    failures = []

    def expect_valid(label, report):
        errors = validate(report)
        if errors:
            failures.append(f"{label}: expected valid, got {errors}")

    def expect_error(label, report, needle):
        errors = validate(report)
        if not errors:
            failures.append(f"{label}: expected an error, got none")
        elif not any(needle in error for error in errors):
            failures.append(f"{label}: expected an error containing {needle!r}, got {errors}")

    # The committed fixture the CI job validates must stay valid.
    expect_valid("committed fixture", _report(_base_profile()))
    expect_valid("legacy four-field profile", _report(_base_profile()))
    expect_valid(
        "a measured profile",
        _report(
            _base_profile(
                idle_cpu_percent=0.4,
                rss_slope_mb_per_min=0.3,
                startup_ms=120.0,
                engine_init_ms=40.0,
                peak_rss_mb=88.0,
                threads_max=14,
                p95_frame_time_ms=4.0,
                mean_frame_time_ms=3.0,
                one_percent_low_fps=55.0,
                status="pass",
                verdicts=[
                    {"metric": "idle_cpu_percent", "status": "pass", "measured": 0.4},
                ],
            )
        ),
    )

    expect_error("no profiles", {"schema_version": 1}, "no profiles")
    expect_error("null field", _report(_base_profile(p99_frame_time_ms=None)), "finite number")
    profile = _base_profile()
    del profile["memory_growth_mb"]
    expect_error("deleted field", _report(profile), "missing fields")
    expect_error("non-numeric", _report(_base_profile(startup_ms="12")), "startup_ms")
    expect_error("NaN", _report(_base_profile(idle_cpu_percent=float("nan"))), "idle_cpu_percent")

    expect_error(
        "idle CPU above target",
        _report(_base_profile(idle_cpu_percent=2.5)),
        "exceeds the 2.0% target",
    )
    expect_valid("idle CPU at target", _report(_base_profile(idle_cpu_percent=2.0)))
    expect_error(
        "growing RSS slope",
        _report(_base_profile(rss_slope_mb_per_min=4.5)),
        "RSS slope",
    )
    expect_valid("flat RSS slope", _report(_base_profile(rss_slope_mb_per_min=0.5)))
    expect_error("zero startup", _report(_base_profile(startup_ms=0.0)), "startup_ms")
    expect_error("negative startup", _report(_base_profile(startup_ms=-1.0)), "startup_ms")
    expect_error("thread count of zero", _report(_base_profile(threads_max=0)), "threads_max")
    expect_error("CPU delta budget", _report(_base_profile(cpu_delta_percent=5.0)), "CPU delta")
    expect_error(
        "memory budget", _report(_base_profile(memory_growth_mb=80.0)), "memory growth"
    )
    expect_error(
        "frame-time regression budget",
        _report(_base_profile(frame_time_regression_percent=2.0)),
        "frame-time regression",
    )
    # A measured report carries its own verdicts and gate: the 1 % fixture rule
    # must not double-gate the same number at a threshold shared runners cannot
    # hold.
    expect_valid(
        "a measured report states its own regression budget",
        _report(
            _base_profile(
                frame_time_regression_percent=6.0,
                status="pass",
                verdicts=[
                    {"metric": "p99_frame_time_ms", "status": "pass", "measured": 5.3},
                ],
            )
        ),
    )
    expect_error(
        "non-positive p99", _report(_base_profile(p99_frame_time_ms=0.0)), "p99_frame_time_ms"
    )

    # Verdict/reason semantics: the whole point of the M9 field.
    expect_error(
        "blocked without reason",
        _report(_base_profile(status="blocked")),
        "status blocked without a reason",
    )
    expect_valid(
        "blocked with reason",
        _report(_base_profile(status="blocked", reason="no battery on this runner")),
    )
    expect_error(
        "unknown status",
        _report(_base_profile(status="fine")),
        "expected one of",
    )
    expect_error(
        "fail without a failing verdict",
        _report(_base_profile(status="fail", reason="nope")),
        "status fail but no verdict failed",
    )
    expect_valid(
        "fail with a failing verdict",
        _report(
            _base_profile(
                status="fail",
                reason="idle CPU regressed",
                verdicts=[
                    {"metric": "idle_cpu_percent", "status": "fail", "reason": "3.1 > 2.0"},
                ],
            )
        ),
    )
    expect_error(
        "verdict without reason",
        _report(_base_profile(verdicts=[{"metric": "startup_ms", "status": "blocked"}])),
        "without a reason",
    )
    expect_error(
        "verdict with an unknown status",
        _report(_base_profile(verdicts=[{"metric": "startup_ms", "status": "maybe"}])),
        "expected one of",
    )
    expect_error(
        "empty verdict list",
        _report(_base_profile(verdicts=[])),
        "non-empty list",
    )

    for failure in failures:
        print(f"self-test FAILED: {failure}", file=sys.stderr)
    if not failures:
        print("resource-efficiency-check self-test: OK")
    return not failures


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("report", nargs="?")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return 0 if run_self_test() else 1
    if not args.report:
        parser.error("the report path is required")

    try:
        with open(args.report, encoding="utf-8") as handle:
            report = json.load(handle)
    except (OSError, json.JSONDecodeError) as error:
        print(f"Resource-efficiency gate: FAIL\n- invalid report: {error}")
        return 1
    errors = validate(report)
    if errors:
        print("Resource-efficiency gate: FAIL")
        for error in errors:
            print(f"- {error}")
        return 1
    print(f"Resource-efficiency gate: PASS ({len(report['profiles'])} profile(s))")
    return 0


if __name__ == "__main__":
    sys.exit(main())
