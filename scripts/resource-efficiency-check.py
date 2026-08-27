#!/usr/bin/env python3
"""Validate resource-efficiency reports for milestone CI gates."""
import argparse
import json
import sys

MAX_CPU_DELTA_PERCENT = 2.0
MAX_MEMORY_GROWTH_MB = 64.0
MAX_FRAME_TIME_REGRESSION_PERCENT = 1.0

REQUIRED_FIELDS = (
    "cpu_delta_percent",
    "memory_growth_mb",
    "frame_time_regression_percent",
    "p99_frame_time_ms",
)


def validate(report):
    errors = []
    profiles = report.get("profiles")
    if not isinstance(profiles, list) or not profiles:
        return ["report contains no profiles"]

    for sample in profiles:
        name = sample.get("name", "unknown")
        missing = [field for field in REQUIRED_FIELDS if field not in sample]
        if missing:
            errors.append(f"{name}: missing fields: {', '.join(missing)}")
            continue
        for field in REQUIRED_FIELDS:
            if not isinstance(sample[field], (int, float)):
                errors.append(f"{name}: {field} must be numeric")
        if any(error.startswith(f"{name}:") for error in errors[-len(REQUIRED_FIELDS):]):
            continue
        if sample["cpu_delta_percent"] > MAX_CPU_DELTA_PERCENT:
            errors.append(f"{name}: CPU delta exceeds {MAX_CPU_DELTA_PERCENT:.1f}%")
        if sample["memory_growth_mb"] > MAX_MEMORY_GROWTH_MB:
            errors.append(f"{name}: memory growth exceeds {MAX_MEMORY_GROWTH_MB:.0f} MiB")
        if sample["frame_time_regression_percent"] > MAX_FRAME_TIME_REGRESSION_PERCENT:
            errors.append(f"{name}: frame-time regression exceeds {MAX_FRAME_TIME_REGRESSION_PERCENT:.1f}%")
        if sample["p99_frame_time_ms"] <= 0:
            errors.append(f"{name}: p99_frame_time_ms must be positive")
        if "p95_frame_time_ms" in sample and sample["p95_frame_time_ms"] <= 0:
            errors.append(f"{name}: p95_frame_time_ms must be positive")
        if "one_percent_low_fps" in sample and sample["one_percent_low_fps"] <= 0:
            errors.append(f"{name}: one_percent_low_fps must be positive")
        if "gpu_utilization_percent" in sample and not 0 <= sample["gpu_utilization_percent"] <= 100:
            errors.append(f"{name}: gpu_utilization_percent must be between 0 and 100")

    return errors


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("report")
    args = parser.parse_args()
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
