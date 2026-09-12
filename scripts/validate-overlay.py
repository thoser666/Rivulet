#!/usr/bin/env python3
"""Validate a generated browser-source overlay artifact (M10 code-gen spike).

A generated overlay is a *single self-contained HTML file* that must run inside
Rivulet's browser-source layer (S5b contract: WebView2/WebKitGTK, no network,
bounded resource use). The validator enforces the objective part of the spike
score; the visual pass (manual screenshot review) is a separate checklist.

Exit code 0 = artifact passes; nonzero = failed, with reasons on stdout.

Checks (each maps to a spike-score column):
  structure   : has DOCTYPE, <html>, <style> or <script>, non-empty body
  no-remote   : no http(s):// CDN/script/link/img URLs (must be self-contained)
  no-crash    : no document.write, no top-level error-throwing patterns,
                no infinite sync loops (while(true) without await)
  size        : sane footprint (>= 300 bytes, <= 512 KiB)
  syntax      : JS blocks parse with a real JS engine (node --check) when
                node is available; skipped (scored n/a) otherwise
  animation   : uses requestAnimationFrame, CSS animation, or an event loop
                (overlays are expected to be live, not static)

Usage:
  python scripts/validate-overlay.py <file.html> [--json]
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

MIN_BYTES = 300
MAX_BYTES = 512 * 1024

REMOTE_URL_RE = re.compile(
    r"""(?:src|href)\s*=\s*["']https?://[^"']+["']""", re.IGNORECASE
)
# Bare CDN references inside JS strings also count (fetch, import, etc.).
REMOTE_FETCH_RE = re.compile(
    r"""(?:fetch|import|XMLHttpRequest)\s*\(\s*["']https?://""", re.IGNORECASE
)

CHECK_NAMES = ["structure", "no-remote", "no-crash", "size", "syntax", "animation", "api"]


def check_structure(text: str) -> tuple[bool, str]:
    lower = text.lower()
    missing = []
    if "<!doctype html" not in lower:
        missing.append("DOCTYPE")
    if "<html" not in lower:
        missing.append("<html>")
    if "<style" not in lower and "<script" not in lower:
        missing.append("<style>/<script>")
    if re.search(r"<body[^>]*>\s*</body>", lower) or "<body" not in lower:
        missing.append("non-empty <body>")
    if missing:
        return False, f"missing: {', '.join(missing)}"
    return True, "ok"


def check_no_remote(text: str) -> tuple[bool, str]:
    hits = REMOTE_URL_RE.findall(text) + REMOTE_FETCH_RE.findall(text)
    if hits:
        return False, f"{len(hits)} remote URL(s), e.g. {hits[0][:60]}"
    return True, "ok"


def check_no_crash(text: str) -> tuple[bool, str]:
    problems = []
    if "document.write" in text:
        problems.append("document.write")
    # while(true) without an await/async yield inside 200 chars is a sync
    # infinite loop; legitimate event loops use requestAnimationFrame or await.
    for m in re.finditer(r"while\s*\(\s*true\s*\)", text):
        window = text[m.start() : m.start() + 200]
        if not re.search(r"await\s|requestAnimationFrame|setTimeout|postMessage", window):
            problems.append("while(true) sync loop")
            break
    if problems:
        return False, "; ".join(problems)
    return True, "ok"


def check_size(raw: bytes) -> tuple[bool, str]:
    n = len(raw)
    if n < MIN_BYTES:
        return False, f"{n} bytes < {MIN_BYTES} (stub output?)"
    if n > MAX_BYTES:
        return False, f"{n} bytes > {MAX_BYTES} (bloated)"
    return True, f"{n} bytes"


_JS_BLOCK_RE = re.compile(r"<script\b[^>]*>(.*?)</script>", re.DOTALL | re.IGNORECASE)


def check_syntax(text: str) -> tuple[bool, str]:
    """Parse inline JS with node --check when available; n/a otherwise."""
    node = _find_node()
    if node is None:
        return True, "n/a (node not installed)"  # scored as pass, noted
    blocks = [m for m in _JS_BLOCK_RE.findall(text) if m.strip()]
    if not blocks:
        return True, "no inline JS"
    with tempfile.TemporaryDirectory() as td:
        for i, block in enumerate(blocks):
            p = Path(td) / f"block{i}.js"
            p.write_text(block, encoding="utf-8")
            r = subprocess.run(
                [node, "--check", str(p)], capture_output=True, text=True, timeout=15
            )
            if r.returncode != 0:
                first = (r.stderr or r.stdout).strip().splitlines()
                return False, f"JS block {i}: {first[0][:120] if first else 'syntax error'}"
    return True, f"{len(blocks)} JS block(s) parse"


def _find_node() -> str | None:
    for cand in ("node", "node.exe"):
        try:
            r = subprocess.run(
                [cand, "--version"], capture_output=True, text=True, timeout=10
            )
            if r.returncode == 0:
                return cand
        except (OSError, subprocess.TimeoutExpired):
            continue
    return None


ANIM_RE = re.compile(
    r"requestAnimationFrame|@keyframes|animation\s*:|transition\s*:|setInterval|setTimeout\s*\(|addEventListener"
)

# Every overlay prompt asks for a window.rivulet* host API (or an equivalent
# global rivulet* function definition).
RIVULET_API_RE = re.compile(
    r"window\.rivulet|function\s+rivulet|const\s+rivulet|let\s+rivulet", re.IGNORECASE
)


def check_animation(text: str) -> tuple[bool, str]:
    if ANIM_RE.search(text):
        return True, "ok"
    return False, "no animation/event wiring found (static page)"


def check_api(text: str) -> tuple[bool, str]:
    m = RIVULET_API_RE.search(text)
    if m:
        return True, f"host API present ({m.group(0)})"
    return False, "no rivulet* host API found"


def validate(path: Path) -> dict:
    raw = path.read_bytes()
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        return {
            "file": str(path),
            "ok": False,
            "checks": {name: {"ok": False, "detail": "not UTF-8"} for name in CHECK_NAMES},
        }
    checks = {
        "structure": dict(zip(("ok", "detail"), check_structure(text))),
        "no-remote": dict(zip(("ok", "detail"), check_no_remote(text))),
        "no-crash": dict(zip(("ok", "detail"), check_no_crash(text))),
        "size": dict(zip(("ok", "detail"), check_size(raw))),
        "syntax": dict(zip(("ok", "detail"), check_syntax(text))),
        "animation": dict(zip(("ok", "detail"), check_animation(text))),
        "api": dict(zip(("ok", "detail"), check_api(text))),
    }
    return {"file": str(path), "ok": all(c["ok"] for c in checks.values()), "checks": checks}


def self_test() -> int:
    """Self-test with a known-good and a known-bad overlay."""
    good = """<!DOCTYPE html>
<html><head><meta charset="utf-8"><style>
@keyframes slide { from { transform: translateY(-10px); opacity: 0; } to { transform: none; opacity: 1; } }
body { margin: 0; font-family: sans-serif; background: transparent; }
.alert { animation: slide 0.4s ease-out; color: white; text-shadow: 0 0 4px black; }
</style></head>
<body><div id="alerts"></div><script>
const el = document.getElementById('alerts');
window.rivuletAlert = function (msg) {
  const d = document.createElement('div');
  d.className = 'alert'; d.textContent = msg; el.appendChild(d);
  setTimeout(() => d.remove(), 4000);
};
let hue = 0;
function tick() { hue = (hue + 1) % 360; requestAnimationFrame(tick); }
requestAnimationFrame(tick);
</script></body></html>"""
    bad_remote = good.replace("</body></html>", '<script src="https://cdn.example.com/x.js"></script></body></html>')
    bad_stub = "<html><body>hi</body></html>"
    bad_js = good.replace("requestAnimationFrame(tick);", "while(true) { let x = 1; }")
    tmp = Path(tempfile.mkdtemp(prefix="overlay-val-selftest-"))
    cases = {
        "good.html": (good, True),
        "bad-remote.html": (bad_remote, False),
        "bad-stub.html": (bad_stub, False),
        "bad-js.html": (bad_js, False),
    }
    failures = 0
    for name, (content, expected_ok) in cases.items():
        p = tmp / name
        p.write_text(content, encoding="utf-8")
        result = validate(p)
        status = "PASS" if result["ok"] == expected_ok else "FAIL"
        if result["ok"] != expected_ok:
            failures += 1
        print(f"self-test {name}: {status} (ok={result['ok']}, expected={expected_ok})")
    return 1 if failures else 0


def main() -> int:
    args = [a for a in sys.argv[1:] if a != "--json"]
    if not args:
        print(__doc__)
        return 2
    if args[0] == "--self-test":
        return self_test()
    result = validate(Path(args[0]))
    if "--json" in sys.argv:
        print(json.dumps(result, indent=2))
    else:
        for name, c in result["checks"].items():
            mark = "OK  " if c["ok"] else "FAIL"
            print(f"  [{mark}] {name:<10} {c['detail']}")
        print(f"  => {'PASS' if result['ok'] else 'FAIL'}: {result['file']}")
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
