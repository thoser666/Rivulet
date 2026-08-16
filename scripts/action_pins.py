"""Shared helpers for deriving third-party action pins from the workflows.

Kept separate so `generate-action-pins.py` (table) and `check-action-pins.py`
(staleness) parse the `uses:` lines identically.
"""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOW_DIR = REPO_ROOT / ".github" / "workflows"

USES_RE = re.compile(r"^\s*uses:\s*(.+?)\s*$")


def parse_workflows():
    """Return ``{action: (sha, version, {workflow filenames})}``.

    Reads every ``uses:`` line in ``.github/workflows/*.yml``, keeps the pinned
    commit SHA and the ``# version`` comment, and skips local reusable
    workflows (``./`` refs).
    """
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
                continue
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
