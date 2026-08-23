#!/usr/bin/env python3
"""
G5 – Performance verification: CI budget gate.

This script validates a benchmark report (JSON) against the overhead budget
defined in docs/game-capture-strategy.md §5.

Usage:
    # Validate a local benchmark report:
    python scripts/g5-benchmark.py --report benchmark-report.json

    # Run the Rust benchmark framework unit tests (CI smoke check):
    python scripts/g5-benchmark.py --smoke-test

    # Generate a synthetic report for testing the CI gate:
    python scripts/g5-benchmark.py --generate-sample > benchmark-report.json

Budget (p99 frame-time delta):
    60 Hz  → < 0.17 ms
    120 Hz → < 0.08 ms
    144 Hz → < 0.07 ms
"""

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

# Budget thresholds from docs/game-capture-strategy.md §5
BUDGET_MS = {
    60: 0.17,
    120: 0.08,
    144: 0.07,
}


def get_budget_ms(refresh_hz: int) -> float:
    """Return the maximum allowed p99 delta (ms) for a refresh rate."""
    for threshold_hz, budget in sorted(BUDGET_MS.items()):
        if refresh_hz <= threshold_hz:
            return budget
    # Above all thresholds use the tightest budget
    return 0.07


def run_smoke_test() -> bool:
    """Run the benchmark module's unit tests as a CI smoke check."""
    print("G5: Running benchmark framework unit tests...")
    result = subprocess.run(
        ["cargo", "test", "-p", "rivulet-core", "--lib", "benchmark"],
        capture_output=True,
        text=True,
        timeout=120,
    )
    if result.returncode != 0:
        print(f"G5: FAIL Benchmark tests failed:\n{result.stdout}\n{result.stderr}")
        return False
    # Count test results
    passed = result.stdout.count(" ok")
    total = result.stdout.count("test benchmark::")
    print(f"G5: OK {passed}/{total} benchmark framework tests passed")
    return True


def validate_report(report: dict) -> tuple[bool, list[str]]:
    """Validate a benchmark report against the overhead budget.

    Returns (all_pass, list_of_failure_descriptions).
    """
    results = report.get("results", [])
    if not results:
        return False, ["No results in report"]

    failures = []
    for r in results:
        backend = r.get("backend", "unknown")
        refresh_hz = r.get("refresh_hz", 60)
        delta = r.get("delta_p99", 0.0)
        budget = get_budget_ms(refresh_hz)
        within = r.get("within_budget", delta <= budget)

        status = "OK" if within else "FAIL"
        print(
            f"  {status} {backend}@{refresh_hz}Hz: "
            f"d_p99={delta:.3f}ms (budget {budget:.3f}ms) "
            f"[baseline={r.get('baseline_p99', 0):.3f}ms, "
            f"capture={r.get('capture_p99', 0):.3f}ms]"
        )
        if not within:
            failures.append(
                f"{backend}@{refresh_hz}Hz: d_p99={delta:.3f}ms exceeds budget {budget:.3f}ms"
            )

    return len(failures) == 0, failures


def generate_sample_report() -> dict:
    """Generate a synthetic benchmark report for testing the CI gate."""
    return {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "host": os.environ.get("RUNNER_NAME", os.environ.get("COMPUTERNAME", "local")),
        "results": [
            {
                "backend": "dxgi",
                "refresh_hz": 60,
                "budget_ms": 0.17,
                "baseline_p99": 16.67,
                "capture_p99": 16.75,
                "delta_p99": 0.08,
                "within_budget": True,
                "baseline": {"p50": 16.6, "p95": 16.65, "p99": 16.67, "min": 16.5, "max": 16.8, "count": 200},
                "capture": {"p50": 16.7, "p95": 16.73, "p99": 16.75, "min": 16.6, "max": 16.9, "count": 200},
            },
            {
                "backend": "vulkan",
                "refresh_hz": 60,
                "budget_ms": 0.17,
                "baseline_p99": 16.67,
                "capture_p99": 16.72,
                "delta_p99": 0.05,
                "within_budget": True,
                "baseline": {"p50": 16.6, "p95": 16.65, "p99": 16.67, "min": 16.5, "max": 16.8, "count": 200},
                "capture": {"p50": 16.65, "p95": 16.70, "p99": 16.72, "min": 16.55, "max": 16.85, "count": 200},
            },
            {
                "backend": "opengl",
                "refresh_hz": 60,
                "budget_ms": 0.17,
                "baseline_p99": 16.67,
                "capture_p99": 16.80,
                "delta_p99": 0.13,
                "within_budget": True,
                "baseline": {"p50": 16.6, "p95": 16.65, "p99": 16.67, "min": 16.5, "max": 16.8, "count": 200},
                "capture": {"p50": 16.72, "p95": 16.78, "p99": 16.80, "min": 16.62, "max": 16.95, "count": 200},
            },
        ],
    }


def main():
    parser = argparse.ArgumentParser(description="G5 Performance verification CI gate")
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--report", type=str, help="Path to benchmark report JSON file")
    group.add_argument("--smoke-test", action="store_true", help="Run benchmark framework unit tests")
    group.add_argument("--generate-sample", action="store_true", help="Print a sample JSON report to stdout")
    args = parser.parse_args()

    if args.generate_sample:
        json.dump(generate_sample_report(), sys.stdout, indent=2)
        print()
        return 0

    if args.smoke_test:
        return 0 if run_smoke_test() else 1

    # Validate report
    report_path = Path(args.report)
    if not report_path.exists():
        print(f"G5: ❌ Report file not found: {report_path}")
        return 1

    with open(report_path) as f:
        report = json.load(f)

    print(f"G5: Validating benchmark report from {report_path}")
    print(f"    Host: {report.get('host', 'unknown')}")
    print(f"    Timestamp: {report.get('timestamp', 'unknown')}")
    print()

    all_pass, failures = validate_report(report)
    print()

    total = len(report.get("results", []))
    passed = total - len(failures)

    if all_pass:
        print(f"G5: {passed}/{total} backends within budget OK")
        return 0
    else:
        print(f"G5: {passed}/{total} backends within budget FAIL")
        for f_desc in failures:
            print(f"  - {f_desc}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
