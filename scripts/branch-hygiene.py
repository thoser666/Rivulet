#!/usr/bin/env python3
"""Map local branches whose upstream is gone against the repository's PR history.

Squash-merging leaves no ancestry link, so `git branch -d` refuses to delete a
merged branch and nothing local records whether its PR landed. This script looks
every local branch with a `[gone]` upstream up in `gh pr list --state all` and
reports MERGED (safe to delete), CLOSED (finished without merge — compare the
content first), OPEN (remote branch deleted while the PR is open) or NO_PR
(never pushed as a PR — may hold unmerged work, so it never fails `--check`).

Run `git fetch --prune` first, otherwise git still believes in upstreams whose
remote branches are gone. Branches that were never pushed have no upstream and
are out of scope: that is local work, not a finished PR.

Exit codes: 0 = nothing actionable, 1 = `--check` found a merged or closed
branch, 2 = git or gh cannot answer (no repository, gh missing, not signed in).

Usage:
    scripts/branch-hygiene.py           # report every gone branch and its PR
    scripts/branch-hygiene.py --check   # exit 1 if a merged/closed one lingers
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
PR_LIMIT = 1000
FAILING = ("MERGED", "CLOSED")
ORDER = {"MERGED": 0, "CLOSED": 1, "OPEN": 2, "NO_PR": 3}

# Unicode output must survive non-UTF-8 consoles (cp1252 on Windows).
for _stream in (sys.stdout, sys.stderr):
    _reconfigure = getattr(_stream, "reconfigure", None)
    if _reconfigure is not None:
        try:
            _reconfigure(encoding="utf-8")
        except (ValueError, OSError):
            pass


def _run(cmd):
    return subprocess.run(
        cmd,
        cwd=REPO_ROOT,
        text=True,
        encoding="utf-8",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def gone_branches():
    """(name, short tip sha) for every local branch whose upstream is gone."""
    result = _run(
        [
            "git",
            "for-each-ref",
            "--format=%(refname:short)%09%(objectname:short)%09%(upstream:track)",
            "refs/heads",
        ]
    )
    if result.returncode != 0:
        raise SystemExit(f"git for-each-ref failed:\n{result.stderr.strip()}")

    branches = []
    for line in result.stdout.splitlines():
        fields = line.split("\t")
        if len(fields) > 2 and fields[2].strip() == "[gone]":
            branches.append((fields[0], fields[1]))
    return sorted(branches)


def load_prs(limit=PR_LIMIT):
    """The most recent PR per head branch, across every PR state."""
    try:
        result = _run(
            [
                "gh",
                "pr",
                "list",
                "--state",
                "all",
                "--limit",
                str(limit),
                "--json",
                "number,state,headRefName,title",
            ]
        )
    except FileNotFoundError:
        raise SystemExit(
            "the GitHub CLI (`gh`) is not installed; install it and run `gh auth login`"
        )
    if result.returncode != 0:
        raise SystemExit(f"`gh pr list` failed:\n{result.stderr.strip()}")

    prs = json.loads(result.stdout or "[]")
    if len(prs) >= limit:
        print(
            f"warning: `gh pr list` hit its {limit}-PR limit, so an older PR looks "
            "like NO_PR",
            file=sys.stderr,
        )

    by_branch = {}
    for pr in prs:
        branch = pr.get("headRefName") or ""
        known = by_branch.get(branch)
        # A reused branch name: the highest number is the latest PR.
        if known is None or pr.get("number", 0) > known.get("number", 0):
            by_branch[branch] = pr
    return by_branch


def classify(pr):
    """MERGED/CLOSED/OPEN as `gh` states them, NO_PR without a PR."""
    if pr is None:
        return "NO_PR"
    return (pr.get("state") or "").upper() or "NO_PR"


def main():
    parser = argparse.ArgumentParser(
        description="Map local branches with a removed upstream against the PR history."
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit 1 when a branch belongs to a merged or closed PR",
    )
    args = parser.parse_args()

    gone = gone_branches()
    if not gone:
        print(
            "No local branch has a removed upstream. "
            "(Run `git fetch --prune` if you expect some.)"
        )
        return 0

    prs = load_prs()
    rows = [(name, sha, classify(prs.get(name)), prs.get(name)) for name, sha in gone]
    rows.sort(key=lambda row: (ORDER.get(row[2], len(ORDER)), row[0]))

    width = max(len(row[0]) for row in rows)
    print(f"{'branch':<{width}}  {'tip':<8}  {'status':<6}  PR")
    for name, sha, status, pr in rows:
        if pr is None:
            detail = "no PR ever opened — local work, keep or push it"
        else:
            title = (pr.get("title") or "").strip()
            if len(title) > 62:
                title = title[:59] + "..."
            detail = f"#{pr['number']} {title}"
        print(f"{name:<{width}}  {sha:<8}  {status:<6}  {detail}")

    stale = [row for row in rows if row[2] in FAILING]
    print(
        f"\n{len(rows)} branch(es) with a removed upstream, "
        f"{len(stale)} of them merged or closed"
    )
    if not stale or not args.check:
        return 0
    for name, _, status, pr in stale:
        print(f"git branch -D {name}   # {status} #{pr['number']}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
