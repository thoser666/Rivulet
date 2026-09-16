#!/usr/bin/env python3
"""Verify that the GUI colors meet WCAG AA in both color schemes.
<arg_value><b88a6f17>The palettes are parsed directly from ``rivulet-gui/src/theme.rs`` (the single
source of truth):

- every status color is checked against the egui panel/window fill of its
  scheme (dark #1b1b1b, light #f8f8f8), and
- the accent widget fills (``widgets.active``/``widgets.hovered`` button
  fills, audit finding ui-006) are checked against their button text — egui
  paints button text from the widget-state ``fg_stroke``, so that is the
  color that sits on the fill.

The script fails (non-zero exit) when any pairing drops below the 4.5:1 AA
threshold, so a future palette change that breaks readability is caught in
CI.

Usage:
    scripts/check-theme-contrast.py [--json] [--self-test]
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

# `linear_multiply` brightens in linear space (ecolor 0.36); this factor must
# mirror theme.rs.
HOVERED_FACTOR = 1.15


def _ecolor_linear_from_gamma(g: float) -> float:
    """ecolor lib.rs `linear_from_gamma` (gamma [0,1] -> linear [0,1])."""
    if g <= 0.04045:
        return g / 12.92
    return ((g + 0.055) / 1.055) ** 2.4


def _ecolor_gamma_u8_from_linear(l: float) -> int:
    """ecolor lib.rs `gamma_u8_from_linear_f32` (linear [0,1] -> gamma u8)."""
    if l <= 0.0:
        return 0
    if l <= 0.0031308:
        return round(3294.6 * l)
    if l <= 1.0:
        return round(269.025 * l ** (1.0 / 2.4) - 14.025)
    return 255


def linear_multiply(rgb: tuple, factor: float) -> tuple:
    """Mirror ecolor 0.36 `Color32::linear_multiply` for an opaque color.

    The conversion chain is premultiplied-alpha: gamma -> linear, multiply,
    then back through the premultiplied `From<Rgba> for Color32` conversion,
    which un-multiplies in linear space and re-multiplies in gamma space.
    """
    out = []
    for c in rgb:
        lin = _ecolor_linear_from_gamma(c / 255.0)
        v = round(_ecolor_gamma_u8_from_linear(lin / factor) * factor)
        out.append(max(0, min(255, v)))
    return tuple(out)


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


def parse_const_color(src: str, const_name: str) -> tuple:
    """Parse an opaque `const {name}: egui::Color32 = ...` (from_rgb/from_gray/named)."""
    m = re.search(rf"const {const_name}:\s*egui::Color32\s*=\s*([^;]+);", src)
    if not m:
        raise ValueError(f"missing const {const_name} in {THEME_RS}")
    expr = m.group(1).strip()
    rgb = re.search(r"from_rgb\((\d+),\s*(\d+),\s*(\d+)\)", expr)
    if rgb:
        return (int(rgb.group(1)), int(rgb.group(2)), int(rgb.group(3)))
    gray = re.search(r"from_gray\((\d+)\)", expr)
    if gray:
        g = int(gray.group(1))
        return (g, g, g)
    named = re.search(r"Color32::(\w+)\s*$", expr)
    if named and named.group(1) in NAMED:
        return NAMED[named.group(1)]
    raise ValueError(f"cannot parse const {const_name} expression {expr!r}")


def self_test() -> int:
    """Guard the ecolor mirror math and the WCAG helpers against drift."""
    # The mirror must reproduce ecolor 0.36.1 exactly (premultiplied chain).
    assert linear_multiply((0, 105, 92), 1.15) == (0, 113, 99), "hover mirror"
    assert linear_multiply((255, 255, 255), 1.15) == (255, 255, 255), "clamp"
    assert linear_multiply((0, 0, 0), 1.15) == (0, 0, 0), "black stays black"
    # WCAG sanity on known pairings.
    assert abs(contrast((255, 255, 255), (0, 0, 0)) - 21.0) < 0.01, "white on black"
    assert contrast((250, 250, 250), (0, 105, 92)) >= AA_THRESHOLD, "active fill"
    assert contrast((250, 250, 250), linear_multiply((0, 105, 92), 1.15)) >= AA_THRESHOLD, "hovered fill"
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--json", action="store_true", help="emit one JSON document on stdout")
    ap.add_argument("--self-test", action="store_true", help="verify the ecolor mirror math and exit")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

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

    # ── Widget-state accent fills (audit finding ui-006) ────────────────
    # egui paints button text from the widget-state `fg_stroke`, so that is
    # the color sitting on the active/hovered fills. The fill and text are
    # pinned consts in theme.rs; the hovered fill is derived at runtime via
    # `linear_multiply`, which the checker mirrors exactly.
    try:
        widget_fill = parse_const_color(src, "WIDGET_ACCENT_FILL")
        widget_text = parse_const_color(src, "WIDGET_TEXT")
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    hovered_fill = linear_multiply(widget_fill, HOVERED_FACTOR)
    for name, fill in (("active", widget_fill), ("hovered", hovered_fill)):
        ratio = contrast(widget_text, fill)
        results.append(
            {
                "scheme": "both",
                "field": f"widgets.{name}",
                "rgb": list(widget_text),
                "background": list(fill),
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
        for scheme in ("dark", "light", "both"):
            rows = [r for r in results if r["scheme"] == scheme]
            if not rows:
                continue
            print(f"== {scheme} scheme ==")
            for r in rows:
                status = "ok" if r["pass"] else "FAIL"
                rgb = "#{:02X}{:02X}{:02X}".format(*r["rgb"])
                bg = "#{:02X}{:02X}{:02X}".format(*r["background"])
                print(
                    f"  {r['field']:<12} {r['contrast']:5.2f}:1  {rgb:<8} on {bg:<8} {status}"
                )
        print(f"\nThreshold: WCAG AA ({AA_THRESHOLD}:1) in both schemes")

    if not ok:
        print(
            "FAIL: at least one color pairing is below the WCAG AA threshold",
            file=sys.stderr,
        )
        return 1
    if not args.json:
        print("OK: all status colors and widget states meet WCAG AA in both schemes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
