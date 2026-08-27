#!/usr/bin/env python3
"""Check new OBS releases for feature-parity candidates.

Network failures are reported as unverified rather than treated as parity.
Use --self-test for deterministic CI coverage and --fixture for offline runs.
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
STATE = ROOT / "scripts" / ".obs-upstream-state.json"
API = "https://api.github.com/repos/obsproject/obs-studio/releases/latest"
FEATURE_WORDS = re.compile(r"\b(add|added|introduce|introduced|support|supports|new|feature|implemented)\b", re.I)


def load_catalog() -> list[dict]:
    return json.loads(CATALOG.read_text(encoding="utf-8"))


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
    known = " ".join((item.get("id", "") + " " + " ".join(item.get("aliases", []))) for item in catalog).lower()
    covered, review = [], []
    for line in lines:
        (covered if any(token and token.lower() in line.lower() or token.lower() in known and token.lower() in line.lower() for token in [line]) else review).append(line)
    # A release-note line is only a candidate; matching the existing catalog is
    # reported separately so maintainers can decide whether it is truly new.
    covered = [line for line in lines if any(alias.lower() in line.lower() for item in catalog for alias in [item.get("id", ""), *item.get("aliases", [])])]
    review = [line for line in lines if line not in covered]
    return covered, review


def report(release: dict, fixture: bool = False) -> str:
    catalog = load_catalog()
    lines = candidate_lines(release.get("body", ""))
    covered, review = classify(lines, catalog)
    previous = json.loads(STATE.read_text(encoding="utf-8")).get("tag_name") if STATE.exists() else None
    title = f"## OBS upstream check — {release.get('tag_name', 'unknown')}"
    mode = "fixture" if fixture else "GitHub API"
    result = [title, "", f"Source: {mode}", f"Previous checked release: {previous or 'none'}", f"Feature-note candidates: {len(lines)}", ""]
    if review:
        result += ["### Needs review", *[f"- {line}" for line in review], ""]
    if covered:
        result += ["### Already represented", *[f"- {line}" for line in covered], ""]
    if not lines:
        result += ["No feature-like release-note lines found.", ""]
    result += ["This report is advisory; maintainers must classify candidates in `scripts/obs-features.json` and the README."]
    return "\n".join(result)


def self_test() -> None:
    fixture = {"tag_name": "30.0.0", "body": "- Added a new projector mode\n- Fixed a crash\n- Supports multitrack video"}
    output = report(fixture, fixture=True)
    assert "projector mode" in output
    assert "multitrack video" in output
    assert "Needs review" in output
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
