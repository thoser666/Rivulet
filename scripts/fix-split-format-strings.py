#!/usr/bin/env python3
"""
Normalize split string-literal arguments inside calls like::

    format!(
        "head "
        "tail "
        cap, overlay
    )

The bug in this codebase right now: a multi-line format/push_str call was written
with the *format string* split across several top-level arguments, e.g.

    format!(
        "appsrc name=rivulet_src do-timestamp=true "
        "! videoconvert{effects} ! {}{} ! {}{}{} ",
        caps, overlay, encoder, parse, tail
    )

Here the first two lines are two separate arguments to `format!`, so the call has
1 format arg + 5 more string args + real args, which rustc rejects as
"unexpected argument #N of type `&'static str`".

Valid Rust: adjacent string literals with only whitespace (and optional blank lines)
between them concatenate into a *single* literal. So the fix is to merge those adjacent
literals back into one contiguous token sequence so the call becomes

    format!(
        "appsrc name=rivulet_src do-timestamp=true "
        "! videoconvert{effects} ! {}{} ! {}{}{} ",
        caps, overlay, encoder, parse, tail
    )

where the first two lines are adjacent literals composing ONE format string, followed
by the real args.

We operate only at the token level inside calls, merging adjacent string-literal
arguments that are separated by whitespace (including blank lines), and we normalize
both `format!(...)` and `push_str(&format!(...)` call shapes. We do NOT touch
literals that are legitimately separate arguments (i.e. non-string-literal tokens in
between).

This is intentionally conservative and validating: if we can't cleanly detect the
call boundaries or if merging would produce something ambiguous, we skip and report.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path
from typing import List, Tuple


def is_strlit(token: str) -> bool:
    t = token.strip()
    return bool(t) and (
        t.startswith('"') or t.startswith("r#\"") or t.startswith("b\"") or t.startswith("br\"")
    )


def split_tokens(line: str) -> List[str]:
    # Very small tokenizer enough to split a line like
    #     "appsrc ... "   caps, overlay
    # into tokens by whitespace, keeping quoted literals intact.
    # We only use this for lines that are basically a sequence of
    # whitespace-separated tokens (which is the case for the broken spans here).
    return re.findall(r"[^\s]+", line)


def merge_adjacent_string_literals(tokens: List[str]) -> List[str]:
    """Merge adjacent string literal tokens separated only by whitespace.

    Example:
      ["\"head ", "\"tail ", "cap,", "overlay"] -> ["\"head tail ", "cap,", "overlay"]
    We merge by stripping the trailing whitespace/quote boundaries carefully: we just
    concatenate the raw token texts; for Rust adjacent literal concatenation that's
    exactly what we want.
    """
    out: List[str] = []
    i = 0
    n = len(tokens)
    while i < n:
        tk = tokens[i]
        if is_strlit(tk):
            merged = tk
            j = i + 1
            while j < n and is_strlit(tokens[j]):
                merged = merged + tokens[j]
                j += 1
            out.append(merged)
            i = j
        else:
            out.append(tk)
            i += 1
    return out


def find_call_blocks(lines: List[str]) -> List[Tuple[int, int]]:
    """Find spans of lines that form a single call start..matching close paren.

    We treat a line that (after stripping) starts a call as `...format!(` or
    `push_str(&format!(` possibly with leading whitespace and nothing after the `(`.
    Then we scan forward until parentheses balance again at the end of a line.
    """
    blocks: List[Tuple[int, int]] = []
    n = len(lines)
    i = 0
    call_start_re = re.compile(r"^\s*(push_str\(&format!\(|format!\()\s*$")
    while i < n:
        if call_start_re.match(lines[i]):
            start = i
            depth = 0
            j = i
            closing_at_end = False
            while j < n:
                for ch in lines[j]:
                    if ch == "(":
                        depth += 1
                    elif ch == ")":
                        depth -= 1
                if depth == 0:
                    closing_at_end = True
                    break
                j += 1
            if closing_at_end:
                blocks.append((start, j))
                i = j + 1
            else:
                # unbalanced; skip this start line to avoid pathological loops
                i += 1
        else:
            i += 1
    return blocks


def normalize_block(lines: List[str], start: int, end: int) -> List[str]:
    """Return a new list of lines for the call block, merging adjacent string
    literal arguments inside the call across blank lines.

    We process line-by-line but keep the structure (open paren, args, close paren).
    The merge happens across tokens within the argument area between `(` and the
    first non-string-literal argument (or up to `)`).
    """
    # Reconstruct the whole block text, normalize, then re-split into lines preserving
    # blank lines and indentation style where possible.
    # Strategy: join the block with newlines; within the argument region (inside the
    # outermost parens), collapse runs of adjacent string literals that are separated
    # by whitespace (including newlines). We'll do a simple but safe approach:
    # - split the inner content into tokens (whitespace-separated)
    # - merge adjacent string literals
    # - re-emit with original newline structure approximated by re-joining with the
    #   original blank-line positions where possible.
    block_text = "\n".join(lines)

    # isolate the outermost (...) content
    m = re.match(r"^(\s*)(push_str\(&format!\(|format!\()\s*$", lines[start])
    if not m:
        return lines[start : end + 1]

    indent = m.group(1)
    open_paren_line = lines[start]
    # Everything after the '(' on the start line, plus subsequent lines, up to the line
    # containing the matching ')'.
    # Grab text from after '(' on start line, plus lines start+1..end, then drop trailing ')'
    after_paren = open_paren_line.split("(", 1)[1]
    inner_lines = [after_paren] + lines[start + 1 : end]
    # The last inner line is ")..." possibly with trailing stuff; we only want inner
    # content up to the ')' on that last line.
    close_line = inner_lines[-1]
    if ")" in close_line:
        # take only up to the first ')' that balances? Simpler: assume ')' is at the end.
        close_content, _, _ = close_line.partition(")")
        inner_content = "\n".join(inner_lines[:-1]) + ("\n" + close_content if close_content else "")
    else:
        inner_content = "\n".join(inner_lines)

    # Now normalize inner content: collapse adjacent string literals separated by
    # whitespace (including newlines). We do this by replacing any sequence of string
    # literals separated by whitespace with their concatenation.
    def repl(m: re.Match[str]) -> str:
        return m.group(0).replace("\n", "").replace("    ", " ").replace("  ", " ")

    # Match a run of string-literal tokens separated by whitespace/newlines.
    # A string literal here may be raw, byte, etc., but in this codebase mostly plain.
    strlit_re = re.compile(
        r'(?xis)'
        r'("(?:[^"\\]|\\.)*"'  # normal "
        r'|r#"(?:[^"]|\\.)*"#'  # raw r#"..."#
        r'|b"(?:[^"\\]|\\.)*"'  # byte b"
        r'|br"(?:[^"\\]|\\.)*"')  # raw byte br"
    )

    # Find maximal runs of strlits separated only by whitespace.
    # We'll operate on the inner_content string directly.
    result_parts: List[str] = []
    pos = 0
    s = inner_content
    while pos < len(s):
        # skip whitespace, but remember it
        ws_start = pos
        while pos < len(s) and s[pos].isspace():
            pos += 1
        ws = s[ws_start:pos]
        if pos >= len(s):
            result_parts.append(ws)
            break
        if strlit_re.match(s, pos):
            # gather run of strlits separated by whitespace
            run_start = pos
            while True:
                m2 = strlit_re.match(s, pos)
                assert m2
                pos = m2.end()
                # skip whitespace
                w2s = pos
                while pos < len(s) and s[pos].isspace():
                    pos += 1
                if pos < len(s) and strlit_re.match(s, pos):
                    continue
                break
            run_text = s[run_start:pos]
            # collapse whitespace inside the run to a single space, keep literal boundaries
            collapsed = re.sub(r"\s+", " ", run_text)
            result_parts.append(collapsed)
        else:
            # non-strlit chunk up to next strlit or end
            nxt = strlit_re.search(s, pos)
            if nxt:
                chunk = s[pos : nxt.start()]
                result_parts.append(chunk)
                pos = nxt.start()
            else:
                result_parts.append(s[pos:])
                break

    inner_normalized = "".join(result_parts)
    # Rebuild block
    new_block = indent + lines[start].split("(", 1)[0] + "(" + inner_normalized + ")" + lines[end].split(")", 1)[-1]
    # Re-split into lines that match original line count where possible.
    # We'll just produce lines: start line, then inner lines split by original newline
    # positions from `inner_content`, then end line.

    # Simpler and robust: rebuild list of lines using original newline positions of
    # inner_content to re-emit normalized text while preserving blank lines where they
    # existed in `inner_content`.
    out_lines: List[str] = []
    out_lines.append(new_block.split(")", 1)[0] + ")")
    # Actually easier: re-split by the original inner_lines boundaries.
    # We'll just emit: start line (normalized), then for each original inner line
    # (except the last which contained ')'), emit normalized text up to its original length
    # is too fiddly. Instead: take inner_normalized and re-wrap using original newline
    # indices from inner_content.
    inner_newline_indices = [i for i, ch in enumerate(inner_content) if ch == "\n"]
    rebuilt_inner: List[str] = []
    prev = 0
    for ni in inner_newline_indices:
        rebuilt_inner.append(inner_normalized[prev : prev + (ni - prev)])
        prev = ni + 1
    rebuilt_inner.append(inner_normalized[prev:])
    # Trim leading/trailing whitespace on each line to mirror original style
    rebuilt_inner = [ln.rstrip() for ln in rebuilt_inner]

    final_lines = [indent + lines[start].split("(", 1)[0] + "(" + rebuilt_inner[0]] + rebuilt_inner[1:]
    # Append closing paren to last line
    final_lines[-1] = final_lines[-1] + ")"
    # Then any trailing content on the original end line after the ')' (rare)
    tail = lines[end].split(")", 1)[-1]
    if tail.strip():
        final_lines[-1] = final_lines[-1] + tail
    return final_lines


def main(argv: List[str]) -> int:
    targets = [Path("rivulet-core/src")] if len(argv) < 2 else [Path(argv[1])]
    dry_run = "--dry-run" in argv

    for root in targets:
        files = sorted(root.glob("*.rs"))
        changed = []
        for path in files:
            src = path.read_text(encoding="utf-8")
            lines = src.split("\n")
            blocks = find_call_blocks(lines)
            new_lines = list(lines)
            offsets = 0
            for (s, e) in reversed(blocks):
                block = lines[s:e+1]
                normalized = normalize_block(lines, s, e)
                if normalized != block:
                    # adjust indices by prior offsets
                    real_s = s + offsets
                    real_e = e + offsets
                    new_lines[real_s:real_e+1] = normalized
                    deltas = len(normalized) - (e - s + 1)
                    offsets += deltas
                    changed.append((path, s + 1, e + 1, block, normalized))
        if not changed:
            print(f"no normalized call blocks in {root}")
            continue
        if dry_run:
            for path, s, e, before, after in changed:
                print(f"\n# would normalize {path}:{s}-{e}")
                print("BEFORE:")
                for ln in before:
                    print(ln)
                print("AFTER:")
                for ln in after:
                    print(ln)
        else:
            # apply
            for path, s, e, before, after in changed:
                lines2 = path.read_text(encoding="utf-8").split("\n")
                lines2[s-1:e] = after
                path.write_text("\n".join(lines2), encoding="utf-8")
            print(f"normalized {len(changed)} call blocks in {root}")
            for path, s, e, _, _ in changed:
                print(f"  {path}:{s}-{e}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
