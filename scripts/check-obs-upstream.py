#!/usr/bin/env python3
"""Check OBS release-note candidates against Rivulet's product vision.

Network failures are reported as unverified rather than treated as parity.
Use --self-test for deterministic coverage and --fixture for offline runs.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "scripts" / "obs-features.json"
VISION = ROOT / "scripts" / "vision-criteria.json"
STATE = ROOT / "scripts" / ".obs-upstream-state.json"
API = "https://api.github.com/repos/obsproject/obs-studio/releases/latest"
FEATURE_WORDS = re.compile(r"\b(add|added|introduce|introduced|support|supports|new|feature|implemented)\b", re.I)


def load_catalog() -> list[dict]:
    data = json.loads(CATALOG.read_text(encoding="utf-8"))
    return data["features"]


def load_vision() -> dict:
    return json.loads(VISION.read_text(encoding="utf-8"))


def fetch_release() -> dict:
    request = urllib.request.Request(API, headers={"Accept": "application/vnd.github+json", "User-Agent": "rivulet-obs-check"})
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.load(response)


def candidate_lines(body: str) -> list[str]:
    lines = []
    for raw in body.splitlines():
        line = raw.strip(" -*\t")
        if line and FEATURE_WORDS.search(line):
            lines.append(line)
    return list(dict.fromkeys(lines))


def classify(lines: list[str], catalog: list[dict]) -> tuple[list[str], list[str]]:
    covered, review = [], []
    for line in lines:
        if any(alias.lower() in line.lower() for item in catalog for alias in [item.get("id", ""), *item.get("aliases", [])]):
            covered.append(line)
        else:
            review.append(line)
    return covered, review


def vision_fit(line: str, vision: dict) -> tuple[str, list[str]]:
    text = line.lower()
    matched = []
    for criterion in vision["criteria"]:
        if any(re.search(rf"\b{re.escape(keyword.lower())}\b", text) for keyword in criterion["keywords"]):
            matched.append(criterion["id"])
    decision = vision["decision"]
    if len(matched) >= decision["strong_fit_minimum"]:
        label = "strong-fit"
    elif len(matched) >= decision["review_minimum"]:
        label = "review"
    else:
        label = "not-aligned"
    return label, matched


def report(release: dict, fixture: bool = False) -> str:
    catalog = load_catalog()
    vision = load_vision()
    lines = candidate_lines(release.get("body", ""))
    covered, review = classify(lines, catalog)
    previous = json.loads(STATE.read_text(encoding="utf-8")).get("tag_name") if STATE.exists() else None
    result = [f"## OBS upstream check — {release.get('tag_name', 'unknown')}", "", f"Source: {'fixture' if fixture else 'GitHub API'}", f"Previous checked release: {previous or 'none'}", f"Feature-note candidates: {len(lines)}", ""]
    if review:
        result += ["### Needs review — vision fit", "", "| Candidate | Vision decision | Matching pillars |", "| --- | --- | --- |"]
        for line in review:
            label, matches = vision_fit(line, vision)
            result.append(f"| {line.replace('|', '\\|')} | **{label}** | {', '.join(matches) or 'none'} |")
        result.append("")
    if covered:
        result += ["### Already represented", *[f"- {line}" for line in covered], ""]
    if not lines:
        result += ["No feature-like release-note lines found.", ""]
    result += ["Vision decisions are advisory. Strong-fit candidates should be evaluated for the roadmap; review candidates need maintainer judgement; not-aligned candidates should not be added solely for parity.", ""]
    result += ["Vision criteria: `scripts/vision-criteria.json`."]
    return "\n".join(result)


def self_test() -> None:
    fixture = {"tag_name": "30.0.0", "body": "- Added a new automated GPU capture\n- Added a new emoji\n- Supports multitrack video"}
    output = report(fixture, fixture=True)
    assert "automated GPU capture" in output
    assert "strong-fit" in output
    assert "not-aligned" in output
    assert "multitrack video" in output
    label, matches = vision_fit("New deterministic WebGPU stream API", load_vision())
    assert label == "strong-fit"
    assert {"deterministic", "modern-rendering", "streamer-value"}.issubset(matches)
    print("OK: OBS upstream checker self-test")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    try:
        release = json.loads(args.fixture.read_text(encoding="utf-8")) if args.fixture else fetch_release()
        print(report(release, fixture=bool(args.fixture)))
        return 0
    except Exception as exc:
        print(f"OBS upstream check unverified: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
