#!/usr/bin/env python3
"""Remove trailing illegal //! blocks that appear AFTER the closing brace of
`#[cfg(test)] mod tests` in rivulet-core/src/*.rs.

Illegal here means: inner doc comments (`//!`) at file/module scope that
appear after the last item in the file (typically after the `mod tests { ... }`
block). Rust requires inner doc comments only before items, and a bare
"//! ... \r\n//! #[allow(missing_docs)]" tail after the closing brace of the
test module is exactly the shape that emits E0753 / "bare CR not allowed in
doc-comment".

This script is deliberately conservative: it only removes trailing doc-comment
lines that would be illegal if they followed an item. It never touches leading
module-level //! comments that preceed the first item.
"""
import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("rivulet-core/src")

# Lines that are entirely a doc comment, possibly with whitespace, possibly
# ending in \r due to mixed line endings in the repo.
TRAILING_DOC = re.compile(r"^\s*///\s*$")
TRAILING_INNER_DOC = re.compile(r"^\s*//!.*\s*$")
# Matches an inline doc-comment ending with content **and** an inner-attribute
# doc-comment fragment, e.g. "//! ... \n//! #[allow(missing_docs)]". We treat
# the whole tail block up to (but not including) the first empty line after the
# closing `}` as removable if it is all //! lines.

raw = "\r\n"
end_marker = re.compile(r"^\s*}\s*$", re.M)


def normalize_newlines(text: str) -> str:
    return text.replace("\r\n", "\n").replace("\r", "\n")


def strip_tail(text: str) -> str:
    lines = text.split("\n")
    # Find the last non-blank non-doc Comment line that is an actual Rust item
    # end (a closing brace at module level). We only want to cut the very tail
    # if it's purely inner doc comments. Anything else we keep untouched.
    cut_index = len(lines)
    for i in range(len(lines) - 1, -1, -1):
        line = lines[i].rstrip("\n")
        if line.strip() == "":
            continue
        if TRAILING_INNER_DOC.match(line):
            cut_index = i
            continue
        # OK, first non-empty non-doc line reached from the end — stop.
        break
    else:
        # Entire file is blank/doc-comment; leave it alone.
        return text

    if cut_index == len(lines):
        return text

    # Verify everything after cut_index (exclusive) is only inner doc comments
    # and blank lines.
    after = lines[cut_index:]
    if any(line.strip() and not TRAILING_INNER_DOC.match(line) for line in after):
        return text

    # Remove the trailing inner-doc-comment block + any trailing blank lines
    # so we end on the last real item line (which is at cut_index - 1).
    before = lines[:cut_index]
    # Trim trailing blank lines from before, but keep at most one blank line
    # before EOF for readability.
    while before and before[-1].strip() == "":
        before.pop()
    return "\n".join(before) + "\n"


def main() -> None:
    changed = []
    for path in sorted(ROOT.glob("*.rs")):
        raw_bytes = path.read_bytes()
        text = raw_bytes.decode("utf-8")
        normalized = normalize_newlines(text)
        stripped = strip_tail(normalized)
        if stripped == normalized:
            continue
        changed.append(path)
        path.write_text(stripped, encoding="utf-8")

    if changed:
        print(f"stripped illegal //! tails from {len(changed)} files:")
        for p in changed:
            print(f"  {p}")
    else:
        print("no files needed stripping")


if __name__ == "__main__":
    main()
