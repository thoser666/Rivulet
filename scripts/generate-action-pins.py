#!/usr/bin/env python3
"""Regenerate the action-pin table in docs/ci-action-pins.md from the workflows.

The single source of truth is the `uses:` lines in `.github/workflows/*.yml`,
which carry both the pinned commit SHA and the `# version` comment. The table in
the doc is derived from them, so it must not be edited by hand.

Usage:
    scripts/generate-action-pins.py          # rewrite the table in place
    scripts/generate-action-pins.py --check  # exit non-zero if the doc drifts
"""

import difflib
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOW_DIR = REPO_ROOT / ".github" / "workflows"
DOC = REPO_ROOT / "docs" / "ci-action-pins.md"

START_MARKER = "<!-- action-pins-table:start -->"
END_MARKER = "<!-- action-pins-table:end -->"

USES_RE = re.compile(r"^\s*uses:\s*(.+?)\s*$")


def parse_workflows():
    """Return {action: (sha, version, {workflow filenames})} from the workflows."""
    pins = {}
    for wf in sorted(WORKFLOW_DIR.glob("*.yml")):
        for line in wf.read_text(encoding="utf-8").splitlines():
            match = USES_RE.match(line)
            if not match:
                continue
            raw = match.group(1)
            ref_part, _, comment = raw.partition("#")
            ref = ref_part.strip()
            if not ref or ref.startswith("./"):
                continue  # local reusable workflow, not a third-party pin
            if "@" not in ref:
                continue
            action, sha = ref.rsplit("@", 1)
            version = comment.strip()
            if action in pins:
                prev_sha, prev_version, files = pins[action]
                if (prev_sha, prev_version) != (sha, version):
                    sys.exit(
                        f"conflicting pins for {action}: "
                        f"{prev_sha} #{prev_version or '?'} vs {sha} #{version or '?'}"
                    )
                files.add(wf.name)
            else:
                pins[action] = (sha, version, {wf.name})
    return pins


def render_table(pins):
    lines = [
        "| Action | Version | Pinned SHA | Used in |",
        "| --- | --- | --- | --- |",
    ]
    for action in sorted(pins):
        sha, version, files = pins[action]
        used_in = ", ".join(sorted(files))
        lines.append(f"| `{action}` | `{version}` | `{sha}` | {used_in} |")
    return "\n".join(lines)


def regenerate(pins):
    doc = DOC.read_text(encoding="utf-8")
    if START_MARKER not in doc or END_MARKER not in doc:
        sys.exit(f"{DOC} is missing the {START_MARKER!r}/{END_MARKER!r} markers")
    prefix_end = doc.index(START_MARKER) + len(START_MARKER)
    suffix_start = doc.index(END_MARKER, prefix_end)
    middle = "\n" + render_table(pins) + "\n"
    return doc[:prefix_end] + middle + doc[suffix_start:]


def main():
    pins = parse_workflows()
    if not pins:
        sys.exit("no third-party action pins found in .github/workflows")
    new_doc = regenerate(pins)
    if "--check" in sys.argv[1:]:
        current = DOC.read_text(encoding="utf-8")
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
    DOC.write_text(new_doc, encoding="utf-8")
    print(f"Wrote {DOC} ({len(pins)} actions).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
