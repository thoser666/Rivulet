#!/usr/bin/env python3
"""Audit wiki links: interwiki, repo docs, and external URLs.

Wiki pages link to three kinds of targets, and all of them drift:

1. **Interwiki links** — `[Page](Page)` / `[Page](Page#anchor)` resolve to
   another page in the same wiki checkout; the anchor must match a heading
   there.
2. **Repo-doc links** — `https://github.com/thoser666/Rivulet/blob/develop/…`
   must point at an existing file on `develop`, and an explicit `#anchor`
   must match a heading (GitHub slugs: lowercase, spaces -> hyphens,
   punctuation stripped, dedup with -1/-2, ...). GitHub renders those pages
   with HTTP 200 even when the anchor is gone, which plain HTTP checks miss.
3. **External URLs** — every other `http(s)` markdown link must be reachable
   (HEAD first, GET as fallback). Checked with a small timeout and optional
   offline skip; GitHub-blob links are handled by rule 2 and never hit the
   network twice.

Exit codes: 0 = all links valid; 1 = broken links found; 2 = usage error.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import urllib.request
from pathlib import Path

REPO_LINK_RE = re.compile(
    r"https://github\.com/thoser666/Rivulet/blob/develop/([^\)#\s]+)(?:#([^\)\s]+))?"
)
MD_LINK_RE = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
HEADING_RE = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
USER_AGENT = "rivulet-wiki-link-audit"
REQUEST_TIMEOUT = 15.0


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


def url_reachable(url: str) -> tuple[bool, str]:
    """HEAD first, GET as fallback; returns (ok, reason)."""
    for method in ("HEAD", "GET"):
        try:
            request = urllib.request.Request(
                url, method=method, headers={"User-Agent": USER_AGENT}
            )
            with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT) as response:
                if response.status < 400:
                    return True, f"HTTP {response.status}"
                return False, f"HTTP {response.status}"
        except Exception as error:  # noqa: BLE001 - report any failure reason
            reason = f"{type(error).__name__}: {error}"
    return False, reason


def normalize_target(target: str) -> tuple[str, str]:
    """Split `Page.md#anchor` / `Page#anchor` into (page, anchor)."""
    page, _, anchor = target.partition("#")
    return page.removesuffix(".md"), anchor


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
    parser.add_argument(
        "--skip-external",
        action="store_true",
        help="do not hit the network for external URLs (offline runs)",
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

    pages = {page.stem: page for page in wiki.glob("*.md")}

    def in_code_fence(line_index: int, lines: list[str]) -> bool:
        """True when the line sits inside a ``` fence (templates/examples)."""
        fences = sum(
            1 for line in lines[:line_index] if line.lstrip().startswith("```")
        )
        return fences % 2 == 1

    broken: list[str] = []
    interwiki = 0
    repo_docs = 0
    external_checked: dict[str, str] = {}
    for page in sorted(wiki.glob("*.md")):
        text = page.read_text(encoding="utf-8")
        lines = text.splitlines()

        def line_of(match: re.Match) -> int:
            return text.count("\n", 0, match.start())

        # 1. Interwiki links: every relative md link must exist in the wiki.
        #    Links inside ``` fences are template examples, not real links.
        for match in MD_LINK_RE.finditer(text):
            if in_code_fence(line_of(match), lines):
                continue
            target = match.group(1)
            if target.startswith(("http://", "https://", "/", "mailto:")):
                continue
            interwiki += 1
            name, anchor = normalize_target(target)
            if name not in pages:
                broken.append(f"{page.name}: wiki page not found: {target}")
                continue
            if anchor:
                anchors = heading_anchors(pages[name].read_text(encoding="utf-8"))
                if anchor not in anchors:
                    broken.append(
                        f"{page.name}: anchor #{anchor} not found in wiki page {name}"
                    )

        # 2. Repo-doc links: file on develop + GitHub heading anchor.
        for match in REPO_LINK_RE.finditer(text):
            repo_docs += 1
            rel, anchor = match.group(1), match.group(2)
            markdown = load_repo_file(repo_root, rel, args.develop_from_origin)
            if markdown is None:
                broken.append(f"{page.name}: missing file on develop: {rel}")
                continue
            if anchor and anchor not in heading_anchors(markdown):
                broken.append(f"{page.name}: anchor #{anchor} not found in {rel}")

        # 3. External URLs: reachable over HTTP (HEAD, GET fallback).
        if not args.skip_external:
            for match in MD_LINK_RE.finditer(text):
                if in_code_fence(line_of(match), lines):
                    continue
                url = match.group(1)
                if not url.startswith(("http://", "https://")):
                    continue
                if "github.com/thoser666/Rivulet/blob" in url:
                    continue  # covered by rule 2
                if url in external_checked:
                    continue  # one network check per unique URL
                ok, reason = url_reachable(url)
                external_checked[url] = reason
                if not ok:
                    broken.append(f"{page.name}: unreachable external URL {url} ({reason})")

    total_external = len(external_checked)
    print(
        f"checked {interwiki} interwiki link(s), {repo_docs} repo-doc link(s), "
        f"{total_external} external URL(s) "
        f"across {len(list(wiki.glob('*.md')))} wiki page(s)"
    )
    if broken:
        print("wiki link audit FAILED:")
        for item in broken:
            print(f"- {item}")
        return 1
    print("wiki link audit passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
