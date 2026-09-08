#!/usr/bin/env python3
"""Verify docs/user-guide.md still describes the current application.

The user guide goes stale silently: a new sidebar view or a shipped feature
does not remind anyone to document it. This check derives the required
coverage from the GUI source of truth (rivulet-gui/src/app.rs):

1. Navigation surface — the `AppView` enum variants must all appear.
2. Feature topics — automatically derived from the i18n keys the GUI uses:
   every `.tr("...")`/`.tr_fmt("...")` key is grouped by its first-underscore
   prefix, and any prefix with at least MIN_KEYS_PER_TOPIC distinct keys
   becomes a required topic (unless it is generic UI vocabulary, see
   GENERIC_PREFIXES). A new GUI feature with its own i18n key namespace
   therefore extends this check automatically — no script edit needed.
3. REQUIRED_TOPICS is an explicit floor on top of the derived topics, for
   features whose keys do not follow the prefix convention.

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
TR_CALL_RE = re.compile(r"\.tr(?:_fmt)?\(\s*\"([a-z0-9_]+)\"")

# A GUI prefix must own at least this many distinct i18n keys before it is
# treated as a feature topic. Below the threshold it is plumbing vocabulary
# (a single label or error message), not something the guide must document.
MIN_KEYS_PER_TOPIC = 5

# Prefixes that are generic UI vocabulary rather than a feature topic. A new
# generic prefix can be added here when a derivation turns out to be noise;
# everything else is derived automatically.
GENERIC_PREFIXES = frozenset(
    {
        "select", "invalid", "no", "not", "unknown", "section", "back",
        "cancel", "save", "refresh", "stop", "start", "next", "done",
        "check", "running", "paused", "stopped", "seconds", "file", "quit",
        "muted", "powered", "presence", "output", "source", "screen",
        "camera", "capture", "split", "system", "theme", "help", "nav",
        "multi", "auto", "updates", "scene", "rate",
    }
)

# Human-readable guide labels for prefixes whose capitalized form would be
# awkward or ambiguous. Everything else derives as prefix.capitalize().
LABEL_OVERRIDES = {
    "obs": "obs-websocket",
    "autoclip": "Auto-Clip",
    "ndi": "NDI",
    "vf": "Video",
}

# Acceptable alternative spellings for a topic (like VIEW_ALIASES). Used when
# the guide documents the feature under its localized or colloquial name.
TOPIC_ALIASES = {
    "Composition": ("Quellen", "Sources"),
}

# Feature topics that must appear in the guide once shipped, independent of
# the i18n-key derivation (floor for features whose keys do not follow the
# prefix convention). Word-level matching (case-insensitive) so "Auto-Clip"
# matches "Auto-Clips" too.
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


def derive_required_topics(app_rs: str) -> list[str]:
    """Derive guide topics from the i18n keys the GUI actually uses.

    Every static `.tr`/`.tr_fmt` key is grouped by its first-underscore
    prefix. Prefixes owning at least MIN_KEYS_PER_TOPIC distinct keys are
    feature topics; GENERIC_PREFIXES filters plumbing vocabulary and
    LABEL_OVERRIDES fixes awkward labels.
    """
    keys = set(TR_CALL_RE.findall(app_rs))
    counts: dict[str, int] = {}
    for key in keys:
        prefix = key.split("_", 1)[0]
        counts[prefix] = counts.get(prefix, 0) + 1
    topics: list[str] = []
    for prefix in sorted(counts):
        if counts[prefix] < MIN_KEYS_PER_TOPIC or prefix in GENERIC_PREFIXES:
            continue
        topics.append(LABEL_OVERRIDES.get(prefix, prefix.capitalize()))
    return topics


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

    derived = derive_required_topics(app_rs)
    topics = list(dict.fromkeys([*REQUIRED_TOPICS, *derived]))
    derived_set = set(derived)
    for topic in topics:
        candidates = [topic, *TOPIC_ALIASES.get(topic, ())]
        if not any(
            re.search(re.escape(c).replace(r"\-", r"[\-\s]?"), guide, re.IGNORECASE)
            for c in candidates
        ):
            origin = (
                "derived from GUI i18n keys"
                if topic in derived_set
                else "required topics floor"
            )
            errors.append(
                f"user guide does not cover feature topic '{topic}' ({origin})"
            )

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
            # A shipped feature with its own key namespace: 6 distinct keys.
            "    self.tr(\"widget_section\");\n"
            "    self.tr(\"widget_hint\");\n"
            "    self.tr(\"widget_enabled\");\n"
            "    self.tr_fmt(\"widget_count\", &[n]);\n"
            "    self.tr(\"widget_reset\");\n"
            "    self.tr(\"widget_theme\");\n"
            # Generic plumbing vocabulary must not derive a topic.
            "    self.tr(\"select_monitor\");\n"
            "    self.tr(\"select_window\");\n"
            "    self.tr(\"select_device\");\n"
            "    self.tr(\"select_scene\");\n"
            "    self.tr(\"select_preset\");\n"
            "    self.tr(\"select_output\");\n"
        )
        (root / "app.rs").write_text(app_rs, encoding="utf-8")
        views = parse_app_views(app_rs)
        assert views == [
            "Record", "Mixer", "Scenes", "Stream", "Assistant", "Settings", "Help"
        ], views

        # The new feature prefix derives automatically; generics do not.
        topics = derive_required_topics(app_rs)
        assert "Widget" in topics, topics
        assert "Select" not in topics, topics
        assert "Self" not in topics, topics

        # The topic alias lets the guide use the localized feature name.
        topics = derive_required_topics(app_rs)
        assert "Widget" in topics, topics
        assert "Composition" not in topics, topics  # below threshold here
        assert TOPIC_ALIASES["Composition"] == ("Quellen", "Sources")

        good = root / "good.md"
        good.write_text(
            "# Guide\n\n## Navigation\nRecord, Mixer, Scenes, Stream, Assistant, "
            "Settings, Hilfe\n\n## Features\nMultistream (Restream), Auto-Clips, "
            "MIDI-Controller, Discord-Status, obs-websocket, Sprache, "
            "Replay Buffer, Hotkeys, Widget-Panel\n",
            encoding="utf-8",
        )
        assert check(good, root / "app.rs") == [], check(good, root / "app.rs")

        # A guide missing the auto-derived feature topic fails — this is the
        # whole point: a new GUI feature extends the check with no script edit.
        stale = root / "stale.md"
        stale.write_text(
            "# Guide\n\n## Navigation\nRecord, Mixer, Scenes, Stream, Assistant, "
            "Settings, Hilfe\n\n## Features\nMultistream, Auto-Clips, MIDI, "
            "Discord, obs-websocket, Sprache, Replay, Hotkeys\n",
            encoding="utf-8",
        )
        errors = check(stale, root / "app.rs")
        assert any("Widget" in e and "derived" in e for e in errors), errors
        # The stale guide mentions "Quellen" but not "Composition" — the alias
        # must NOT silently satisfy a missing topic in the failing case either
        # way: here the guide omits both the topic and its aliases.
        assert all("Composition" not in e for e in errors), errors

        # Broken AppView source is reported as an error, not a crash.
        (root / "broken.rs").write_text("no enum here", encoding="utf-8")
        errors = check(good, root / "broken.rs")
        assert len(errors) == 1 and "cannot derive" in errors[0], errors

        # The --report-topics split: derived topics carry [derived], floor-only
        # topics carry [floor], and the summary line adds up correctly.
        import io
        from contextlib import redirect_stdout

        buf = io.StringIO()
        with redirect_stdout(buf):
            code = report_topics(root / "app.rs")
        assert code == 0, code
        out = buf.getvalue()
        assert "- Widget [derived]" in out, out
        for floor_topic in REQUIRED_TOPICS:
            assert f"- {floor_topic} [floor]" in out, out
        summary = out.strip().splitlines()[-1]
        assert summary.startswith("total: "), out
        derived_n = len([l for l in out.splitlines() if l.endswith("[derived]")])
        floor_n = len([l for l in out.splitlines() if l.endswith("[floor]")])
        assert (derived_n, floor_n) == (1, 8), (derived_n, floor_n)
        assert (
            summary
            == f"total: {derived_n} derived + {floor_n} floor = "
            f"{derived_n + floor_n} required topics"
        ), summary

        # A broken app source exits 2 with an error, not a crash.
        import contextlib

        err = io.StringIO()
        with redirect_stdout(err), contextlib.redirect_stderr(io.StringIO()):
            code = report_topics(root / "broken.rs")
        assert code == 2, code

    print("self-test OK")
    return 0


def report_topics(app_rs_path: Path) -> int:
    """Print the derived vs. floor topic split for the wiki job's report.

    Never fails on coverage (that is the check mode's job); exits 2 only on
    environment errors. The split makes the weekly wiki report show which
    topics were auto-derived from GUI i18n keys and which come from the
    REQUIRED_TOPICS floor, so a maintainer can see the derivation working
    (and spot GENERIC_PREFIXES/LABEL_OVERRIDES entries that need review).
    """
    if not app_rs_path.is_file():
        print(f"error: app source not found: {app_rs_path}", file=sys.stderr)
        return 2
    app_rs = app_rs_path.read_text(encoding="utf-8")
    try:
        # Validate the AppView enum first so a broken GUI source is reported
        # (exit 2) instead of silently reporting an empty topic list.
        parse_app_views(app_rs)
        derived = derive_required_topics(app_rs)
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    floor = [t for t in REQUIRED_TOPICS if t not in derived]
    print("user-guide topics: derived (from GUI i18n keys)")
    for topic in derived:
        print(f"- {topic} [derived]")
    print("user-guide topics: floor (REQUIRED_TOPICS only)")
    for topic in floor:
        print(f"- {topic} [floor]")
    print(
        f"total: {len(derived)} derived + {len(floor)} floor = "
        f"{len(derived) + len(floor)} required topics"
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--guide", default="docs/user-guide.md")
    parser.add_argument("--app-rs", default="rivulet-gui/src/app.rs")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--report-topics",
        action="store_true",
        help="print the derived vs. floor topic split and exit 0",
    )
    args = parser.parse_args()

    if args.self_test:
        return self_test()
    if args.report_topics:
        return report_topics(Path(args.app_rs))

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
            "add a LABEL_OVERRIDES/GENERIC_PREFIXES entry in "
            "scripts/check-user-guide-freshness.py if the derivation is wrong, "
            "or extend REQUIRED_TOPICS deliberately."
        )
        return 1
    print("user guide freshness check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
