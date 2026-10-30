#!/usr/bin/env python3
"""Regenerate the public-API overview block in docs/api-overview.md.

`rivulet-core`'s public surface is declared in `rivulet-core/src/lib.rs` — the
`pub mod` lines and the `pub use` re-exports. Everything this script writes is
read back out of that file and the modules it names, so the overview cannot
describe a surface the crate does not have:

* the module list is the `pub mod` list, not a hand-kept table,
* each module's summary is the first line of its own `//!` doc comment,
* each module's size is its count of top-level `pub` items,
* the re-export table is the `pub use` list, i.e. the names a consumer reaches
  as `rivulet_core::…`,
* modules whose own documentation calls them experimental are listed with the
  line that says so, instead of being silently presented as stable.

Only the block between the markers is generated; the prose around it is
hand-written.

Usage:
    scripts/generate-api-overview.py          # rewrite the block in place
    scripts/generate-api-overview.py --check  # exit non-zero if the doc drifts
    scripts/generate-api-overview.py --write-baseline
                                              # freeze the current crate-root
                                              # re-exports as the removal baseline
    scripts/generate-api-overview.py --self-test
"""

from __future__ import annotations

import difflib
import re
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CRATE = "rivulet-core"

START_MARKER = "<!-- api-overview:start -->"
END_MARKER = "<!-- api-overview:end -->"

# Words a module's own documentation uses when it is not promising stability.
# The script reports the first matching line rather than deciding the class
# itself, and it reads doc comments only: `sort_unstable()` and a clippy allow
# for the unstable `Vec::as_slice` are implementation notes, not stability
# statements. Word boundaries keep `sort_unstable` out for the same reason.
EXPERIMENTAL_MARKER = re.compile(
    r"\b(experimental|unstable|not stable|subject to change)\b", re.IGNORECASE
)

PUB_ITEM = re.compile(r"^pub (?:fn|struct|enum|trait|const|static|type|mod|use) ")
PUB_MOD = re.compile(r"^pub mod ([A-Za-z0-9_]+);")
VERSION_LINE = re.compile(r'^version = "([^"]+)"')
SECTION_HEADER = re.compile(r"^\[([^\]]+)\]")

# The removal baseline read by `rivulet-core/tests/api_stability.rs`. It is the
# crate-root surface of the last released line; the guard fails when a name
# disappears from `docs/api-overview.md` without a migration row in
# `docs/api-migration.md`. It is updated when a release line opens, not when a
# guard fails (see the policy document).
BASELINE = Path("rivulet-core") / "tests" / "api_surface_baseline.txt"

BASELINE_HEADER = """\
# Crate-root re-exports of rivulet-core, frozen at the last released version.
#
# This file is the removal baseline for docs/api-migration.md: the
# `m8_api_removals_are_documented_in_the_migration_policy` test in
# rivulet-core/tests/api_stability.rs fails when a name listed here is gone from
# docs/api-overview.md and the "Applied migrations" table in
# docs/api-migration.md does not document it with a replacement and a note.
#
# Regenerate ONLY when a new release line opens, and only after every removal in
# that line has a migration row:
#
#     python scripts/generate-api-overview.py --write-baseline
#
# Regenerating it to silence a failing guard deletes the evidence the guard
# exists for.
"""


def crate_version(root: Path) -> str:
    """The version from `rivulet-core/Cargo.toml`, workspace-inherited or not."""
    manifest = root / CRATE / "Cargo.toml"
    version = None
    if manifest.is_file():
        for line in manifest.read_text(encoding="utf-8").splitlines():
            match = VERSION_LINE.match(line.strip())
            if match:
                version = match.group(1)
                break
    if version is None:
        # `rivulet-core` inherits `version.workspace = true`; the value lives in
        # the workspace's `[workspace.package]` table.
        root_manifest = (root / "Cargo.toml").read_text(encoding="utf-8")
        in_package = False
        for line in root_manifest.splitlines():
            section = SECTION_HEADER.match(line.strip())
            if section:
                in_package = section.group(1) == "workspace.package"
                continue
            if in_package:
                match = VERSION_LINE.match(line.strip())
                if match:
                    return match.group(1)
        return "unknown"
    return version


def module_source(root: Path, module: str) -> Path | None:
    for candidate in (
        root / CRATE / "src" / f"{module}.rs",
        root / CRATE / "src" / module / "mod.rs",
    ):
        if candidate.is_file():
            return candidate
    return None


def module_summary(path: Path) -> str:
    """The first `//!` doc line of a module, as its one-line summary."""
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("//!"):
            text = stripped[3:].strip()
            if text:
                return text
        elif stripped and not stripped.startswith("#!["):
            break
    return "(no module documentation)"


def public_item_count(path: Path) -> int:
    """Top-level `pub` items in a module file.

    Column-anchored on purpose: nested `pub` items inside `impl` blocks are part
    of an existing type's surface, not additional top-level API, and counting
    them by indentation would make the number depend on formatting.
    """
    return sum(
        1
        for line in path.read_text(encoding="utf-8").splitlines()
        if PUB_ITEM.match(line)
    )


def experimental_evidence(path: Path) -> tuple[int, str] | None:
    """First doc comment in the module that flags the module as not stable."""
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        stripped = line.strip()
        if not (stripped.startswith("//!") or stripped.startswith("///")):
            continue
        if EXPERIMENTAL_MARKER.search(stripped):
            return number, " ".join(stripped.split())
    return None


def parse_exports(lib: str) -> list[tuple[str, list[str]]]:
    """`pub use` statements as (module, items) pairs, in declaration order.

    Re-export statements span several lines (`pub use alerts::{A, B};`), so the
    text is joined up to the terminating semicolon before it is parsed.
    """
    exports: list[tuple[str, list[str]]] = []
    lines = lib.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index]
        if not line.startswith("pub use "):
            index += 1
            continue
        statement = line
        while ";" not in statement and index + 1 < len(lines):
            index += 1
            statement += " " + lines[index].strip()
        index += 1
        statement = statement.strip().rstrip(";")
        body = statement[len("pub use ") :]
        if "::" not in body:
            continue
        module, rest = body.split("::", 1)
        module = module.strip()
        rest = rest.strip()
        if rest.startswith("{") and rest.endswith("}"):
            items = [
                item.strip().rstrip(",")
                for item in rest[1:-1].split(",")
                if item.strip()
            ]
        else:
            items = [rest]
        exports.append((module, sorted(items)))
    return exports


def crate_lint_state(lib: str) -> str:
    for line in lib.splitlines():
        stripped = line.strip()
        if stripped.startswith("#![warn(missing_docs)]") or stripped.startswith(
            "#![deny(missing_docs)]"
        ):
            return "active"
    return "not active (the `//!` line in `lib.rs` is a doc-comment example)"


def examples(root: Path) -> list[str]:
    directory = root / CRATE / "examples"
    if not directory.is_dir():
        return []
    return sorted(
        path.stem for path in directory.iterdir() if path.suffix == ".rs"
    )


def build_overview(root: Path) -> str:
    lib_source = (root / CRATE / "src" / "lib.rs").read_text(encoding="utf-8")
    modules: list[str] = []
    for line in lib_source.splitlines():
        match = PUB_MOD.match(line)
        if match:
            modules.append(match.group(1))
    modules.sort()

    exports = parse_exports(lib_source)
    exported_modules = {module for module, _ in exports}
    example_names = examples(root)

    lines = [
        "<!-- Generated by scripts/generate-api-overview.py — do not edit by hand. -->",
        "",
        f"- Crate: `{CRATE}` {crate_version(root)}",
        f"- Public modules: **{len(modules)}**",
        f"- Crate-root re-exports: **{len(exports)}** statements",
        f"- `#![warn(missing_docs)]`: **{crate_lint_state(lib_source)}**",
        f"- Runnable examples: **{len(example_names)}** "
        f"({', '.join(f'`{name}`' for name in example_names)})",
        "",
        "### Public modules",
        "",
        "| Module | Summary | Top-level public items | Re-exported from the crate root |",
        "| --- | --- | --- | --- |",
    ]
    for module in modules:
        path = module_source(root, module)
        if path is None:
            summary = "(declared in `lib.rs`, no source file found)"
            count = "-"
        else:
            summary = module_summary(path)
            count = str(public_item_count(path))
        reachable = "yes" if module in exported_modules else "no"
        lines.append(f"| `{module}` | {summary} | {count} | {reachable} |")

    lines += [
        "",
        "### Crate-root re-exports",
        "",
        "These are the names a foreign crate reaches as `rivulet_core::…`; the",
        "module index above marks every other `pub mod` as reachable only through",
        "its own path.",
        "",
        "| From module | Names |",
        "| --- | --- |",
    ]
    for module, items in exports:
        names = ", ".join(f"`{item}`" for item in items)
        lines.append(f"| `{module}` | {names} |")

    experimental = []
    for module in modules:
        path = module_source(root, module)
        if path is None:
            continue
        evidence = experimental_evidence(path)
        if evidence:
            number, text = evidence
            experimental.append((module, path.relative_to(root).as_posix(), number, text))
    lines += [
        "",
        "### Modules whose own documentation calls them experimental",
        "",
        "A module whose own documentation says so is not covered by the 1.0",
        "stability promise, whatever else the milestone claims.",
        "",
        "| Module | Evidence |",
        "| --- | --- |",
    ]
    if experimental:
        for module, path, number, text in experimental:
            lines.append(f"| `{module}` | `{path}:{number}` — {text} |")
    else:
        lines.append("| — | no module documents itself as experimental |")

    lines.append("")
    return "\n".join(lines)


def replace_block(doc: str, block: str) -> str:
    start = doc.index(START_MARKER) + len(START_MARKER)
    end = doc.index(END_MARKER)
    return doc[:start] + "\n" + block + doc[end:]


def check(root: Path) -> int:
    doc_path = root / "docs" / "api-overview.md"
    if not doc_path.is_file():
        print(f"{doc_path} is missing; run scripts/generate-api-overview.py", file=sys.stderr)
        return 1
    doc = doc_path.read_text(encoding="utf-8")
    if START_MARKER not in doc or END_MARKER not in doc:
        print(f"{doc_path} lost its {START_MARKER} / {END_MARKER} markers", file=sys.stderr)
        return 1
    expected = replace_block(doc, build_overview(root))
    if expected == doc:
        print("api-overview: docs/api-overview.md matches rivulet-core/src/lib.rs")
        return 0
    diff = difflib.unified_diff(
        doc.splitlines(keepends=True),
        expected.splitlines(keepends=True),
        fromfile="docs/api-overview.md",
        tofile="docs/api-overview.md (generated)",
    )
    sys.stderr.writelines(diff)
    print(
        "api-overview: docs/api-overview.md is stale — run "
        "scripts/generate-api-overview.py",
        file=sys.stderr,
    )
    return 1


def write(root: Path) -> int:
    doc_path = root / "docs" / "api-overview.md"
    doc = doc_path.read_text(encoding="utf-8")
    doc_path.write_text(replace_block(doc, build_overview(root)), encoding="utf-8")
    print(f"api-overview: rewrote the generated block in {doc_path}")
    return 0


def export_names(root: Path) -> list[str]:
    """Every crate-root name a consumer reaches as `rivulet_core::…`."""
    lib_source = (root / CRATE / "src" / "lib.rs").read_text(encoding="utf-8")
    return sorted({item for _, items in parse_exports(lib_source) for item in items})


def documented_removals(root: Path) -> set[str]:
    """Names with a row in the "Applied migrations" table of the policy doc."""
    path = root / "docs" / "api-migration.md"
    if not path.is_file():
        return set()
    documented: set[str] = set()
    inside = False
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("## "):
            inside = "applied migrations" in stripped.lower()
            continue
        if not inside or not stripped.startswith("|"):
            continue
        cells = [cell.strip() for cell in stripped.strip("|").split("|")]
        if len(cells) < 4:
            continue
        item, _removed_in, replacement, note = cells
        # A migration row needs a replacement and a note; "-" is a placeholder,
        # not a migration path ("removed" is not a migration note either).
        if not replacement or replacement in {"-", "—"} or not note:
            continue
        documented.update(re.findall(r"`([^`]+)`", item))
    return documented


def write_baseline(root: Path) -> int:
    path = root / BASELINE
    names = export_names(root)
    previous: set[str] = set()
    if path.is_file():
        previous = {
            line.strip()
            for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.strip().startswith("#")
        }
    documented = documented_removals(root)
    undocumented = sorted(name for name in previous - set(names) if name not in documented)
    if undocumented:
        print(
            "api-overview: refusing to drop names without a migration row in "
            "docs/api-migration.md: " + ", ".join(undocumented),
            file=sys.stderr,
        )
        return 1
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(BASELINE_HEADER + "".join(f"{name}\n" for name in names), encoding="utf-8")
    print(f"api-overview: wrote {len(names)} names to {path}")
    return 0


FIXTURE_LIB = """\
//! Fixture crate.
pub mod alpha;
pub use alpha::{Alpha, ALPHA_LIMIT};
pub mod beta;
pub use beta::Beta;

pub struct Direct;
"""

FIXTURE_ALPHA = """\
//! Alpha module.
//!
//! More prose.
pub struct Alpha;
pub const ALPHA_LIMIT: usize = 3;
pub fn hidden() {}
"""

FIXTURE_BETA = """\
//! Beta module, experimental while the backend lands.
pub struct Beta;
"""

FIXTURE_MANIFEST = """\
[package]
name = "fixture"

[workspace.package]
version = "9.9.9-fixture"
"""


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "docs").mkdir()
        (root / "rivulet-core" / "src").mkdir(parents=True)
        (root / "rivulet-core" / "examples").mkdir()
        (root / "Cargo.toml").write_text(FIXTURE_MANIFEST, encoding="utf-8")
        (root / "rivulet-core" / "Cargo.toml").write_text(
            "[package]\nname = \"rivulet-core\"\nversion.workspace = true\n",
            encoding="utf-8",
        )
        (root / "rivulet-core" / "src" / "lib.rs").write_text(FIXTURE_LIB, encoding="utf-8")
        (root / "rivulet-core" / "src" / "alpha.rs").write_text(FIXTURE_ALPHA, encoding="utf-8")
        (root / "rivulet-core" / "src" / "beta.rs").write_text(FIXTURE_BETA, encoding="utf-8")
        (root / "rivulet-core" / "examples" / "demo.rs").write_text("fn main() {}", encoding="utf-8")

        overview = build_overview(root)
        for expected in [
            "| `alpha` | Alpha module. | 3 | yes |",
            "| `beta` | Beta module, experimental while the backend lands. | 1 | yes |",
            # Items are sorted, so the constant precedes the type.
            "`ALPHA_LIMIT`, `Alpha`",
            "| `beta` | `Beta` |",
            "`rivulet-core/src/beta.rs:1` — //! Beta module, experimental while the backend lands.",
            "9.9.9-fixture",
            "`demo`",
        ]:
            assert expected in overview, f"missing from the generated overview: {expected}"

        # A module without a crate-root re-export must be marked as such.
        (root / "rivulet-core" / "src" / "gamma.rs").write_text(
            "//! Gamma module.\npub struct Gamma;\n", encoding="utf-8"
        )
        lib = (root / "rivulet-core" / "src" / "lib.rs").read_text(encoding="utf-8")
        (root / "rivulet-core" / "src" / "lib.rs").write_text(
            lib + "pub mod gamma;\n", encoding="utf-8"
        )
        overview = build_overview(root)
        assert "| `gamma` | Gamma module. | 1 | no |" in overview, overview

        # `--check` must catch a stale document and accept a fresh one.
        (root / "docs" / "api-overview.md").write_text(
            f"# Fixture\n\n{START_MARKER}\nstale\n{END_MARKER}\n", encoding="utf-8"
        )
        assert check(root) == 1, "--check must reject a stale block"
        assert write(root) == 0, "write must succeed"
        assert check(root) == 0, "--check must accept the freshly generated block"

        # Drift detection: a new module in the tree must invalidate the doc.
        lib = (root / "rivulet-core" / "src" / "lib.rs").read_text(encoding="utf-8")
        (root / "rivulet-core" / "src" / "lib.rs").write_text(
            lib + "pub mod delta;\npub use delta::Delta;\n", encoding="utf-8"
        )
        (root / "rivulet-core" / "src" / "delta.rs").write_text(
            "//! Delta module.\npub struct Delta;\n", encoding="utf-8"
        )
        assert check(root) == 1, "--check must reject a module the doc does not list"

        # The removal baseline: written from the surface, and refusing to drop a
        # name that the migration policy does not document.
        baseline = root / BASELINE
        assert write_baseline(root) == 0, "the first baseline write must succeed"
        written = baseline.read_text(encoding="utf-8")
        assert "Alpha\n" in written and "ALPHA_LIMIT\n" in written, written
        assert "`Beta`" not in written, "the baseline holds names, not backticks"

        lib = (root / "rivulet-core" / "src" / "lib.rs").read_text(encoding="utf-8")
        without_limit = lib.replace("pub use alpha::{Alpha, ALPHA_LIMIT};", "pub use alpha::Alpha;")
        (root / "rivulet-core" / "src" / "lib.rs").write_text(without_limit, encoding="utf-8")
        assert (
            write_baseline(root) == 1
        ), "removing a name without a migration row must refuse the baseline update"

        (root / "docs" / "api-migration.md").write_text(
            "## Applied migrations\n\n"
            "| Removed or changed item | Removed in | Replacement | Migration note |\n"
            "| --- | --- | --- | --- |\n"
            "| `ALPHA_LIMIT` | 9.9.9 | `Alpha::LIMIT` | call the constant on the type |\n",
            encoding="utf-8",
        )
        assert write_baseline(root) == 0, "a documented removal must be allowed"
        assert "ALPHA_LIMIT\n" not in baseline.read_text(encoding="utf-8")

    print("api-overview: self-test passed")
    return 0


def main(argv: list[str]) -> int:
    arguments = argv[1:]
    unknown = [
        arg
        for arg in arguments
        if arg not in {"--check", "--self-test", "--write-baseline"}
    ]
    if unknown:
        print(f"unknown arguments: {' '.join(unknown)}", file=sys.stderr)
        return 2
    if "--self-test" in arguments:
        return self_test()
    if "--write-baseline" in arguments:
        return write_baseline(REPO_ROOT)
    if "--check" in arguments:
        return check(REPO_ROOT)
    return write(REPO_ROOT)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
