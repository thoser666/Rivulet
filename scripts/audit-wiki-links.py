#!/usr/bin/env python3
"""Audit wiki links into the repository documentation.

Every wiki page may link into the versioned repo docs
(https://github.com/thoser666/Rivulet/blob/develop/...). GitHub renders those
pages from markdown, so a link is only valid when

1. the target file exists on `develop`, and
2. an explicit `#anchor` matches a real heading (GitHub slugs: lowercase,
   spaces -> hyphens, punctuation stripped, dedup with -1/-2, ...).

Heading drift is invisible to plain HTTP checks (GitHub always returns 200
for the file), which is exactly what this script catches. Run against a
checkout, it clones nothing: it resolves the repo files straight from the
working tree of the repository the wiki clone lives in, or (with
`--develop-from origin`) from the remote develop branch via `git show`.

Exit codes: 0 = all links valid; 1 = broken links found; 2 = usage error.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

REPO_LINK_RE = re.compile(
    r"https://github\.com/thoser666/Rivulet/blob/develop/([^\)#\s]+)(?:#([^\)\s]+))?"
)
HEADING_RE = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")


def github_slug(text: str) -> str:
    """Reproduce GitHub's heading-anchor slug algorithm (the common cases)."""
    slug = text.strip().lower()
    slug = re.sub(r"[^\w\- ]", "", slug, flags=re.UNICODE)
    slug = slug.replace(" ", "-")
    return slug


def slugs_for(text: str) -> list[str]:
    """Slugs GitHub could generate for a heading, including dedup variants."""
    base = github_slug(text)
    return [base, base + "-1", base + "-2", base + "-3"]


def load_repo_file(repo_root: Path, rel: str, from_git: bool) -> str | None:
    path = repo_root / rel
    if from_git:
        result = subprocess.run(
            ["git", "show", f"origin/develop:{rel}"],
            cwd=repo_root,
            capture_output=True,
        )
        if result.returncode != 0:
            return None
        return result.stdout.decode("utf-8", errors="replace")
    try:
        return path.read_text(encoding="utf-8")
    except OSError:
        return None


def heading_anchors(markdown: str) -> set[str]:
    anchors: set[str] = set()
    used: set[str] = set()
    for line in markdown.splitlines():
        match = HEADING_RE.match(line)
        if not match:
            continue
        for candidate in slugs_for(match.group(2)):
            if candidate not in used:
                used.add(candidate)
                anchors.add(candidate)
                break
    return anchors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wiki", nargs="?", default=".", help="wiki checkout dir")
    parser.add_argument(
        "--repo",
        default=None,
        help="repository checkout that contains docs/ (default: resolve from "
        "the wiki checkout's parent, else the CWD)",
    )
    parser.add_argument(
        "--develop-from-origin",
        action="store_true",
        help="resolve target files from origin/develop instead of the working tree",
    )
    args = parser.parse_args()

    wiki = Path(args.wiki)
    repo_root = (
        Path(args.repo).resolve()
        if args.repo
        else next(
            (parent.resolve() for parent in [wiki, *wiki.parents] if (parent / "docs").is_dir()),
            Path.cwd(),
        )
    )
    if not (repo_root / "docs").is_dir():
        print(f"error: {repo_root} does not look like the repository checkout", file=sys.stderr)
        return 2

    broken: list[str] = []
    checked = 0
    for page in sorted(wiki.glob("*.md")):
        text = page.read_text(encoding="utf-8")
        for match in REPO_LINK_RE.finditer(text):
            checked += 1
            rel, anchor = match.group(1), match.group(2)
            markdown = load_repo_file(repo_root, rel, args.develop_from_origin)
            if markdown is None:
                broken.append(f"{page.name}: missing file on develop: {rel}")
                continue
            if anchor:
                if anchor not in heading_anchors(markdown):
                    broken.append(f"{page.name}: anchor #{anchor} not found in {rel}")

    print(f"checked {checked} repo-doc link(s) across {len(list(wiki.glob('*.md')))} wiki page(s)")
    if broken:
        print("wiki repo-doc link audit FAILED:")
        for item in broken:
            print(f"- {item}")
        return 1
    print("wiki repo-doc link audit passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
