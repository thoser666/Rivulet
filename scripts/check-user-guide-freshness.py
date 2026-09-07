#!/usr/bin/env python3
"""Verify docs/user-guide.md still describes the current application.

The user guide goes stale silently: a new sidebar view or a shipped feature
does not remind anyone to document it. This check derives the navigation
surface from the GUI source of truth (the `AppView` enum in
rivulet-gui/src/app.rs) and asserts the guide mentions every view, plus a
curated list of feature topics that must stay documented once shipped.

Exit codes: 0 = guide is current; 1 = guide is missing required coverage;
2 = usage/environment error.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys

APP_VIEW_RE = re.compile(r"enum AppView\s*\{(.*?)\}", re.DOTALL)
ENUM_VARIANT_RE = re.compile(r"^\s*([A-Z][A-Za-z0-9]*)\s*,")

# Feature topics that must appear in the guide once shipped. Word-level
# matching (case-insensitive) so "Auto-Clip" matches "Auto-Clips" too.
REQUIRED_TOPICS = [
    "Multistream",
    "Auto-Clip",
    "MIDI",
    "Discord",
    "obs-websocket",
    "Sprache",
    "Replay",
    "Hotkey",
]

# The guide is written in German, so an English enum variant may legitimately
# appear under its localized label. Each variant lists acceptable alternative
# spellings; the variant name itself is always accepted.
VIEW_ALIASES = {
    "Help": ("Hilfe",),
}


def parse_app_views(app_rs: str) -> list[str]:
    """Extract AppView enum variants from the GUI source."""
    match = APP_VIEW_RE.search(app_rs)
    if not match:
        raise ValueError("AppView enum not found in rivulet-gui/src/app.rs")
    body = match.group(1)
    views: list[str] = []
    for line in body.splitlines():
        # Strip attributes like #[default] and comments.
        line = re.sub(r"#\[.*?\]", "", line)
        line = line.split("//", 1)[0].strip()
        m = ENUM_VARIANT_RE.match(line)
        if m and m.group(1) not in views:
            views.append(m.group(1))
    return views


def check(guide_path: Path, app_rs_path: Path) -> list[str]:
    errors: list[str] = []
    guide = guide_path.read_text(encoding="utf-8")
    app_rs = app_rs_path.read_text(encoding="utf-8")

    try:
        views = parse_app_views(app_rs)
    except ValueError as exc:
        return [f"cannot derive navigation surface: {exc}"]

    for view in views:
        candidates = [view, *VIEW_ALIASES.get(view, ())]
        if not any(re.search(rf"\b{re.escape(c)}\b", guide) for c in candidates):
            errors.append(
                f"user guide does not mention navigation view '{view}' "
                f"(derived from AppView in rivulet-gui/src/app.rs)"
            )

    for topic in REQUIRED_TOPICS:
        pattern = re.escape(topic).replace(r"\-", r"[\-\s]?")
        if not re.search(pattern, guide, re.IGNORECASE):
            errors.append(f"user guide does not cover feature topic '{topic}'")

    return errors


def self_test() -> int:
    """Offline logic test with canned sources (no repo files needed)."""
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        app_rs = (
            "enum AppView {\n"
            "    #[default]\n"
            "    Record,\n"
            "    Mixer,\n"
            "    // a comment line\n"
            "    Scenes,\n"
            "    Stream,\n"
            "    Assistant,\n"
            "    Settings,\n"
            "    Help,\n"
            "}\n"
        )
        (root / "app.rs").write_text(app_rs, encoding="utf-8")
        views = parse_app_views(app_rs)
        assert views == [
            "Record", "Mixer", "Scenes", "Stream", "Assistant", "Settings", "Help"
        ], views

        good = root / "good.md"
        good.write_text(
            "# Guide\n\n## Navigation\nRecord, Mixer, Scenes, Stream, Assistant, "
            "Settings, Hilfe\n\n## Features\nMultistream (Restream), Auto-Clips, "
            "MIDI-Controller, Discord-Status, obs-websocket, Sprache, "
            "Replay Buffer, Hotkeys\n",
            encoding="utf-8",
        )
        assert check(good, root / "app.rs") == [], check(good, root / "app.rs")

        stale = root / "stale.md"
        stale.write_text("# Guide\n\nRecord und Mixer only.\n", encoding="utf-8")
        errors = check(stale, root / "app.rs")
        assert any("Scenes" in e for e in errors), errors
        assert any("Multistream" in e for e in errors), errors
        assert len(errors) > 5, errors

        # Broken AppView source is reported as an error, not a crash.
        (root / "broken.rs").write_text("no enum here", encoding="utf-8")
        errors = check(good, root / "broken.rs")
        assert len(errors) == 1 and "cannot derive" in errors[0], errors

    print("self-test OK")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--guide", default="docs/user-guide.md")
    parser.add_argument("--app-rs", default="rivulet-gui/src/app.rs")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    guide = Path(args.guide)
    app_rs = Path(args.app_rs)
    if not guide.is_file():
        print(f"error: guide not found: {guide}", file=sys.stderr)
        return 2
    if not app_rs.is_file():
        print(f"error: app source not found: {app_rs}", file=sys.stderr)
        return 2

    errors = check(guide, app_rs)
    if errors:
        print("user guide is OUT OF DATE:")
        print("\n".join(f"- {e}" for e in errors))
        print(
            "\nUpdate docs/user-guide.md to cover the missing views/topics, "
            "or extend REQUIRED_TOPICS in scripts/check-user-guide-freshness.py "
            "deliberately."
        )
        return 1
    print("user guide freshness check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
