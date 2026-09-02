#!/usr/bin/env python3
"""Keep wiki language-switch links and page pairs synchronized."""
from __future__ import annotations

import argparse
from datetime import date
from pathlib import Path
import subprocess
import sys


def run(*args: str) -> None:
    subprocess.run(args, check=True)


def git_sha(root: Path, rev: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(root), "rev-parse", rev],
        capture_output=True,
        text=True,
    )
    return result.stdout.strip() if result.returncode == 0 else ""


def verify_remote(root: Path) -> None:
    """Fetch and confirm HEAD matches the upstream after a push.

    Raises SystemExit(1) with a clear message if the remote no longer points at
    our pushed commit (for example a concurrent push, a force-push, or a
    broken remote URL).
    """
    run("git", "-C", str(root), "fetch", "--quiet", "origin")
    local = git_sha(root, "HEAD")
    upstream = git_sha(root, "@{u}") or git_sha(root, "origin/master")
    if not upstream:
        print(
            "ERROR: could not resolve the wiki remote HEAD (no upstream "
            "tracking ref and no origin/master); cannot verify the push",
            file=sys.stderr,
        )
        raise SystemExit(1)
    if not local:
        print("ERROR: could not resolve local HEAD of the wiki clone", file=sys.stderr)
        raise SystemExit(1)
    if local != upstream:
        print(
            f"ERROR: wiki remote hash {upstream} does not match local HEAD "
            f"{local}; the pushed sync commit was not applied (concurrent or "
            f"force push?)",
            file=sys.stderr,
        )
        raise SystemExit(1)
    print(f"ok: remote verified at {local[:12]} matching local HEAD")


def sync(root: Path, check_only: bool) -> list[str]:
    findings: list[str] = []
    for source in sorted(root.glob("*.md")):
        if source.stem.endswith("-de") or source.name == "Languages.md":
            continue
        target = root / f"{source.stem}-de.md"
        if not target.exists():
            findings.append(f"translation required: {source.name} -> {target.name}")
            continue
        german = target.read_text(encoding="utf-8")
        if "[English](" not in german and "[German](" not in german:
            findings.append(f"missing English switch: {target.name}")
            if not check_only:
                target.write_text(
                    f"[English]({source.stem}) · [Deutsch]({target.stem})\n\n{german}",
                    encoding="utf-8",
                )
    return findings


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--publish", action="store_true")
    parser.add_argument(
        "--skip-verify",
        action="store_true",
        help="push without the post-push remote hash check (for a read-only context)",
    )
    parser.add_argument("root", nargs="?", default=".freebuff-rivulet-wiki")
    args = parser.parse_args()
    root = Path(args.root)
    findings = sync(root, check_only=args.check)
    if findings:
        print("wiki synchronization findings:")
        print("\n".join(f"- {item}" for item in findings))
    if args.check:
        return 1 if findings else 0
    if args.publish:
        run("git", "-C", str(root), "config", "user.name", "github-actions[bot]")
        run("git", "-C", str(root), "config", "user.email", "41898282+github-actions[bot]@users.noreply.github.com")
        run("git", "-C", str(root), "add", "*.md")
        result = subprocess.run(["git", "-C", str(root), "diff", "--cached", "--quiet"])
        if result.returncode:
            run("git", "-C", str(root), "commit", "-m", f"docs: synchronize wiki translations ({date.today().isoformat()})")
            run("git", "-C", str(root), "push")
        if not args.skip_verify:
            verify_remote(root)
    return 0


if __name__ == "__main__":
    sys.exit(main())