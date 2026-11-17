#!/usr/bin/env python3
"""Regenerate the action-pin table in docs/ci-action-pins.md from the workflows.

The single source of truth is the `uses:` lines in `.github/workflows/*.yml`,
which carry both the pinned commit SHA and the `# version` comment. The table in
the doc is derived from them, so it must not be edited by hand.

Usage:
    scripts/generate-action-pins.py          # rewrite the table in place
    scripts/generate-action-pins.py --check  # exit non-zero if the doc drifts
    scripts/generate-action-pins.py --help   # print this text, write nothing
"""

import difflib
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from action_pins import REPO_ROOT, parse_workflows  # noqa: E402

DOC = REPO_ROOT / "docs" / "ci-action-pins.md"

START_MARKER = "<!-- action-pins-table:start -->"
END_MARKER = "<!-- action-pins-table:end -->"


def read_doc():
    """The doc verbatim, without newline translation.

    `read_text` folds every CRLF into LF, so the rewrite hands back a file whose
    bytes differ from the checkout while its content does not: git then reports
    a modification it cannot diff. Reading and writing the bytes keeps whatever
    endings the file already had.
    """
    return DOC.read_bytes().decode("utf-8")


def newline_of(doc):
    """The doc's line ending, so a generated row matches the file around it."""
    return "\r\n" if "\r\n" in doc else "\n"


def render_table(pins, nl):
    lines = [
        "| Action | Version | Pinned SHA | Used in |",
        "| --- | --- | --- | --- |",
    ]
    for action in sorted(pins):
        sha, version, files = pins[action]
        used_in = ", ".join(sorted(files))
        lines.append(f"| `{action}` | `{version}` | `{sha}` | {used_in} |")
    return nl.join(lines)


def regenerate(pins):
    doc = read_doc()
    if START_MARKER not in doc or END_MARKER not in doc:
        sys.exit(f"{DOC} is missing the {START_MARKER!r}/{END_MARKER!r} markers")
    nl = newline_of(doc)
    prefix_end = doc.index(START_MARKER) + len(START_MARKER)
    suffix_start = doc.index(END_MARKER, prefix_end)
    middle = nl + render_table(pins, nl) + nl
    return doc[:prefix_end] + middle + doc[suffix_start:]


def main():
    args = sys.argv[1:]
    if "--help" in args or "-h" in args:
        print(__doc__.strip())
        return 0

    pins = parse_workflows()
    if not pins:
        sys.exit("no third-party action pins found in .github/workflows")
    new_doc = regenerate(pins)
    if "--check" in args:
        current = read_doc()
        if new_doc == current:
            print("Action pin table is up to date.")
            return 0
        print(
            f"{DOC} drifted. Re-run scripts/generate-action-pins.py to regenerate.",
            file=sys.stderr,
        )
        for line in difflib.unified_diff(
            current.splitlines(),
            new_doc.splitlines(),
            fromfile=str(DOC),
            tofile=str(DOC) + " (regenerated)",
            lineterm="",
        ):
            print(line, file=sys.stderr)
        return 1
    DOC.write_bytes(new_doc.encode("utf-8"))
    print(f"Wrote {DOC} ({len(pins)} actions).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
