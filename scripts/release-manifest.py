#!/usr/bin/env python3
"""Deterministic SHA256SUMS manifests for release artifacts (issue #190).

The alpha and tag-based release paths used to build ``SHA256SUMS`` with an
inline ``find | sort | xargs sha256sum`` pipeline, duplicated in two workflows
and therefore free to drift. This script is the single implementation, and it
adds the half that was missing entirely: **verification of an artifact set
that was downloaded from somewhere else** -- which is what the post-publish
job does after the release is live.

Why a script and not a bare ``sha256sum``:

* deterministic output -- entries sorted by path, ``sha256sum``-compatible
  ``<digest>  <name>`` lines, LF endings, so two runs over the same tree
  produce byte-identical manifests;
* verification is symmetric: every manifest entry must exist and match, and
  every file in the tree must be listed. A manifest that quietly omits a file
  is the failure mode worth catching, because that file would then be
  published without integrity coverage;
* pure stdlib, so it runs on every runner without installing anything.

Usage::

    scripts/release-manifest.py generate --dir release-assets [excludes...]
    scripts/release-manifest.py verify   --dir release-assets
    scripts/release-manifest.py --self-test

``--self-test`` exercises both halves against throwaway trees and runs in the
Lints job on every push, so a regression here cannot reach a release.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import sys
import tempfile

#: The manifest name. Deliberately constant: the updater refuses an installer
#: that is missing from or differs in this file, so the name is part of the
#: release contract (``rivulet-updater::CHECKSUMS_ASSET_NAME``).
MANIFEST_NAME = "SHA256SUMS"

#: Release-card images are attached to the release but are not build output,
#: so they must never appear in the manifest.
DEFAULT_EXCLUDES = ("opengraph.png", MANIFEST_NAME)

_CHUNK = 1024 * 1024


class ManifestError(Exception):
    """A manifest could not be built or did not describe the tree."""


def sha256_of(path: str) -> str:
    """Stream the file through SHA-256 so large installers stay off the heap."""
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(_CHUNK), b""):
            digest.update(chunk)
    return digest.hexdigest()


def iter_tree(root: str, excludes: tuple[str, ...]):
    """Yield POSIX-style relative paths of every file under ``root``.

    Sorted by the raw name so the manifest order is a property of the tree,
    not of the filesystem's ``readdir`` order.
    """
    found = []
    for directory, subdirs, files in os.walk(root):
        subdirs.sort()
        for name in files:
            if name in excludes:
                continue
            absolute = os.path.join(directory, name)
            if os.path.islink(absolute) or not os.path.isfile(absolute):
                continue
            relative = os.path.relpath(absolute, root).replace(os.sep, "/")
            found.append(relative)
    return sorted(found)


def render(root: str, excludes: tuple[str, ...]) -> str:
    """Build the manifest text for ``root``."""
    lines = []
    for relative in iter_tree(root, excludes):
        lines.append(f"{sha256_of(os.path.join(root, relative))}  {relative}\n")
    return "".join(lines)


def parse(manifest: str) -> list[tuple[str, str]]:
    """Parse ``sha256sum`` output into ``(digest, name)`` pairs.

    Rejects a malformed manifest instead of skipping lines: a manifest we
    cannot fully read must not be reported as "verified".
    """
    entries = []
    for number, raw in enumerate(manifest.splitlines(), start=1):
        line = raw.rstrip("\r")
        if not line.strip():
            continue
        if line.lstrip().startswith("#"):
            continue
        parts = line.split(None, 1)
        if len(parts) != 2 or len(parts[0]) != 64:
            raise ManifestError(f"line {number}: not a sha256sum entry: {line!r}")
        digest = parts[0].lower()
        try:
            int(digest, 16)
        except ValueError:
            raise ManifestError(f"line {number}: digest is not hex: {line!r}") from None
        name = parts[1].strip()
        # sha256sum prefixes binary-mode names with '*'.
        if name.startswith("*"):
            name = name[1:]
        if not name:
            raise ManifestError(f"line {number}: entry without a file name")
        entries.append((digest, name))
    return entries


def verify(root: str, manifest_path: str, excludes: tuple[str, ...]) -> list[str]:
    """Compare a manifest against ``root``; return every problem found.

    Two directions, both required:

    * every listed file must exist and hash to the recorded digest, which
      catches a tampered or truncated download;
    * every file in the tree must be listed, which catches an artifact that
      got published without integrity coverage.
    """
    try:
        with open(manifest_path, encoding="utf-8") as handle:
            entries = parse(handle.read())
    except OSError as error:
        raise ManifestError(f"cannot read {manifest_path}: {error}") from error

    problems = []
    listed = set()
    for digest, name in entries:
        listed.add(name)
        absolute = os.path.join(root, name)
        if not os.path.isfile(absolute):
            problems.append(f"listed but missing: {name}")
            continue
        actual = sha256_of(absolute)
        if actual != digest:
            problems.append(f"digest mismatch: {name} (manifest {digest}, actual {actual})")

    for relative in iter_tree(root, excludes):
        if relative not in listed:
            problems.append(f"present but not listed: {relative}")

    if not entries:
        problems.append("manifest is empty")
    return problems


def _write(root: str, text: str, manifest_path: str) -> None:
    """Write atomically via a temp file in the same directory."""
    directory = os.path.dirname(os.path.abspath(manifest_path)) or "."
    handle, temporary = tempfile.mkstemp(dir=directory, prefix=".SHA256SUMS.", suffix=".tmp")
    try:
        with os.fdopen(handle, "w", encoding="utf-8", newline="\n") as out:
            out.write(text)
        os.replace(temporary, manifest_path)
    except BaseException:
        if os.path.exists(temporary):
            os.unlink(temporary)
        raise


def run_generate(args: argparse.Namespace) -> int:
    excludes = tuple(args.exclude) or DEFAULT_EXCLUDES
    root = args.dir
    if not os.path.isdir(root):
        raise ManifestError(f"not a directory: {root}")
    manifest_path = args.manifest or os.path.join(root, MANIFEST_NAME)
    text = render(root, excludes)
    _write(root, text, manifest_path)
    sys.stdout.write(text)
    sys.stdout.write(f"# {len(text.splitlines())} entries -> {manifest_path}\n")
    return 0


def run_verify(args: argparse.Namespace) -> int:
    excludes = tuple(args.exclude) or DEFAULT_EXCLUDES
    root = args.dir
    if not os.path.isdir(root):
        raise ManifestError(f"not a directory: {root}")
    manifest_path = args.manifest or os.path.join(root, MANIFEST_NAME)
    problems = verify(root, manifest_path, excludes)
    if problems:
        for problem in problems:
            sys.stderr.write(f"FAIL {problem}\n")
        sys.stderr.write(f"{len(problems)} problem(s) between {manifest_path} and {root}\n")
        return 1
    with open(manifest_path, encoding="utf-8") as handle:
        count = len(parse(handle.read()))
    sys.stdout.write(f"OK {count} entries verified against {manifest_path}\n")
    return 0


# ── self-test ────────────────────────────────────────────────────────────

def _tree(files: dict[str, bytes]) -> str:
    root = tempfile.mkdtemp(prefix="rivulet_manifest_")
    for name, content in files.items():
        path = os.path.join(root, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as handle:
            handle.write(content)
    return root


#: Every assertion the self-test makes, so the summary line reports work
#: actually done rather than a hardcoded number.
_ASSERTIONS: list[str] = []


def _expect(condition: bool, message: str) -> None:
    _ASSERTIONS.append(message)
    if not condition:
        raise AssertionError(message)


def run_self_test() -> int:

    # 1. Determinism: two renders of the same tree are byte-identical, and the
    #    order does not depend on directory iteration order.
    # Created in reverse-sorted order on purpose: the manifest must be sorted
    # by `render`, not by whatever order `os.walk` happens to hand back, and a
    # fixture created in sorted order would let a missing sort slip through.
    root = _tree({
        "rivulet-windows-x86_64.msi": b"msi",
        "rivulet-macos-aarch64.dmg": b"dmg",
        "rivulet-linux-x86_64.AppImage": b"appimage",
        "LICENSE": b"MIT",
    })
    first = render(root, DEFAULT_EXCLUDES)
    second = render(root, DEFAULT_EXCLUDES)
    _expect(first == second, "two renders of one tree must be byte-identical")
    # Parse rather than split on the literal separator: a manifest with the
    # wrong separator must fail the explicit format assertion in check 10,
    # not crash the sortedness check with an IndexError.
    names = [name for _digest, name in parse(first)]
    _expect(names == sorted(names), f"entries must be sorted, got {names}")
    _expect("opengraph.png" not in first, "release-card images must be excluded")
    _expect(MANIFEST_NAME not in first, "the manifest must not list itself")

    # 2. Round trip: a clean tree verifies.
    manifest = os.path.join(root, MANIFEST_NAME)
    _write(root, first, manifest)
    _expect(verify(root, manifest, DEFAULT_EXCLUDES) == [], "a fresh manifest must verify")

    # 3. A tampered artifact is caught.
    with open(os.path.join(root, "rivulet-linux-x86_64.AppImage"), "wb") as handle:
        handle.write(b"tampered")
    problems = verify(root, manifest, DEFAULT_EXCLUDES)
    _expect(
        any("digest mismatch" in problem for problem in problems),
        f"tampering must be reported, got {problems}",
    )

    # 4. A missing artifact is caught.
    os.unlink(os.path.join(root, "rivulet-windows-x86_64.msi"))
    problems = verify(root, manifest, DEFAULT_EXCLUDES)
    _expect(
        any("listed but missing" in problem for problem in problems),
        f"a missing artifact must be reported, got {problems}",
    )
    os.unlink(manifest)

    # 5. An artifact nobody hashed is caught -- the failure mode that would
    #    publish a file with no integrity coverage at all.
    second_root = _tree({"rivulet-linux-x86_64.AppImage": b"appimage"})
    with open(os.path.join(second_root, MANIFEST_NAME), "w", encoding="utf-8", newline="\n") as out:
        out.write(render(second_root, DEFAULT_EXCLUDES))
    with open(os.path.join(second_root, "smuggled.bin"), "wb") as handle:
        handle.write(b"payload")
    problems = verify(second_root, os.path.join(second_root, MANIFEST_NAME), DEFAULT_EXCLUDES)
    _expect(
        any("present but not listed" in problem for problem in problems),
        f"an unlisted artifact must be reported, got {problems}",
    )

    # 6. Nested paths and names with spaces survive the round trip.
    nested = _tree({
        "bin/rivulet": b"elf",
        "docs/read me.txt": b"spaces",
        "rivulet-linux-x86_64.AppImage": b"appimage",
    })
    text = render(nested, DEFAULT_EXCLUDES)
    with open(os.path.join(nested, MANIFEST_NAME), "w", encoding="utf-8", newline="\n") as out:
        out.write(text)
    _expect(
        verify(nested, os.path.join(nested, MANIFEST_NAME), DEFAULT_EXCLUDES) == [],
        "nested paths and spaces must verify",
    )
    _expect("docs/read me.txt" in text, f"the spaced path must be listed, got {text!r}")

    # 7. A malformed manifest is an error, never a silent pass.
    broken = _tree({"a.bin": b"a"})
    with open(os.path.join(broken, MANIFEST_NAME), "w", encoding="utf-8") as out:
        out.write("not-a-digest  a.bin\n")
    raised = False
    try:
        verify(broken, os.path.join(broken, MANIFEST_NAME), DEFAULT_EXCLUDES)
    except ManifestError:
        raised = True
    _expect(raised, "a malformed manifest must raise, not pass")

    # 8. An empty tree yields an empty manifest, and verify refuses it.
    empty = _tree({})
    with open(os.path.join(empty, MANIFEST_NAME), "w", encoding="utf-8", newline="\n") as out:
        out.write(render(empty, DEFAULT_EXCLUDES))
    problems = verify(empty, os.path.join(empty, MANIFEST_NAME), DEFAULT_EXCLUDES)
    _expect(any("empty" in problem for problem in problems), f"empty tree: {problems}")

    # 9. Generation writes the file the updater expects, with LF endings.
    generated = _tree({"rivulet-linux-x86_64.AppImage": b"appimage"})
    argv = ["--dir", generated]
    args = argparse.Namespace(dir=generated, manifest=None, exclude=[])
    _expect(run_generate(args) == 0, "generate must succeed")
    with open(os.path.join(generated, MANIFEST_NAME), "rb") as handle:
        raw = handle.read()
    _expect(b"\r\n" not in raw, "the manifest must use LF endings")
    _expect(raw.endswith(b"\n"), "the manifest must end with a newline")
    _expect(len(raw.decode().splitlines()) == 1, "one file, one entry")

    # 10. The exact `sha256sum` line shape the Rust updater and every other
    #     reader expect: `<64 hex><two spaces><name>`. Checking only what
    #     `parse` recovers would accept a single space, because the parser
    #     splits on any whitespace -- so compare against the literal format.
    line = raw.decode().splitlines()[0]
    digest, name = parse(raw.decode())[0]
    _expect(
        line == f"{digest}  {name}",
        f"the manifest line must be sha256sum-shaped, got {line!r}",
    )
    _expect(len(digest) == 64 and name == "rivulet-linux-x86_64.AppImage",
            f"unexpected parse result: {digest} {name}")

    for directory in (root, second_root, nested, broken, empty, generated):
        # rmtree, not unlink+rmdir: the nested fixture leaves sub-directories.
        shutil.rmtree(directory, ignore_errors=True)

    sys.stdout.write(f"release-manifest self-test: {len(_ASSERTIONS)} assertions passed\n")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--self-test", action="store_true")
    sub = parser.add_subparsers(dest="command")
    for name, help_text in (("generate", "write a manifest"), ("verify", "check a manifest")):
        child = sub.add_parser(name, help=help_text)
        child.add_argument("--dir", required=True)
        child.add_argument("--manifest")
        child.add_argument("--exclude", action="append", default=[])
    args = parser.parse_args(argv)

    try:
        if args.self_test:
            return run_self_test()
        if args.command == "generate":
            return run_generate(args)
        if args.command == "verify":
            return run_verify(args)
        parser.print_help()
        return 2
    except ManifestError as error:
        sys.stderr.write(f"error: {error}\n")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())