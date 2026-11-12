#!/usr/bin/env python3
"""Find undocumented public items in *every* `cfg` branch, not just one platform.

`cargo doc -p rivulet-core --no-deps` with `RUSTDOCFLAGS: -D warnings` is the
real guard for missing docs, but it only ever sees the branches that compile on
the machine running it. The Lints job runs on Linux, so a Windows-only item is
checked only by whoever happens to run the doc build on Windows — and vice
versa. That is not hypothetical: `ShmFrameReader`'s non-Windows stub methods in
`rivulet-core/src/opengl_hook.rs` stayed undocumented through #342/#433 because
every `missing_docs` count ran on Windows, where that `impl` is not compiled,
and the Lints job found them only when #436 changed Linux code and made CI run.

This script reads the sources textually, so it works the same on every runner:

* it walks the modules reachable from `lib.rs` (`pub mod` -> `src/x.rs` or
  `src/x/mod.rs`), mirroring what rustdoc considers the public surface; a
  private module's `pub` items are not linted by `missing_docs` and are skipped
  here too;
* it tracks `#[cfg(...)]` attributes on items *and* on their enclosing `mod`/
  `impl`/`trait` blocks, so an item is reported together with the gate that
  hides it from the current platform;
* it applies the same reachability rules rustdoc does for the common shapes:
  `impl Trait for Type` bodies and function bodies are not part of the checked
  surface, inline `#[cfg(test)] mod tests` blocks are ignored, and `pub mod`
  declarations are skipped because `generate-api-overview.py` already guards
  the module list and each module's `//!` summary.

Item kinds checked: `fn`, `struct`, `enum`, `trait`, `type`, `const`, `static`,
`union` (plus `fn`/`const`/`type` inside `trait` declarations, which are public
without a `pub` keyword). Struct and enum *fields* are not checked: their
visibility rules are subtle and, unlike whole items, they are never what a
platform gate hides.

Usage:
    scripts/check-platform-docs.py            # scan rivulet-core, exit 1 on findings
    scripts/check-platform-docs.py --file F    # scan one file as a public root
    scripts/check-platform-docs.py --path DIR  # scan a synthetic crate directory
    scripts/check-platform-docs.py --self-test # fixture cases, no repo state

Exit codes: 0 when every public item is documented, 1 otherwise.
"""

import re
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SRC = REPO_ROOT / "rivulet-core" / "src"

# A gate hides an item from the running platform when it names an OS, a family
# or an architecture. `cfg(test)`, `cfg(feature = ...)` and friends are not
# platform gates: rustdoc sees those branches on every runner too.
PLATFORM_GATE = re.compile(
    r"target_os|target_family|target_arch|\bunix\b|\bwindows\b", re.IGNORECASE
)

DOC_ATTR = re.compile(r"^\[\s*doc\b")
CFG_ATTR = re.compile(r"^\[\s*cfg\s*\((.*)\)\s*\]$", re.DOTALL)

# `pub` with any modifier rustfmt leaves in front of the keyword.
PUB_ITEM = re.compile(
    r"^pub\s+(?:(?:async|unsafe|const|default)\s+|extern\s+(?:\"[^\"]*\"\s+)?)*"
    r"(fn|struct|enum|trait|type|const|static|union)\s+([A-Za-z0-9_]+)"
)
# Trait items are public without `pub`.
TRAIT_ITEM = re.compile(
    r"^(?:(?:async|unsafe|const|default)\s+|extern\s+(?:\"[^\"]*\"\s+)?)*"
    r"(fn|type|const)\s+([A-Za-z0-9_]+)"
)


def blank_out(text):
    """Return (code, doc_lines).

    `code` has string/char literal contents and comments blanked out (newlines
    kept, so offsets and line numbers still line up), which makes brace and
    attribute scanning safe. `doc_lines` holds the 1-based line numbers that
    start a `///` doc comment; `////` separators and `//!` inner docs are not
    item docs.
    """
    out = []
    doc_lines = set()
    line = 1
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        if ch == "\n":
            out.append(ch)
            line += 1
            i += 1
            continue
        if text.startswith("//", i):
            end = text.find("\n", i)
            if end == -1:
                end = n
            segment = text[i:end]
            if segment.startswith("///") and not segment.startswith("////"):
                doc_lines.add(line)
            out.append(" " * (end - i))
            i = end
            continue
        if text.startswith("/*", i):
            depth = 1
            j = i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    if text[j] == "\n":
                        out.append("\n")
                        line += 1
                    else:
                        out.append(" ")
                    j += 1
            i = j
            continue
        if ch == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            # One output character per input character: the attribute slice is
            # read from the original text by index, so lengths must not drift.
            for value in text[i:j]:
                if value == "\n":
                    out.append("\n")
                    line += 1
                else:
                    out.append(" ")
            i = j
            continue
        if ch == "r" and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            j = i + 1
            hashes = 0
            while j < n and text[j] == "#":
                hashes += 1
                j += 1
            if j < n and text[j] == '"':
                terminator = '"' + "#" * hashes
                end = text.find(terminator, j + 1)
                end = n if end == -1 else end + len(terminator)
                for value in text[i:end]:
                    if value == "\n":
                        out.append("\n")
                        line += 1
                    else:
                        out.append(" ")
                i = end
                continue
        if ch == "'":
            # Char literal or lifetime? `'a` is a lifetime, `'x'` / `'\n'` a literal.
            if i + 1 < n and text[i + 1] == "\\":
                j = i + 2
                while j < n and text[j] != "'":
                    j += 1
                j = min(j + 1, n)
                out.append(" " * (j - i))
                i = j
                continue
            if i + 2 < n and text[i + 2] == "'":
                out.append("   ")
                i += 3
                continue
            out.append(ch)
            i += 1
            continue
        out.append(ch)
        i += 1
    return "".join(out), doc_lines


def classify_header(header):
    """Classify the construct that owns the `{` we just reached."""
    text = " ".join(header.split())
    if not text:
        return "body"
    if re.match(r"^(?:pub\s+)?(?:default\s+)?(?:unsafe\s+)?impl\b", text):
        # `impl Trait for Type` bodies are not part of the documented surface.
        return "impl_trait" if " for " in text else "impl"
    if re.search(r"\bmod\s+[A-Za-z0-9_]+$", text):
        return "mod"
    if re.search(r"\btrait\s+[A-Za-z0-9_]+", text):
        return "trait"
    if re.search(r"\b(fn|struct|enum|union|extern)\b", text):
        # Function bodies and struct/enum field lists hold no items this guard
        # judges; stopping here also keeps nested braces from confusing it.
        return "body"
    return "body"


def matching_bracket(code, start):
    """Return the index of the `]` closing the `[` at `start`."""
    depth = 0
    for index in range(start, len(code)):
        if code[index] == "[":
            depth += 1
        elif code[index] == "]":
            depth -= 1
            if depth == 0:
                return index
    return len(code) - 1


def gate_of(attrs):
    """Return the platform gate expressed by `attrs`, or None."""
    gates = []
    for attr in attrs:
        match = CFG_ATTR.match(attr)
        if match and PLATFORM_GATE.search(match.group(1)):
            gates.append(" ".join(match.group(1).split()))
    return " && ".join(gates) if gates else None


def is_test_module(attrs, kind):
    if kind != "mod":
        return False
    for attr in attrs:
        match = CFG_ATTR.match(attr)
        if match and re.search(r"\btest\b", match.group(1)):
            return True
    return False


def scan_source(label, text, root_public=True):
    """Return (findings, stats) for one source file."""
    code, doc_lines = blank_out(text)
    findings = []
    stats = {"items": 0, "gated": 0}

    scopes = []  # {"kind": ..., "public": bool, "test": bool, "gate": str|None}
    header = []
    header_line = 1
    attrs = []
    doc_pending = False
    line = 1
    index = 0
    total = len(code)

    def visible_scope():
        for scope in scopes:
            if scope["test"] or scope["kind"] in ("body", "impl_trait", "struct"):
                return False
            if scope["kind"] == "mod" and not scope["public"]:
                return False
        return True

    def scope_gates():
        return [scope["gate"] for scope in scopes if scope["gate"]]

    def record(header_text, current_line):
        nonlocal doc_pending
        normalized = " ".join(header_text.split())
        in_trait = bool(scopes) and scopes[-1]["kind"] == "trait"
        match = PUB_ITEM.match(normalized)
        if not match and in_trait:
            match = TRAIT_ITEM.match(normalized)
        if not match:
            doc_pending = False
            return
        if not visible_scope():
            doc_pending = False
            return
        if normalized.startswith("pub(") and not normalized.startswith("pub(crate)"):
            doc_pending = False
            return
        if normalized.startswith("pub(crate)"):
            doc_pending = False
            return
        kind, name = match.group(1), match.group(2)
        if kind == "mod":
            # `generate-api-overview.py` guards the module list and summaries.
            doc_pending = False
            return
        gate = gate_of(attrs) or (" && ".join(gate for gate in scope_gates() if gate) or None)
        stats["items"] += 1
        if gate:
            stats["gated"] += 1
        if not doc_pending:
            findings.append(
                {
                    "path": label,
                    "line": current_line,
                    "kind": kind,
                    "name": name,
                    "gate": gate,
                    "scope": scopes[-1]["kind"] if scopes else "mod",
                }
            )
        doc_pending = False

    while index < total:
        ch = code[index]
        if ch == "\n":
            if not header:
                header_line = line + 1
            if line in doc_lines:
                doc_pending = True
            line += 1
            index += 1
            continue
        if ch == "#" and code.startswith("[", index + 1):
            end = matching_bracket(code, index + 1)
            # Read the attribute from the original text: the blanked copy hides
            # the string literals, and `not(target_os = "windows")` is exactly
            # the part that decides whether the gate is a platform gate.
            attr = text[index + 1 : end + 1]
            if DOC_ATTR.match(attr):
                doc_pending = True
            else:
                attrs.append(attr)
            line += code.count("\n", index, end + 1)
            index = end + 1
            continue
        if ch == "{":
            kind = classify_header("".join(header))
            gated = gate_of(attrs)
            public = bool(re.match(r"^pub\b", " ".join("".join(header).split())))
            record("".join(header), header_line)
            scopes.append(
                {
                    "kind": kind,
                    "public": public,
                    "test": is_test_module(attrs, kind),
                    "gate": gated,
                }
            )
            header = []
            attrs = []
            doc_pending = False
            index += 1
            continue
        if ch == "}":
            if scopes:
                scopes.pop()
            header = []
            attrs = []
            doc_pending = False
            index += 1
            continue
        if ch == ";":
            record("".join(header), header_line)
            header = []
            attrs = []
            doc_pending = False
            index += 1
            continue
        if not header and not ch.isspace():
            header_line = line
        header.append(ch)
        index += 1

    return findings, stats


def reachable_files(src_dir):
    """Resolve the `pub mod` tree from `lib.rs` the way rustdoc sees it."""
    files = []
    seen = set()

    def resolve(name, current):
        candidates = [
            current.parent / f"{name}.rs",
            current.parent / name / "mod.rs",
            src_dir / f"{name}.rs",
            src_dir / name / "mod.rs",
        ]
        for candidate in candidates:
            if candidate.is_file():
                return candidate
        return None

    def visit(path):
        if path in seen:
            return
        seen.add(path)
        files.append(path)
        try:
            text = path.read_text(encoding="utf-8")
        except OSError:
            return
        for match in re.finditer(r"^\s*pub\s+mod\s+([A-Za-z0-9_]+)\s*;", text, re.MULTILINE):
            target = resolve(match.group(1), path)
            if target is not None:
                visit(target)

    root = src_dir / "lib.rs"
    if not root.is_file():
        raise SystemExit(f"no lib.rs under {src_dir}")
    visit(root)
    return files


def render(findings, stats, checked):
    lines = []
    for finding in findings:
        gate = f" [gate: {finding['gate']}]" if finding["gate"] else ""
        lines.append(
            f"{finding['path']}:{finding['line']}: pub {finding['kind']} "
            f"{finding['name']}{gate}"
        )
    lines.append(
        f"{stats['items']} public items checked in {checked} file(s), "
        f"{stats['gated']} platform-gated, {len(findings)} undocumented"
    )
    return lines


def run_scan(paths, root_public=True):
    findings = []
    stats = {"items": 0, "gated": 0}
    for path in paths:
        text = path.read_text(encoding="utf-8")
        file_findings, file_stats = scan_source(str(path), text, root_public)
        findings.extend(file_findings)
        stats["items"] += file_stats["items"]
        stats["gated"] += file_stats["gated"]
    return findings, stats


def run_self_test():
    """Fixture cases; needs neither network nor repository state."""
    failures = 0

    def case(name, ok, detail=""):
        nonlocal failures
        print(("PASS" if ok else "FAIL") + f" [{name}]" + (f": {detail}" if not ok else ""))
        if not ok:
            failures += 1

    source = '''//! fixture module

/// Documented stub.
#[cfg(not(target_os = "windows"))]
pub struct Stub;

#[cfg(not(target_os = "windows"))]
impl Stub {
    /// Documented.
    pub fn documented(&self) -> u8 {
        0
    }

    pub fn undocumented(&self) {}
}

#[cfg(target_os = "windows")]
pub fn gated_free_function() {}

#[cfg(target_os = "windows")]
/// Documented.
pub fn gated_documented_function() {}

#[cfg(test)]
mod tests {
    pub fn helper() {}
}

mod private {
    pub fn hidden() {}
}

/// Documented container.
pub struct Container {
    pub fields_are_not_judged: u8,
}

/// Documented trait.
pub trait Surface {
    /// Documented.
    fn documented(&self);

    fn undocumented_trait_method(&self);
}

/// Documented body.
pub fn quoted_braces_do_not_shift_scopes() {
    let _ = format!("{{ not a scope }}");
    let _ = r#"raw " { } "#;
    let _ = '{';
}
'''

    findings, stats = scan_source("fixture.rs", source)
    names = sorted(f["name"] for f in findings)
    gates = {f["name"]: f["gate"] for f in findings}

    case(
        "undocumented items are exactly the three expected ones",
        names == ["gated_free_function", "undocumented", "undocumented_trait_method"],
        str(names),
    )
    case(
        "the hidden gate is reported with the item",
        gates.get("undocumented") == 'not(target_os = "windows")',
        str(gates.get("undocumented")),
    )
    case(
        "a windows-only gate is reported",
        gates.get("gated_free_function") == 'target_os = "windows"',
        str(gates.get("gated_free_function")),
    )
    case("documented impl method is not reported", "documented" not in names)
    case(
        "cfg before the doc comment still counts as documented",
        "gated_documented_function" not in names,
    )
    case("cfg(test) items are ignored", "helper" not in names)
    case("private module items are ignored", "hidden" not in names)
    case("struct fields are not judged", stats["items"] == 10, str(stats["items"]))
    case(
        "braces inside string literals do not shift scopes",
        "quoted_braces_do_not_shift_scopes" not in names,
    )

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp) / "src"
        root.mkdir()
        (root / "lib.rs").write_text(
            "//! crate\n\npub mod gated;\n\nmod unreachable;\n", encoding="utf-8"
        )
        (root / "gated.rs").write_text(
            "//! module\n\n#[cfg(target_os = \"linux\")]\npub fn undocumented() {}\n",
            encoding="utf-8",
        )
        (root / "unreachable.rs").write_text("pub fn not_reachable() {}\n", encoding="utf-8")
        files = reachable_files(root)
        case(
            "reachability follows `pub mod` only",
            [path.name for path in files] == ["lib.rs", "gated.rs"],
            str([path.name for path in files]),
        )
        found, _ = run_scan(files)
        case(
            "gated item in a public module file is reported",
            [item["name"] for item in found] == ["undocumented"],
            str([item["name"] for item in found]),
        )

    if failures:
        print(f"{failures} self-test case(s) FAILED")
        return 1
    print("All self-test cases passed.")
    return 0


def main():
    args = sys.argv[1:]
    if "--help" in args or "-h" in args:
        print(__doc__.strip())
        return 0
    if "--self-test" in args:
        return run_self_test()

    targets = []
    if "--file" in args:
        targets = [Path(args[args.index("--file") + 1])]
        files = targets
    else:
        src = DEFAULT_SRC
        if "--path" in args:
            src = Path(args[args.index("--path") + 1])
        files = reachable_files(src)

    findings, stats = run_scan(files)
    for output_line in render(findings, stats, len(files)):
        print(output_line)
    if findings:
        print(
            f"\n{len(findings)} public item(s) without documentation; "
            "run `cargo doc` on the platform each gate names, or add the doc comment here",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
