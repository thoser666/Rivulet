#!/usr/bin/env python3
"""
Fix the recurring compile error class in rivulet-core/src/lib.rs where a
`format!(...)` / `push_str(&format!(...)` / `push_str(...)` call body has the
format string spread over several top-level string-literal arguments.

Valid Rust permits adjacent string literals (optionally separated by whitespace,
including blank lines) to concatenate into one literal, so the fix is to merge
adjacent string-literal *arguments* that appear before the first non-string-literal
argument inside the call body.

We operate at call granularity: find call sites whose opening line is
  - `format!(`
  - `push_str(&format!(`
  - `s.push_str(` / `out.push_str(` / `<expr>.push_str(`
and whose body, between the opening `(` and the matching `)`, contains several
adjacent string literals before non-string content. Merge them into one literal.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path("rivulet-core/src")
LIB = ROOT / "lib.rs"


def is_strlit(token: str) -> bool:
    t = token.strip()
    return bool(t) and t.startswith('"')


def tokenize(line: str) -> list[str]:
    return re.findall(r"[^\s]+", line)


def find_call_spans(src: str) -> list[tuple[int, int]]:
    """Return (start, end) 0-indexed line spans of call sites whose first line
    is a `format!(` / push_str call start."""
    lines = src.split("\n")
    spans: list[tuple[int, int]] = []
    call_start_re = re.compile(r"^\s*(push_str\(&format!\(|push_str\(|format!\()\s*$")
    i = 0
    n = len(lines)
    while i < n:
        if call_start_re.match(lines[i]):
            start = i
            depth = 0
            j = i
            while j < n:
                for ch in lines[j]:
                    if ch == "(":
                        depth += 1
                    elif ch == ")":
                        depth -= 1
                if depth == 0 and j > i:
                    break
                j += 1
            if depth == 0:
                spans.append((start, j))
                i = j + 1
            else:
                i += 1
        else:
            i += 1
    return spans


def merge_call_body(src: str, start: int, end: int) -> list[str] | None:
    """If the call body has adjacent string-literal arguments to merge, return
    the new lines for the span; otherwise return None."""
    lines = src.split("\n")
    block = lines[start : end + 1]
    first_line = block[0]
    last_line = block[-1]

    # extract text between outermost '(' and ')'
    if "(" not in first_line:
        return None
    head, _ = first_line.split("(", 1)
    opening = head + "("

    if ")" not in last_line:
        return None
    idx = last_line.index(")")
    closing = last_line[: idx + 1]
    trailing = last_line[idx + 1 :]

    # body text: after '(' on first line + middle lines + before ')' on last line
    first_body = first_line.split("(", 1)[1] or ""
    middle = "\n".join(block[1:-1])
    last_body = last_line.split(")", 1)[0] or ""
    body_text = first_body + "\n" + middle + "\n" + last_body

    toks = tokenize(body_text)
    # find first non-strlit
    first_non: int | None = None
    for k, t in enumerate(toks):
        if not is_strlit(t):
            first_non = k
            break
    if first_non is None or first_non <= 1:
        return None  # no split-format issue or trivial

    strlit_args = toks[:first_non]
    if len(strlit_args) < 2:
        return None

    # merge adjacent string literals into one per contiguous run
    merged: list[str] = []
    k = 0
    while k < len(strlit_args):
        run = [strlit_args[k]]
        k += 1
        while k < len(strlit_args) and is_strlit(strlit_args[k]):
            run.append(strlit_args[k])
            k += 1
        merged.append("".join(run))

    # keep the rest tokens unchanged
    rest = toks[first_non:]
    new_tokens = merged + rest
    new_body = " ".join(new_tokens)

    # reassemble lines preserving blank-line structure loosely:
    # - first new line: opening + first chunk of new_body
    # - subsequent new lines: chunks of new_body up to the closing ')'
    # We'll just put opening + new_body + closing on one line unless body_text
    # contained blank lines, in which case we preserve one blank line.
    had_blank = "\n\n" in body_text or any(ln.strip() == "" for ln in block[1:-1])

    if had_blank:
        new_lines = [opening + " " + new_body, "", closing + trailing]
    else:
        new_lines = [opening + " " + new_body + closing + trailing]

    return new_lines


def main() -> int:
    src = LIB.read_text(encoding="utf-8")
    spans = find_call_spans(src)
    changes: list[tuple[int, int, list[str], list[str]]] = []

    # process spans backward so line numbers stay stable for subsequent diffs
    lines = src.split("\n")
    cur_lines = list(lines)
    offset = 0
    for start, end in reversed(spans):
        real_start = start
        real_end = end
        merged = merge_call_body("\n".join(cur_lines), real_start, real_end)
        if merged is None:
            continue
        before = cur_lines[real_start : real_end + 1]
        cur_lines[real_start : real_end + 1] = merged
        changes.append((real_start + 1, real_end + 1, before, merged))

    if not changes:
        print("no split-format call bodies to fix in lib.rs")
        return 0

    report_only = "--report" in sys.argv
    if report_only:
        print(f"would fix {len(changes)} split-format call bodies in lib.rs (report-only):")
        for s, e, before, after in changes:
            print(f"\n=== lib.rs:{s}-{e} ===")
            print("BEFORE:")
            for ln in before:
                print(ln)
            print("AFTER:")
            for ln in after:
                print(ln)
    else:
        LIB.write_text("\n".join(cur_lines), encoding="utf-8")
        print(f"fixed {len(changes)} split-format call bodies in lib.rs:")
        for s, e, _, _ in changes:
            print(f"  lib.rs:{s}-{e}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
