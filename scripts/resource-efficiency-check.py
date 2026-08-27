#!/usr/bin/env python3
"""Validate a resource-efficiency report for milestone CI gates."""
import argparse
import json
import sys

MAX_CPU_DELTA_PERCENT = 2.0
MAX_MEMORY_GROWTH_MB = 64.0
MAX_FRAME_TIME_REGRESSION_PERCENT = 1.0


def validate(report):
    errors = []
    for sample in report.get("profiles", []):
        name = sample.get("name", "unknown")
        if sample.get("cpu_delta_percent", 0) > MAX_CPU_DELTA_PERCENT:
            errors.append(f"{name}: CPU delta exceeds {MAX_CPU_DELTA_PERCENT:.1f}%")
        if sample.get("memory_growth_mb", 0) > MAX_MEMORY_GROWTH_MB:
            errors.append(f"{name}: memory growth exceeds {MAX_MEMORY_GROWTH_MB:.0f} MiB")
        if sample.get("frame_time_regression_percent", 0) > MAX_FRAME_TIME_REGRESSION_PERCENT:
            errors.append(f"{name}: frame-time regression exceeds {MAX_FRAME_TIME_REGRESSION_PERCENT:.1f}%")
        if sample.get("p99_frame_time_ms", 0) <= 0:
            errors.append(f"{name}: p99_frame_time_ms must be positive")
    if not report.get("profiles"):
        errors.append("report contains no profiles")
    return errors


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("report")
    args = parser.parse_args()
    with open(args.report, encoding="utf-8") as handle:
        report = json.load(handle)
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
