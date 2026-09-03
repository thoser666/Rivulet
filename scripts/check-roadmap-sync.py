#!/usr/bin/env python3
"""Check that the roadmap docs stay in sync with the GitHub milestones.

The milestone sequence is owned by GitHub: the milestones carry the canonical
`M<n> – Title` names, and the README "Milestone overview" table plus
`docs/milestone-quality-gates.md` must mirror that ordering and those names.
A milestone renumber (the M6–M11 shift) or a rename therefore has to touch
three places; this checker makes a forgotten one fail CI instead of shipping
a README whose rows, badges, and gates disagree with the repository.

What is compared:

1. **README overview table** (rows `| M0 – Title | Focus | … | badge |`): the
   milestone numbers must be a contiguous ascending run, and each row's badge
   (GitHub milestone id embedded in the shields URL) must match the GitHub
   milestone that carries exactly that `M<n> – Title`.

2. **docs/milestone-quality-gates.md**: the resource-efficiency table
   (`| M<n> | … |`) must list the same milestone numbers in the same order as
   the README table, and every milestone at/after M2 must have a
   `### M<n>:` quality-gate section. M0/M1 predate the gates document and are
   deliberately exempt from the section requirement.

3. **GitHub milestones** (live REST call in CI, or a `--fixture` JSON for
   offline runs/tests): the milestone titles must equal the README labels
   exactly, the badge id of each README row must point at that milestone, and
   the GitHub `M<n>` prefixes must form the same contiguous set as the docs.

Exit codes: `0` everything is in sync; `1` a drift was found; `2` the local
docs are fine but the GitHub comparison could not run (API failure), so the
live part is unverified.

Usage:
    scripts/check-roadmap-sync.py                 # local + live GitHub check
    scripts/check-roadmap-sync.py --fixture f.json  # local + fixture (offline)
    scripts/check-roadmap-sync.py --self-test     # deterministic unit tests
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

for _stream in (sys.stdout, sys.stderr):
    _reconfigure = getattr(_stream, "reconfigure", None)
    if _reconfigure is not None:
        try:
            _reconfigure(encoding="utf-8")
        except (ValueError, OSError):
            pass

REPO_ROOT = Path(__file__).resolve().parent.parent
README = REPO_ROOT / "README.md"
GATES = REPO_ROOT / "docs" / "milestone-quality-gates.md"

# Milestones M0/M1 are the recording foundation and predate the quality-gates
# document, which starts its milestone-specific sections at M2.
FIRST_GATE_SECTION = 2

EN_DASH = "\u2013"
MILESTONE_TITLE_RE = re.compile(rf"^M(\d+)\s*{EN_DASH}\s*(.+)$")
README_ROW_RE = re.compile(r"^\|\s*M(\d+)\s*[–-]\s*([^|]+?)\s*\|\s*([^|]*?)\s*\|")
BADGE_ID_RE = re.compile(r"milestones%2F(\d+)")
GATES_ROW_RE = re.compile(r"^\|\s*M(\d+)\s*\|")
GATES_SECTION_RE = re.compile(r"^###\s*M(\d+)\s*:")


def parse_readme_table(text: str) -> list[dict]:
    """Return ``[{n, title, badge, row}]`` for the Milestone overview rows."""
    rows = []
    lines = text.splitlines()
    in_table = False
    for line in lines:
        if line.strip().startswith("### Milestone overview"):
            in_table = True
            continue
        if in_table and line.strip().startswith("###"):
            break
        if not in_table:
            continue
        stripped = line.strip()
        if not stripped.startswith("| M"):
            continue
        match = README_ROW_RE.match(stripped)
        if not match:
            continue
        n = int(match.group(1))
        title = f"M{n} {EN_DASH} {match.group(2).strip()}"
        cell = match.group(3)
        badge_match = BADGE_ID_RE.search(stripped)
        rows.append(
            {
                "n": n,
                "title": title,
                "badge": int(badge_match.group(1)) if badge_match else None,
                "row": cell.strip(),
            }
        )
    return rows


def parse_gates(text: str) -> tuple[list[int], list[int]]:
    """Return ``(resource_rows, section_numbers)`` of the gates document."""
    resource = []
    sections = []
    for line in text.splitlines():
        stripped = line.strip()
        row_match = GATES_ROW_RE.match(stripped)
        if row_match and "|" in stripped[1:]:
            resource.append(int(row_match.group(1)))
        section_match = GATES_SECTION_RE.match(stripped)
        if section_match:
            sections.append(int(section_match.group(1)))
    return resource, sections


def is_contiguous(numbers: list[int]) -> bool:
    return numbers == list(range(numbers[0], numbers[-1] + 1))


def local_checks(readme_rows: list[dict], gates_resource: list[int], gates_sections: list[int]) -> list[str]:
    """Return a list of drift messages (empty when the docs agree)."""
    problems = []

    readme_numbers = [row["n"] for row in readme_rows]
    if not readme_numbers:
        problems.append("README Milestone overview table has no milestone rows")
        return problems
    if readme_numbers != sorted(readme_numbers):
        problems.append("README milestone rows are not in ascending order")
    if len(readme_numbers) != len(set(readme_numbers)):
        problems.append("README milestone rows contain duplicate numbers")
    elif not is_contiguous(readme_numbers):
        problems.append(
            f"README milestone rows are not contiguous: {readme_numbers}"
        )
    for row in readme_rows:
        if row["badge"] is None:
            problems.append(f"README row M{row['n']} has no GitHub milestone badge")
    for row in readme_rows:
        if not MILESTONE_TITLE_RE.match(row["title"]):
            problems.append(f"README row has a malformed title: {row['title']!r}")

    if gates_resource != readme_numbers:
        problems.append(
            f"gates resource table ({gates_resource}) differs from the README "
            f"milestone order ({readme_numbers})"
        )
    missing_sections = [
        n
        for n in readme_numbers
        if n >= FIRST_GATE_SECTION and n not in gates_sections
    ]
    if missing_sections:
        problems.append(
            "gates doc is missing a `### M<n>:` quality-gate section for "
            f"milestone(s) {missing_sections}"
        )
    extra_sections = [
        n for n in gates_sections if n not in readme_numbers
    ]
    if extra_sections:
        problems.append(
            f"gates doc has quality-gate sections for unknown milestone(s) "
            f"{extra_sections}"
        )
    if gates_sections != sorted(gates_sections):
        problems.append("gates doc quality-gate sections are not in ascending order")
    return problems


def github_checks(readme_rows: list[dict], milestones: list[dict]) -> list[str]:
    """Compare README rows/badges against the GitHub milestone list."""
    problems = []
    by_number = {m["number"]: m["title"] for m in milestones}

    by_title = {}
    for milestone in milestones:
        match = MILESTONE_TITLE_RE.match(milestone["title"])
        if not match:
            continue
        n = int(match.group(1))
        if n in by_title:
            problems.append(f"GitHub has duplicate M{n} milestones")
        by_title[milestone["title"]] = milestone["number"]

    for row in readme_rows:
        number = by_title.get(row["title"])
        if number is None:
            problems.append(
                f"no GitHub milestone is titled {row['title']!r} (README table)"
            )
            continue
        if row["badge"] != number:
            problems.append(
                f"README M{row['n']} badge points at GitHub milestone "
                f"#{row['badge']} but {row['title']!r} is #{number}"
            )
        if by_number.get(number) != row["title"]:
            problems.append(
                f"GitHub milestone #{number} title {by_number.get(number)!r} "
                f"differs from the README label {row['title']!r}"
            )

    readme_numbers = [row["n"] for row in readme_rows]
    github_numbers = sorted(
        int(match.group(1))
        for milestone in milestones
        if (match := MILESTONE_TITLE_RE.match(milestone["title"]))
    )
    if github_numbers != readme_numbers:
        problems.append(
            f"GitHub milestone prefixes ({github_numbers}) differ from the "
            f"README milestone set ({readme_numbers})"
        )
    if github_numbers and not is_contiguous(github_numbers):
        problems.append(f"GitHub milestone prefixes are not contiguous: {github_numbers}")
    return problems


def fetch_milestones(token: str | None, repo: str) -> list[dict] | None:
    """GET the milestone list; return None when the API is unreachable."""
    url = f"https://api.github.com/repos/{repo}/milestones?state=all&per_page=100"
    headers = {"Accept": "application/vnd.github+json", "User-Agent": "rivulet-roadmap-sync"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            return json.load(response)
    except (urllib.error.HTTPError, urllib.error.URLError, OSError, ValueError):
        return None


def resolve_repo(override: str | None) -> str:
    if override:
        return override
    env = os.environ.get("GITHUB_REPOSITORY")
    if env:
        return env
    return "thoser666/Rivulet"


def load_fixture(path: Path) -> list[dict]:
    return json.loads(path.read_text(encoding="utf-8"))


def report(readme_rows, gates_resource, gates_sections, milestones=None) -> tuple[list[str], list[str]]:
    """Return ``(problems, notes)`` for the given sources."""
    problems = local_checks(readme_rows, gates_resource, gates_sections)
    notes = []
    if milestones is not None:
        problems += github_checks(readme_rows, milestones)
    else:
        notes.append("GitHub comparison skipped (offline: no milestone list given)")
    return problems, notes


def self_test() -> int:
    readme_ok = """\
### Milestone overview

| Milestone | Focus | Status | Badge |
| --- | --- | --- | --- |
| M0 – Recording Foundation | Capture | ✅ Done | [![M0](https://img.shields.io/badge/milestones%2F4)](https://api.github.com/repos/x/Rivulet/milestones/4) |
| M1 – Solid Recording | Audio | ✅ Done | [![M1](https://img.shields.io/badge/milestones%2F7)](https://api.github.com/repos/x/Rivulet/milestones/7) |
| M2 – Scenes & Composition | Scenes | 📅 Planned | [![M2](https://img.shields.io/badge/milestones%2F1)](https://api.github.com/repos/x/Rivulet/milestones/1) |
"""
    gates_ok = """\
| Milestone | Evidence |
| --- | --- |
| M0 | Capture baseline |
| M1 | Encoder overhead |
| M2 | Scene composition |

### M2: Scenes and Composition
- keyboard focus
"""
    milestones_ok = [
        {"number": 4, "title": "M0 \u2013 Recording Foundation"},
        {"number": 7, "title": "M1 \u2013 Solid Recording"},
        {"number": 1, "title": "M2 \u2013 Scenes & Composition"},
    ]

    cases = []

    def expect(name: str, condition: bool) -> None:
        cases.append((name, condition))

    rows = parse_readme_table(readme_ok)
    resource, sections = parse_gates(gates_ok)
    expect("README rows parse with badge-less rows ignored", len(rows) == 3)
    expect("gates resource rows parse", resource == [0, 1, 2])
    expect("gates sections parse", sections == [2])

    problems, _ = report(rows, resource, sections, milestones_ok)
    expect("happy path has no drift", problems == [])

    # Drift: README renamed a milestone without renaming GitHub.
    drifted_rows = [
        {**row, "title": "M2 \u2013 Scenes"} if row["n"] == 2 else row for row in rows
    ]
    problems, _ = report(drifted_rows, resource, sections, milestones_ok)
    expect("renamed README label is caught", any("no GitHub milestone is titled" in p for p in problems))

    # Drift: README badge points at the wrong GitHub milestone number.
    badge_rows = [
        {**row, "badge": row["badge"] + 100 if row["badge"] else None} for row in rows
    ]
    problems, _ = report(badge_rows, resource, sections, milestones_ok)
    expect("wrong badge id is caught", any("badge points at" in p for p in problems))

    # Drift: gates resource table drops a milestone.
    problems, _ = report(rows, [0, 2], sections, milestones_ok)
    expect("gates resource drop is caught", any("gates resource table" in p for p in problems))

    # Drift: gates doc lost the M2 quality-gate section.
    problems, _ = report(rows, resource, [], milestones_ok)
    expect("missing gates section is caught", any("missing a `### M" in p for p in problems))

    # Drift: GitHub gained an extra milestone prefix the docs do not know.
    extra = milestones_ok + [{"number": 99, "title": "M3 \u2013 Streaming"}]
    problems, _ = report(rows, resource, sections, extra)
    expect("extra GitHub milestone is caught", any("differ from the" in p for p in problems))

    # Drift: README rows out of order.
    problems, _ = report(list(reversed(rows)), resource, sections, milestones_ok)
    expect("out-of-order README rows are caught", any("ascending" in p or "contiguous" in p for p in problems))

    failures = [name for name, ok in cases if not ok]
    if failures:
        for name in failures:
            print(f"FAIL: {name}", file=sys.stderr)
        return 1
    print("OK: roadmap-sync checker self-test")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate roadmap docs against the GitHub milestones")
    parser.add_argument("--fixture", type=Path, help="offline milestone list (JSON array of {number,title})")
    parser.add_argument("--repo", help="owner/repo (default: GITHUB_REPOSITORY or thoser666/Rivulet)")
    parser.add_argument("--self-test", action="store_true", help="run deterministic unit tests and exit")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    readme_text = README.read_text(encoding="utf-8")
    gates_text = GATES.read_text(encoding="utf-8")
    rows = parse_readme_table(readme_text)
    resource, sections = parse_gates(gates_text)

    milestones = None
    if args.fixture:
        try:
            milestones = load_fixture(args.fixture)
        except (OSError, json.JSONDecodeError) as exc:
            print(f"roadmap-sync: cannot read fixture {args.fixture}: {exc}", file=sys.stderr)
            return 2
    else:
        token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
        milestones = fetch_milestones(token, resolve_repo(args.repo))
        if milestones is None:
            print(
                "roadmap-sync: GitHub milestone list unreachable — the live "
                "comparison is unverified (local doc checks still ran).",
                file=sys.stderr,
            )

    problems, notes = report(rows, resource, sections, milestones)
    for note in notes:
        print(f"note: {note}")
    for problem in problems:
        print(f"error: {problem}")
    if problems:
        return 1
    if milestones is None:
        # Local docs are consistent, but without the live milestone list the
        # GitHub half of the contract could not be verified.
        print("OK: README table and gates doc agree with each other (GitHub comparison unverified)")
        return 2
    print("OK: README milestone table and docs/milestone-quality-gates.md are in sync with the GitHub milestones")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
