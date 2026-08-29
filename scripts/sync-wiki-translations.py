#!/usr/bin/env python3
"""Keep wiki language-switch links and page pairs synchronized.

The script intentionally does not machine-translate prose. It creates a reviewable
translation task when an English page has no German counterpart and can commit/push
only generated navigation metadata when requested.
"""
from __future__ import annotations

import argparse
from datetime import date
from pathlib import Path
import subprocess
import sys


def run(*args: str) -> None:
    subprocess.run(args, check=True)


def sync(root: Path, check_only: bool) -> list[str]:
    changes: list[str] = []
    for source in sorted(root.glob("*.md")):
        if source.stem.endswith("-de") or source.name == "Languages.md":
            continue
        target = root / f"{source.stem}-de.md"
        if not target.exists():
            changes.append(f"translation required: {source.name} -> {target.name}")
            continue
        german = target.read_text(encoding="utf-8")
        if "[English](" not in german and "[German](" not in german:
            changes.append(f"missing English switch: {target.name}")
            if not check_only:
                target.write_text(
                    f"[English]({source.stem}) · [Deutsch]({target.stem})\n\n" + german,
                    encoding="utf-8",
                )
    return changes


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--publish", action="store_true")
    parser.add_argument("root", nargs="?", default=".freebuff-rivulet-wiki")
    args = parser.parse_args()
    root = Path(args.root)
    changes = sync(root, check_only=args.check)
    if changes:
        print("wiki synchronization findings:")
        print("\n".join(f"- {item}" for item in changes))
    if args.check:
        return 1 if changes else 0
    if args.publish:
        run("git", "-C", str(root), "config", "user.name", "github-actions[bot]")
        run("git", "-C", str(root), "config", "user.email", "41898282+github-actions[bot]@users.noreply.github.com")
        run("git", "-C", str(root), "add", "*.md")
        result = subprocess.run(["git", "-C", str(root), "diff", "--cached", "--quiet"])
        if result.returncode:
            run("git", "-C", str(root), "commit", "-m", f"docs: synchronize wiki translations ({date.today().isoformat()})")
            run("git", "-C", str(root), "push")
    return 0


if __name__ == "__main__":
    sys.exit(main())
