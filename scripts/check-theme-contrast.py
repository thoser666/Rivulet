#!/usr/bin/env python3
"""Verify that the GUI status colors meet WCAG AA in both color schemes.

The palettes are parsed directly from ``rivulet-gui/src/theme.rs`` (the single
source of truth) and every status color is checked against the egui
panel/window fill of its scheme (dark #1b1b1b, light #f8f8f8). The script
fails (non-zero exit) when any color drops below the 4.5:1 AA threshold, so a
future palette change that breaks readability is caught in CI.

Usage:
    scripts/check-theme-contrast.py [--json]
"""

import argparse
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
THEME_RS = REPO_ROOT / "rivulet-gui" / "src" / "theme.rs"

# RGB values of the egui/ecolor named constants used by the palettes
# (ecolor 0.36.1). Unknown constants abort the check instead of guessing.
NAMED = {
    "LIGHT_GREEN": (144, 238, 144),
    "YELLOW": (255, 255, 0),
    "LIGHT_RED": (255, 128, 128),
    "LIGHT_BLUE": (173, 216, 230),
    "GRAY": (160, 160, 160),
}

AA_THRESHOLD = 4.5


def linear(channel: float) -> float:
    c = channel / 255.0
    return c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4


def luminance(rgb) -> float:
    r, g, b = rgb
    return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)


def contrast(fg, bg) -> float:
    l1, l2 = luminance(fg), luminance(bg)
    hi, lo = (l1, l2) if l1 > l2 else (l2, l1)
    return (hi + 0.05) / (lo + 0.05)


def extract_method(src: str, name: str) -> str:
    """Return the body of `fn {name}(...) { ... }` from the Rust source."""
    start = src.index(f"pub fn {name}(")
    brace = src.index("{", start)
    depth = 0
    for i in range(brace, len(src)):
        if src[i] == "{":
            depth += 1
        elif src[i] == "}":
            depth -= 1
            if depth == 0:
                return src[brace + 1 : i]
    raise ValueError(f"unbalanced braces in fn {name} in {THEME_RS}")


def parse_palette(body: str) -> dict:
    """Extract `field: egui::Color32::NAME` / `from_rgb(r, g, b)` pairs."""
    field_re = re.compile(
        r"(\w+):\s*egui::Color32::(?:from_rgb\((\d+),\s*(\d+),\s*(\d+)\)|(\w+))"
    )
    palette = {}
    for line in body.splitlines():
        m = field_re.search(line)
        if not m:
            continue
        field, r, g, b, name = m.groups()
        if name:
            if name not in NAMED:
                raise ValueError(f"unknown egui color constant {name!r} in {THEME_RS}")
            palette[field] = NAMED[name]
        else:
            palette[field] = (int(r), int(g), int(b))
    return palette


def parse_background(src: str, const_name: str) -> tuple:
    """Parse `const {name}: ... = egui::Color32::from_gray(n);`."""
    m = re.search(rf"const {const_name}:.*?from_gray\((\d+)\)", src)
    if not m:
        raise ValueError(f"missing const {const_name} in {THEME_RS}")
    g = int(m.group(1))
    return (g, g, g)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--json", action="store_true", help="emit one JSON document on stdout")
    args = ap.parse_args()

    if not THEME_RS.is_file():
        print(f"error: {THEME_RS} not found", file=sys.stderr)
        return 2

    src = THEME_RS.read_text(encoding="utf-8")
    schemes = [
        (
            "dark",
            parse_palette(extract_method(src, "dark")),
            parse_background(src, "DARK_BG"),
        ),
        (
            "light",
            parse_palette(extract_method(src, "light")),
            parse_background(src, "LIGHT_BG"),
        ),
    ]

    fields = ("success", "warning", "error", "active", "info", "hint")
    results = []
    ok = True
    for scheme, palette, bg in schemes:
        for field in fields:
            if field not in palette:
                print(f"error: {scheme} palette missing field {field!r}", file=sys.stderr)
                return 2
            ratio = contrast(palette[field], bg)
            results.append(
                {
                    "scheme": scheme,
                    "field": field,
                    "rgb": list(palette[field]),
                    "background": list(bg),
                    "contrast": round(ratio, 2),
                    "pass": ratio >= AA_THRESHOLD,
                }
            )
            ok = ok and ratio >= AA_THRESHOLD

    if args.json:
        print(
            json.dumps(
                {
                    "threshold": AA_THRESHOLD,
                    "results": results,
                },
                indent=2,
            )
        )
    else:
        for scheme in ("dark", "light"):
            print(f"== {scheme} scheme ==")
            for r in (r for r in results if r["scheme"] == scheme):
                status = "ok" if r["pass"] else "FAIL"
                rgb = "#{:02X}{:02X}{:02X}".format(*r["rgb"])
                print(
                    f"  {r['field']:<8} {r['contrast']:5.2f}:1  {rgb:<8} {status}"
                )
        print(f"\nThreshold: WCAG AA ({AA_THRESHOLD}:1) in both schemes")

    if not ok:
        print(
            "FAIL: at least one status color is below the WCAG AA threshold",
            file=sys.stderr,
        )
        return 1
    if not args.json:
        print("OK: all status colors meet WCAG AA in both schemes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
