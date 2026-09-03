#!/usr/bin/env python3
"""Check OBS release-note candidates against Rivulet's product vision."""
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
DOC = ROOT / "docs" / "obs-vision-candidates.md"
API = "https://api.github.com/repos/obsproject/obs-studio/releases/latest"
FEATURE_WORDS = re.compile(r"\b(add|added|introduce|introduced|support|supports|new|feature|implemented)\b", re.I)


def load_catalog() -> list[dict]:
    return json.loads(CATALOG.read_text(encoding="utf-8"))["features"]


def load_vision() -> dict:
    return json.loads(VISION.read_text(encoding="utf-8"))


def fetch_release() -> dict:
    request = urllib.request.Request(API, headers={"Accept": "application/vnd.github+json", "User-Agent": "rivulet-obs-check"})
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.load(response)


def candidate_lines(body: str) -> list[str]:
    return list(dict.fromkeys(line.strip(" -*\t") for line in body.splitlines() if line.strip(" -*\t") and FEATURE_WORDS.search(line)))


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
    matched = [criterion["id"] for criterion in vision["criteria"] if any(re.search(rf"\b{re.escape(keyword.lower())}\b", text) for keyword in criterion["keywords"])]
    decision = vision["decision"]
    if len(matched) >= decision["strong_fit_minimum"]:
        return "strong-fit", matched
    if len(matched) >= decision["review_minimum"]:
        return "review", matched
    return "not-aligned", matched


def strong_fit_rows(lines: list[str], vision: dict) -> list[tuple[str, list[str]]]:
    rows = []
    for line in lines:
        label, matches = vision_fit(line, vision)
        if label == "strong-fit":
            rows.append((line, matches))
    return rows


def candidate_doc_fragment(release: dict, rows: list[tuple[str, list[str]]]) -> str:
    if not rows:
        return "<!-- No new strong-fit candidates detected in the latest checked release. -->"
    version = release.get("tag_name", "unknown")
    lines = [f"### {version}", "", "| Candidate | Matching vision pillars |", "| --- | --- |"]
    lines.extend(f"| {line.replace('|', '\\|')} | {', '.join(matches)} |" for line, matches in rows)
    return "\n".join(lines)


def update_candidate_doc(release: dict, rows: list[tuple[str, list[str]]]) -> None:
    if not DOC.exists():
        return
    text = DOC.read_text(encoding="utf-8")
    start = "<!-- OBS-VISION-CANDIDATES:START -->"
    end = "<!-- OBS-VISION-CANDIDATES:END -->"
    if start not in text or end not in text:
        raise ValueError(f"{DOC}: candidate markers missing")
    fragment = candidate_doc_fragment(release, rows)
    # The END marker must be written back too — dropping it (as an earlier
    # version did) leaves the doc with only a START marker, which makes the
    # next run fail with "candidate markers missing" and silently empties
    # the weekly report (masked by continue-on-error in the workflow).
    updated = text.split(start)[0] + start + "\n" + fragment + "\n" + end + text.split(end, 1)[1]
    DOC.write_text(updated, encoding="utf-8")


def report(release: dict, fixture: bool = False) -> str:
    vision = load_vision()
    lines = candidate_lines(release.get("body", ""))
    covered, review = classify(lines, load_catalog())
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
    result += ["Vision decisions are advisory. Strong-fit candidates are copied to `docs/obs-vision-candidates.md` for maintainer review; they are not automatically added to the roadmap.", "", "Vision criteria: `scripts/vision-criteria.json`."]
    return "\n".join(result)


def self_test() -> None:
    fixture = {"tag_name": "30.0.0", "body": "- Added a new automated GPU capture\n- Added a new emoji\n- Supports multitrack video"}
    vision = load_vision()
    output = report(fixture, fixture=True)
    assert "automated GPU capture" in output and "strong-fit" in output and "not-aligned" in output
    assert strong_fit_rows(candidate_lines(fixture["body"]), vision)
    label, matches = vision_fit("New deterministic WebGPU stream API", vision)
    assert label == "strong-fit" and {"deterministic", "modern-rendering", "streamer-value"}.issubset(matches)
    print("OK: OBS upstream checker self-test")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--update-doc", action="store_true", help="update docs/obs-vision-candidates.md with strong-fit candidates")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    try:
        release = json.loads(args.fixture.read_text(encoding="utf-8")) if args.fixture else fetch_release()
        if args.update_doc:
            rows = strong_fit_rows(candidate_lines(release.get("body", "")), load_vision())
            update_candidate_doc(release, rows)
        print(report(release, fixture=bool(args.fixture)))
        return 0
    except Exception as exc:
        print(f"OBS upstream check unverified: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
