#!/usr/bin/env python3
"""Verify that the README Feature-Parity Checklist covers every OBS feature.

The machine-readable OBS feature catalog lives in ``scripts/obs-features.json``
(one entry per OBS capability, with the aliases used to match the checklist
rows). This script parses the ``Feature-Parity Checklist (vs. OBS)`` table in
``README.md`` and reports every catalog feature without a matching row, so a
new OBS capability cannot silently drop off the roadmap. Rows that match no
catalog entry (Rivulet-specific items such as the AI assistant) are listed as
informational and do not fail the check.

Exit codes: ``0`` = every catalog feature is covered by the checklist,
``1`` = at least one feature is missing, ``2`` = README or catalog could not
be parsed.

Usage:
    scripts/check-parity-checklist.py [--json] [--self-test]
"""

import argparse
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
README = REPO_ROOT / "README.md"
CATALOG = REPO_ROOT / "scripts" / "obs-features.json"

SECTION_MARKER = "Feature-Parity Checklist"
SEPARATOR_CELL_RE = re.compile(r"^:?-{3,}:?$")


def load_catalog(path):
    """Load and validate the JSON feature catalog; return the feature list."""
    data = json.loads(path.read_text(encoding="utf-8"))
    features = data.get("features")
    if not isinstance(features, list):
        raise ValueError(f"{path}: missing 'features' list")
    for feature in features:
        if not isinstance(feature, dict) or not feature.get("id"):
            raise ValueError(f"{path}: every feature needs a non-empty 'id'")
        feature.setdefault("aliases", [])
    return features


def extract_rows(text):
    """Return the OBS-category cells of the parity table, or None if the
    section header is missing. The header row and the ``|---|`` separator are
    skipped."""
    lines = text.splitlines()
    start = None
    for i, line in enumerate(lines):
        if line.strip().startswith("###") and SECTION_MARKER in line:
            start = i
            break
    if start is None:
        return None

    rows = []
    for line in lines[start + 1:]:
        stripped = line.strip()
        if stripped.startswith("## "):
            break
        if not stripped.startswith("|"):
            continue
        cells = [c.strip() for c in stripped.strip("|").split("|")]
        if len(cells) < 2:
            continue
        category = cells[0]
        if not category or category.lower() == "obs category":
            continue
        if SEPARATOR_CELL_RE.fullmatch(category):
            continue
        rows.append(category)
    return rows


def needles(feature):
    """Lowercased search terms for a feature: its id plus its aliases."""
    return [feature["id"].lower()] + [a.lower() for a in feature.get("aliases", [])]


def covered(category, feature):
    """True if the checklist row `category` mentions `feature` (id or alias).

    Matching is word-boundary based so that e.g. the alias ``window`` does not
    match the unrelated row ``Platform parity (Windows/macOS)``."""
    hay = category.lower()
    return any(
        re.search(rf"\b{re.escape(needle)}\b", hay) is not None
        for needle in needles(feature)
    )


def find_missing(rows, features):
    """Catalog features with no matching checklist row."""
    return [f["id"] for f in features if not any(covered(r, f) for r in rows)]


def find_extra(rows, features):
    """Checklist rows that match no catalog entry (Rivulet-specific items)."""
    return [r for r in rows if not any(covered(r, f) for f in features)]


def self_test() -> int:
    """Run the built-in unit tests for the parser and the matching logic."""
    fixture = (
        "# test\n"
        "### \U0001F3AF Feature-Parity Checklist (vs. OBS)\n"
        "| OBS category | Status |\n"
        "| --- | --- |\n"
        "| Display capture | Partial |\n"
        "| Remux & file management | Open (M4) |\n"
        "| AI chat assistant | Open (M9) |\n"
        "## Next section\n"
    )
    failures = []

    def expect(condition, message):
        if condition:
            print(f"  PASS {message}")
        else:
            failures.append(message)
            print(f"  FAIL {message}")

    rows = extract_rows(fixture)
    expect(
        rows == ["Display capture", "Remux & file management", "AI chat assistant"],
        "extract_rows keeps the three data rows and skips header/separator",
    )

    features = [
        {"id": "display-capture", "aliases": ["display"]},
        {"id": "remux", "aliases": ["remux"]},
        {"id": "game-capture", "aliases": ["game"]},
    ]
    expect(
        find_missing(rows, features) == ["game-capture"],
        "find_missing reports only the uncovered feature (game-capture)",
    )
    expect(
        find_extra(rows, features) == ["AI chat assistant"],
        "find_extra lists the Rivulet-specific row as informational",
    )
    expect(
        covered("Remux & file management", {"id": "remux", "aliases": []}),
        "covered matches the id as a substring of the row",
    )
    expect(
        not covered("Display capture", {"id": "x", "aliases": ["monitor"]}),
        "covered does not false-positive on an unrelated row",
    )
    expect(
        not covered(
            "Platform parity (Windows/macOS)",
            {"id": "window-capture", "aliases": ["window"]},
        ),
        "word-boundary matching: 'window' does not match 'Windows' in platform parity",
    )
    expect(
        extract_rows("# no table here") is None,
        "extract_rows returns None when the section is missing",
    )

    if failures:
        print(f"\nself-test FAILED: {len(failures)} assertion(s)")
        return 1
    print("\nself-test OK")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.strip())
    ap.add_argument("--json", action="store_true", help="emit one JSON document on stdout")
    ap.add_argument("--self-test", action="store_true", help="run the built-in unit tests")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    if not README.is_file() or not CATALOG.is_file():
        missing = README if not README.is_file() else CATALOG
        print(f"error: {missing} not found", file=sys.stderr)
        return 2

    try:
        features = load_catalog(CATALOG)
        rows = extract_rows(README.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    if rows is None:
        print(f"error: '{SECTION_MARKER}' section not found in {README}", file=sys.stderr)
        headers = [
            l.strip() for l in README.read_text(encoding="utf-8").splitlines()
            if l.strip().startswith("#")
        ]
        print(
            f"debug: {len(headers)} markdown headings; sample: {headers[:8]}",
            file=sys.stderr,
        )
        return 2

    missing = find_missing(rows, features)
    extra = find_extra(rows, features)
    ok = not missing

    if args.json:
        print(
            json.dumps(
                {
                    "ok": ok,
                    "catalog_features": len(features),
                    "checklist_rows": len(rows),
                    "missing": missing,
                    "extra_rows": extra,
                },
                indent=2,
            )
        )
    else:
        print(
            f"== OBS feature-parity check: {len(features)} catalog features vs "
            f"{len(rows)} checklist rows =="
        )
        if missing:
            print("Missing (no matching checklist row):")
            for feature_id in missing:
                print(f"  - {feature_id}")
        if extra:
            print("Checklist rows not in the OBS catalog (Rivulet-specific):")
            for row in extra:
                print(f"  - {row}")
        print()
        if ok:
            print("OK: every OBS feature in the catalog is covered by the checklist")
        else:
            print(
                f"FAIL: {len(missing)} OBS feature(s) missing from the Feature-Parity Checklist",
                file=sys.stderr,
            )

    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
