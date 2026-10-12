#!/usr/bin/env python3
from pathlib import Path
import re, io, sys

path = Path("rivulet-core/src/lib.rs")
lines = path.read_text(encoding="utf-8").split("\n")

out = []

i = 0
n = len(lines)

def peek(k):
    if i + k < n:
        return lines[i + k]
    return ""

def take_span(end_pred):
    """Collect lines until end_pred(lines, i) is true, mutate i."""
    buf = []
    while not end_pred(lines, i):
        buf.append(lines[i])
        i += 1
        if i >= n:
            break
    return buf

def rewrite_split_format(buf):
    """
    buf is a sequence of lines forming:
        format!(
            "a "
            "b "
            args...
        )
    or
        push_str(&format!(
            "a "
            "b "
            args...
        )
    or
        s.push_str(
            "a "
            "b "
            ...
        )
    We merge adjacent string-literal lines (those that after stripping look like a
    double-quoted literal) that appear before the first non-string-literal argument,
    collapsing them into one literal per line-join.
    """
    # Determine call shape by first line
    first = buf[0].rstrip()
    is_push = first.endswith("push_str(") or first.endswith("push_str(&format!(")
    # For a simple `push_str(` call with multiline string args, format is similar.
    # Find the '(' position
    if "(" in first:
        head = first[: first.index("(") + 1]
    else:
        head = first

    # Collect tokens across the body (lines after head, before the closing ')')
    body_lines = buf[1:]
    # Find index of the line that contains the closing ')' for the call, assuming
    # the call closes on the last line of buf.
    # We'll just operate on buf as the full call block; last line should contain ')'.
    if not buf[-1].strip().endswith(")"):
        # not a simple block; leave untouched
        return buf

    # Parse: separate the "header" part (up to and including '(') and the trailing ')'
    last = buf[-1]
    if ")" in last:
        close_idx = last.index(")")
        close_head = last[: close_idx + 1]
        close_tail = last[close_idx + 1 :]
    else:
        close_head = last
        close_tail = ""

    # Body = everything after '(' on first line, plus lines[1:-1], plus text before ')' on last line
    first_body = first.split("(", 1)[1] if "(" in first else ""
    mid = body_lines[0:-1] if len(body_lines) > 1 else []
    last_body = last.split(")", 1)[0]

    full_body = first_body + "\n" + "\n".join(mid) + "\n" + last_body

    # Tokenize full_body preserving whitespace layout: we'll collapse runs of adjacent
    # string literals (whitespace-separated quotes) into one token.
    tokens = re.findall(r "[^\s]+", full_body)
    merged_tokens: list[str] = []
    k = 0
    while k < len(tokens):
        t = tokens[k]
        if t.startswith('"'):
            run = [t]
            k += 1
            while k < len(tokens) and tokens[k].startswith('"'):
                run.append(tokens[k])
                k += 1
            merged_tokens.append("".join(run))
        else:
            merged_tokens.append(t)
            k += 1

    # Rebuild body: put a space between merged tokens to mirror original layout.
    merged_body = " ".join(merged_tokens)
    # Now reconstruct call block lines:
    # first new line: head + first chunk of merged_body up to the first newline-equivalent spot.
    # We'll just emit: head + " " + merged_body + ")" and then pad with original blank lines
    # only if there were blank lines originally.
    had_blank_between = any(ln.strip() == "" for ln in buf[1:-1])

    new_first = head + " " + merged_body + close_head
    result = [new_first]
    if close_tail:
        result[-1] = result[-1] + close_tail
    return result


i = 0
changes = []
while i < n:
    line = lines[i].rstrip()
    if line.endswith("format!(") or line.endswith("push_str(&format!(") or line.endswith("push_str("):
        # try to collect a simple block
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
        end = j
        block = lines[start : end + 1]
        # check whether it contains split string literals in the body
        body = "\n".join(block[1:-1])
        strlit_lines = [ln.strip() for ln in block[1:-1] if ln.strip().startswith('"')]
        if len(strlit_lines) >= 2:
            rewritten = rewrite_split_format(block)
            if rewritten != block:
                changes.append((start + 1, end + 1, block, rewritten))
                out.extend(rewritten)
                i = end + 1
                continue
        out.extend(block)
        i = end + 1
    else:
        out.append(lines[i])
        i += 1

report_only = "--report" in sys.argv
if changes:
    print(f"found {len(changes)} split-format call blocks in lib.rs:")
    for s, e, before, after in changes:
        print(f"\n=== lib.rs:{s}-{e} ===")
        print("BEFORE:")
        for ln in before:
            print(ln)
        print("AFTER:")
        for ln in after:
            print(ln)
    if not report_only:
        path.write_text("\n".join(out), encoding="utf-8")
        print("\napplied changes to disk.")
    else:
        print("\nreport-only: not writing to disk.")
else:
    print("no split-format blocks to collapse in lib.rs")
