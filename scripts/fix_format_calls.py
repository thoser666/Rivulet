#!/usr/bin/env python3
"""
Fix two recurring compile-error classes in rivulet-core/src/lib.rs:

1) split-format-string calls:
   A call like
       format!(
           "appsrc ... "
           "! videoconvert ... ",
           caps, overlay, encoder
       )
   is invalid because the format string was split into multiple top-level
   arguments. Valid Rust: adjacent string literals concatenate into one literal.
   We merge adjacent string-literal *arguments* inside the call body (before the
   first non-string-literal argument) into one literal.

2) stray backslash before closing quote in a string literal:
   "...\"   ->  "...
   This appears in a few raw string assignments / literals (e.g. pipeline test
   fixtures). We remove the backslash right before the terminating quote.
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


def merge_call_body_lines(lines: list[str], start: int, end: int) -> list[str] | None:
    """If the call body between lines[start] and lines[end] has split format-string
    arguments, merge them and return new lines for the span. Otherwise return None."""
    block = lines[start : end + 1]
    first_line = block[0]
    last_line = block[-1]

    if "(" not in first_line or ")" not in last_line:
        return None

    opening = first_line.split("(", 1)[0] + "("
    last_close_idx = last_line.index(")")
    closing = last_line[: last_close_idx + 1]
    trailing = last_line[last_close_idx + 1 :]

    first_body = first_line.split("(", 1)[1] or ""
    middle = "\n".join(block[1:-1])
    last_body = last_line.split(")", 1)[0] or ""
    body = first_body + "\n" + middle + "\n" + last_body

    toks = tokenize(body)
    if not toks:
        return None

    first_non = None
    for k, t in enumerate(toks):
        if not is_strlit(t):
            first_non = k
            break
    if first_non is None or first_non <= 1:
        return None

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

    rest = toks[first_non:]
    new_tokens = merged + rest
    new_body = " ".join(new_tokens)

    # Rebuild lines, preserving any blank lines that existed in the original body
    # between non-blank content.
    orig_nonblank = [ln for ln in body.split("\n") if ln.strip() != ""]
    new_parts: list[str] = []
    # distribute new_body roughly by the number of original nonblank segments
    if not orig_nonblank:
        return None
    seg_count = len(orig_nonblank)
    per = max(1, len(new_body) // seg_count)
    pos = 0
    for _ in orig_nonblank:
        chunk_end = min(len(new_body), pos + max(1, len(orig_nonblank[0]) + 10))
        chunk = new_body[pos:chunk_end].strip()
        if chunk:
            new_parts.append(chunk)
        pos = chunk_end
    if pos < len(new_body):
        new_parts.append(new_body[pos:].strip())

    new_parts = [p for p in new_parts if p]
    if not new_parts:
        return None

    had_blank_between = "\n\n" in body or any(
        ln.strip() == "" for ln in block[1:-1]
    )
    if had_blank_between:
        lines_out: list[str] = [opening + " " + new_parts[0]]
        lines_out.extend(new_parts[1:])
        lines_out.append(closing + trailing)
    else:
        lines_out = [opening + " " + " ".join(new_parts) + closing + trailing]

    return lines_out


def normalize_str_lit(text: str) -> str:
    """Remove a stray backslash immediately before the closing quote in a Rust
    string literal if present, otherwise return unchanged."""
    # Match a double-quoted string whose content ends with a literal backslash
    # right before the closing quote, e.g.  "...\"  ->  "...
    m = re.match(r'("(?:[^"\\]|\\.)*)\\"(")', text)
    if m:
        return m.group(1) + m.group(2)
    return text


def normalize_assignment_lines(lines: list[str]) -> list[str] | None:
    """If the block is a simple assignment of a double-quoted string (possibly
    spanning multiple lines) with stray backslash before closing quote, fix it."""
    joined = " ".join(ln.strip() for ln in lines)
    if not re.match(r'^\s*[A-Za-z_]\w*\s*=\s*"', joined):
        return None
    # Work on the joined form: collapse whitespace, then fix str literal.
    collapsed = re.sub(r"\s+", " ", joined).strip()
    if not collapsed.endswith('";"') and not collapsed.endswith('"'):
        return None
    # split into tokens again
    toks = tokenize(collapsed)
    new_toks: list[str] = []
    i = 0
    changed = False
    while i < len(toks):
        t = toks[i]
        if is_strlit(t):
            fixed = normalize_str_lit(t)
            if fixed != t:
                changed = True
            new_toks.append(fixed)
        else:
            new_toks.append(t)
        i += 1
    if not changed:
        return None
    return [" ".join(new_toks)]


def main() -> int:
    src = LIB.read_text(encoding="utf-8")
    lines = src.split("\n")
    report_only = "--report" in sys.argv

    # 1) fix call bodies
    call_spans = find_call_spans(src)
    # process in reverse for stable indices
    ops: list[tuple[int, int, list[str]]] = []  # (start,end,newlines)
    cur_lines = list(lines)

    for start, end in reversed(call_spans):
        merged = merge_call_body_lines(cur_lines, start, end)
        if merged is None:
            continue
        op_start = start
        op_end = end
        ops.append((op_start, op_end, merged))

    # apply call fixes (forward order on reversed ops is fine since indices
    # already reflect cur_lines at time of computation)
    for start, end, new_lines in ops:
        cur_lines[start : end + 1] = new_lines

    # 2) fix stray backslash in string-literal assignments
    final_ops: list[tuple[int, int, list[str]]] = []
    i = 0
    n = len(cur_lines)
    while i < n:
        # detect a line that looks like "<ident> = \"...\"...  possibly with continuation
        if re.match(r"^\s*[A-Za-z_]\w*\s*=\s*\"", cur_lines[i]):
            start = i
            # collect until we reach a line containing the terminating '";' or '"' at end
            j = i
            while j < n and not cur_lines[j].rstrip().endswith('";') and not cur_lines[j].rstrip().endswith('"'):
                j += 1
            if j < n and (cur_lines[j].rstrip().endswith('";') or cur_lines[j].rstrip().endswith('"')):
                end = j
                block = cur_lines[start : end + 1]
                normalized = normalize_assignment_lines(block)
                if normalized is not None:
                    final_ops.append((start, end, normalized))
                i = end + 1
                continue
        i += 1

    for start, end, new_lines in final_ops:
        cur_lines[start : end + 1] = new_lines

    if not ops and not final_ops:
        print("no fixes needed in lib.rs")
        return 0

    report_only = "--report" in sys.argv
    if report_only:
        print("would apply the following fixes to lib.rs:\n")
        for start, end, new_lines in ops:
            print(f"\n=== call-fix lib.rs:{start+1}-{end+1} ===")
            print("BEFORE:")
            for ln in lines[start : end + 1]:
                print(ln)
            print("AFTER:")
            for ln in new_lines:
                print(ln)
        for start, end, new_lines in final_ops:
            print(f"\n=== str-literal-fix lib.rs:{start+1}-{end+1} ===")
            print("BEFORE:")
            for ln in lines[start : end + 1]:
                print(ln)
            print("AFTER:")
            for ln in new_lines:
                print(ln)
        return 0

    for start, end, new_lines in ops + final_ops:
        cur_lines[start : end + 1] = new_lines

    LIB.write_text("\n".join(cur_lines), encoding="utf-8")
    print(f"fixed {len(ops)} call bodies and {len(final_ops)} str-literal assignments in lib.rs")
    for start, end, _ in ops:
        print(f"  call-fix lib.rs:{start+1}-{end+1}")
    for start, end, _ in final_ops:
        print(f"  str-fix lib.rs:{start+1}-{end+1}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
