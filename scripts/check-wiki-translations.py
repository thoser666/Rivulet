#!/usr/bin/env python3
"""Validate bilingual GitHub Wiki page pairs."""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys

PAIR_RE = re.compile(r"^\[(?:Deutsch|German)[^\]]*\]\(([^)]+)\)")


def page_target(value: str) -> Path:
    value = value.split("#", 1)[0].split("?", 1)[0]
    if value.startswith("http") or value.startswith("/"):
        return Path()
    return Path(value.removesuffix(".md") + ".md")


def check(root: Path) -> list[str]:
    errors: list[str] = []
    english = sorted(p for p in root.glob("*.md") if not p.stem.endswith("-de"))
    for source in english:
        if source.name == "Languages.md":
            continue
        german = root / f"{source.stem}-de.md"
        if not german.exists():
            errors.append(f"missing German pair: {source.name} -> {german.name}")
            continue
        text = source.read_text(encoding="utf-8")
        german_text = german.read_text(encoding="utf-8")
        if not PAIR_RE.search(text):
            errors.append(f"missing German language link: {source.name}")
        if not re.search(r"\[(?:English|German)[^\]]*\]\(([^)]+)\)", german_text):
            errors.append(f"missing English language link: {german.name}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=".freebuff-rivulet-wiki")
    args = parser.parse_args()
    errors = check(Path(args.root))
    if errors:
        print("wiki translation check failed:")
        print("\n".join(f"- {error}" for error in errors))
        return 1
    print("wiki translation check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
