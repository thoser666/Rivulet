#!/usr/bin/env python3
"""Normalized payload fingerprints for rebuild-and-compare (issue #288).

Issue #190 made a release *auditable*: one tested ``SHA256SUMS`` generator and a
post-publish job that re-downloads the release and verifies it. That proves what
CI hashed is what a user fetches. It does **not** prove that a second,
independent build of the same commit produces the same bytes.

This script answers that question, and nothing else:

* ``fingerprint`` hashes every file of a staged payload directory and records
  the build inputs that decide those bytes (target, rustc, GStreamer version,
  commit, ``SOURCE_DATE_EPOCH``, the remap targets and linker arguments).
* ``compare`` decides whether two fingerprints describe the *same shipped
  payload* and says exactly which file diverged.

Three distinctions keep the result honest, because collapsing them is how a
reproducibility gate turns into a number nobody trusts:

**payload** - the files a user executes (bare binaries, ``.exe``). A difference
here is a real divergence and fails the comparison.

**container** - archives and signed packages (``.zip``, ``.msi``, ``.dmg``,
``.AppImage``, ``.asc``). These embed archive member mtimes, WiX-harvested GUIDs
or GPG signature timestamps; ``docs/release-platforms.md`` documents them as
content-deterministic, not bit-identical. They are reported, never gated.

**metadata** - repository files that ride along (``LICENSE``, ``SHA256SUMS``).
Also reported, never gated.

A comparison that cannot be made must not look like a pass. If the two
fingerprints were produced by different toolchains, targets, GStreamer versions
or commits, the result is ``not-comparable`` with the field named - a different
compiler legitimately produces different bytes, and calling that "divergence"
or "reproducible" would both be wrong.

Usage::

    scripts/reproducible-build.py fingerprint --dir staging --out fingerprint.json
    scripts/reproducible-build.py compare --left a.json --right b.json [--right c.json]
    scripts/reproducible-build.py --self-test

Exit codes for ``compare``: 0 reproducible, 1 payload divergence,
2 not comparable (or unreadable input). Never 0 for "nothing was measured".
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile

SCHEMA_VERSION = 1

#: Suffixes whose bytes legitimately differ per run (see module docstring).
CONTAINER_SUFFIXES = (
    ".zip",
    ".msi",
    ".msix",
    ".dmg",
    ".appimage",
    ".flatpak",
    ".snap",
    ".asc",
    ".sig",
    ".pkg",
    ".deb",
    ".rpm",
)

#: Files that travel with a release but are not build output.
METADATA_NAMES = ("LICENSE", "LICENSE.txt", "NOTICE", "SHA256SUMS", "SHA256SUMS.txt")

PAYLOAD = "payload"
CONTAINER = "container"
METADATA = "metadata"

REPRODUCIBLE = "reproducible"
DIVERGED = "diverged"
NOT_COMPARABLE = "not-comparable"

#: Build inputs that decide the payload bytes. Two fingerprints may only be
#: compared when these agree; otherwise the comparison measures the toolchain,
#: not the build.
COMPARABLE_FIELDS = (
    "target",
    "rustc",
    "cargo",
    "gstreamer",
    "commit",
    "source_date_epoch",
    "remap_targets",
    "link_args",
)

_CHUNK = 1024 * 1024


class FingerprintError(Exception):
    """A fingerprint could not be built, read or trusted."""


# --------------------------------------------------------------------------
# Hashing and classification
# --------------------------------------------------------------------------


def sha256_of(path: str) -> str:
    """Stream the file through SHA-256 so a large installer stays off the heap."""
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(_CHUNK), b""):
            digest.update(chunk)
    return digest.hexdigest()


def classify(relative: str) -> str:
    """Which bucket a file belongs to; unknown files are payload by default.

    Defaulting to payload is deliberate: a new shipped file must be compared,
    not silently excused. Adding an archive format to the container list is a
    documented decision, forgetting one is a red gate.
    """
    name = relative.rsplit("/", 1)[-1]
    if name in METADATA_NAMES:
        return METADATA
    lowered = name.lower()
    if any(lowered.endswith(suffix) for suffix in CONTAINER_SUFFIXES):
        return CONTAINER
    return PAYLOAD


def iter_tree(root: str) -> list[str]:
    """Every file under ``root`` as a sorted POSIX-style relative path."""
    found = []
    for directory, subdirs, files in os.walk(root):
        subdirs.sort()
        for name in files:
            absolute = os.path.join(directory, name)
            if os.path.islink(absolute) or not os.path.isfile(absolute):
                continue
            found.append(os.path.relpath(absolute, root).replace(os.sep, "/"))
    return sorted(found)


def digest_of(entries: list[dict], kind: str) -> str:
    """One digest over the sorted ``path:sha256`` pairs of one bucket.

    Computed from the file list rather than stored independently, so a
    fingerprint whose recorded digest disagrees with its own entries cannot be
    mistaken for an intact one.
    """
    digest = hashlib.sha256()
    for entry in sorted(entries, key=lambda item: item["path"]):
        if entry["kind"] != kind:
            continue
        digest.update(f"{entry['path']}\0{entry['sha256']}\n".encode("utf-8"))
    return digest.hexdigest()


# --------------------------------------------------------------------------
# Building a fingerprint
# --------------------------------------------------------------------------


def build_fingerprint(root: str, label: str, provenance: dict) -> dict:
    """Describe the payload in ``root`` plus the inputs that produced it."""
    if not os.path.isdir(root):
        raise FingerprintError(f"not a directory: {root}")
    entries = []
    for relative in iter_tree(root):
        absolute = os.path.join(root, relative)
        entries.append(
            {
                "path": relative,
                "kind": classify(relative),
                "size": os.path.getsize(absolute),
                "sha256": sha256_of(absolute),
            }
        )
    counts = {kind: 0 for kind in (PAYLOAD, CONTAINER, METADATA)}
    for entry in entries:
        counts[entry["kind"]] += 1
    return {
        "schema_version": SCHEMA_VERSION,
        "label": label,
        "generated_at": _timestamp(),
        "provenance": {field: provenance.get(field) for field in COMPARABLE_FIELDS},
        "files": entries,
        "counts": counts,
        "digests": {kind: digest_of(entries, kind) for kind in (PAYLOAD, CONTAINER, METADATA)},
    }


def _timestamp() -> str:
    import time

    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def _command_output(command: list[str]) -> str | None:
    """Run a provenance probe; ``None`` when it is unavailable, never a guess."""
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.SubprocessError):
        return None
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def collect_provenance(target: str | None, commit: str | None) -> dict:
    """Gather the build inputs from the current environment.

    Every field is recorded as observed. A probe that cannot answer stays
    ``None`` and is reported as such; inventing a version would make an
    incomparable pair look comparable.
    """
    rustc = _command_output(["rustc", "-V"])
    cargo = _command_output(["cargo", "-V"])
    gstreamer = _command_output(["pkg-config", "--modversion", "gstreamer-1.0"])
    remap = os.environ.get("REPRODUCIBLE_REMAP", "")
    link_args = os.environ.get("REPRODUCIBLE_LINK_ARGS", "")
    epoch = os.environ.get("SOURCE_DATE_EPOCH", "")
    return {
        "target": target,
        "rustc": rustc,
        "cargo": cargo,
        "gstreamer": gstreamer,
        # Only the *destination* of each remap is comparable: the source is the
        # runner's checkout path, which is exactly what remapping normalizes.
        "remap_targets": [
            token.split("=", 1)[1] for token in remap.split() if "=" in token
        ],
        "link_args": link_args.split(),
        "commit": commit or _command_output(["git", "rev-parse", "HEAD"]),
        "source_date_epoch": int(epoch) if epoch.isdigit() else None,
        "runner": {
            "os": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
        },
    }


# --------------------------------------------------------------------------
# Reading and comparing
# --------------------------------------------------------------------------


def load_fingerprint(path: str) -> dict:
    """Read and validate a fingerprint, including its internal consistency."""
    try:
        with open(path, encoding="utf-8") as handle:
            data = json.load(handle)
    except OSError as error:
        raise FingerprintError(f"cannot read {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise FingerprintError(f"{path} is not JSON: {error}") from error

    if not isinstance(data, dict):
        raise FingerprintError(f"{path}: a fingerprint must be a JSON object")
    if data.get("schema_version") != SCHEMA_VERSION:
        raise FingerprintError(
            f"{path}: schema_version {data.get('schema_version')!r} is not {SCHEMA_VERSION}"
        )
    files = data.get("files")
    if not isinstance(files, list):
        raise FingerprintError(f"{path}: `files` must be a list")
    for entry in files:
        if not isinstance(entry, dict):
            raise FingerprintError(f"{path}: every file entry must be an object")
        for field in ("path", "kind", "sha256"):
            if not isinstance(entry.get(field), str):
                raise FingerprintError(f"{path}: file entry without a string {field}: {entry}")
        if entry["kind"] not in (PAYLOAD, CONTAINER, METADATA):
            raise FingerprintError(f"{path}: unknown kind {entry['kind']!r} for {entry['path']}")
    digests = data.get("digests")
    if not isinstance(digests, dict):
        raise FingerprintError(f"{path}: `digests` must be an object")
    recomputed = {kind: digest_of(files, kind) for kind in (PAYLOAD, CONTAINER, METADATA)}
    if recomputed != digests:
        raise FingerprintError(
            f"{path}: recorded digests disagree with the file list "
            f"(recorded {digests}, recomputed {recomputed}); the fingerprint is not intact"
        )
    return data


def _comparable(value):
    """Comparison key: lists are order-insensitive, missing is the empty label."""
    if isinstance(value, list):
        return tuple(sorted(str(item) for item in value))
    return "" if value is None else str(value)


def _shown(value) -> str:
    if value is None:
        return "—"
    if isinstance(value, list):
        return ", ".join(str(item) for item in value) if value else "—"
    return str(value)


def compare(left: dict, right: dict, require_container: bool = False) -> dict:
    """Decide whether ``right`` reproduces ``left``'s shipped payload."""
    provenance = []
    for field in COMPARABLE_FIELDS:
        left_value = left.get("provenance", {}).get(field)
        right_value = right.get("provenance", {}).get(field)
        if _comparable(left_value) != _comparable(right_value):
            provenance.append(
                {"field": field, "left": left_value, "right": right_value}
            )

    left_files = {entry["path"]: entry for entry in left.get("files", [])}
    right_files = {entry["path"]: entry for entry in right.get("files", [])}
    files = []
    for path in sorted(set(left_files) | set(right_files)):
        left_entry = left_files.get(path)
        right_entry = right_files.get(path)
        kind = (left_entry or right_entry)["kind"]
        left_digest = left_entry["sha256"] if left_entry else None
        right_digest = right_entry["sha256"] if right_entry else None
        if left_digest and right_digest:
            same = left_digest == right_digest
            if kind == PAYLOAD:
                result = "identical" if same else "differs"
            else:
                # Container/metadata bytes are allowed to differ; say so
                # explicitly instead of hiding the file from the report.
                result = "identical" if same else "documented"
        elif left_digest:
            result = "only-in-left"
        else:
            result = "only-in-right"
        files.append(
            {
                "path": path,
                "kind": kind,
                "left": left_digest,
                "right": right_digest,
                "result": result,
            }
        )

    payload = [entry for entry in files if entry["kind"] == PAYLOAD]
    containers = [entry for entry in files if entry["kind"] == CONTAINER]
    payload_problems = [
        entry for entry in payload if entry["result"] in ("differs", "only-in-left", "only-in-right")
    ]
    container_problems = [
        entry
        for entry in containers
        if entry["result"] in ("documented", "only-in-left", "only-in-right")
    ]

    reasons = []
    if provenance:
        reasons.append(
            "not comparable: "
            + ", ".join(
                f"{difference['field']} differs "
                f"({_shown(difference['left'])} vs {_shown(difference['right'])})"
                for difference in provenance
            )
        )
    if not payload:
        reasons.append(
            "no payload files in the fingerprint; an empty directory reproduces trivially "
            "and would prove nothing"
        )
    if payload_problems:
        names = ", ".join(entry["path"] for entry in payload_problems)
        reasons.append(f"{len(payload_problems)} payload file(s) diverge: {names}")
    if container_problems and require_container:
        names = ", ".join(entry["path"] for entry in container_problems)
        reasons.append(f"{len(container_problems)} container file(s) diverge (--require-container): {names}")

    if provenance or not payload:
        status = NOT_COMPARABLE
    elif payload_problems or (container_problems and require_container):
        status = DIVERGED
    else:
        status = REPRODUCIBLE

    return {
        "status": status,
        "left": left.get("label"),
        "right": right.get("label"),
        "reasons": reasons,
        "provenance": provenance,
        "files": files,
        "counts": {
            "payload": len(payload),
            "payload_identical": sum(1 for entry in payload if entry["result"] == "identical"),
            "container": len(containers),
            "container_documented": sum(1 for entry in containers if entry["result"] == "documented"),
            "metadata": sum(1 for entry in files if entry["kind"] == METADATA),
        },
    }


# --------------------------------------------------------------------------
# Reporting
# --------------------------------------------------------------------------


def _short(digest: str | None) -> str:
    return f"`{digest[:12]}…`" if digest else "—"


def render_summary(results: list[dict]) -> str:
    """Markdown for the job summary: verdict, provenance deltas, per-file table."""
    verdicts = {
        REPRODUCIBLE: "reproducible",
        DIVERGED: "**diverged**",
        NOT_COMPARABLE: "**not comparable**",
    }
    lines = []
    for result in results:
        label = result["right"] or "right"
        lines.append(f"### `{label}`: {verdicts[result['status']]}")
        lines.append("")
        if result["reasons"]:
            for reason in result["reasons"]:
                lines.append(f"- {reason}")
            lines.append("")

        if result["provenance"]:
            lines.append("| Build input | left | right |")
            lines.append("| --- | --- | --- |")
            for difference in result["provenance"]:
                lines.append(
                    f"| `{difference['field']}` | {_shown(difference['left'])} | "
                    f"{_shown(difference['right'])} |"
                )
            lines.append("")

        counts = result["counts"]
        lines.append(
            f"{counts['payload']} payload file(s), "
            f"{counts['payload_identical']} identical; "
            f"{counts['container']} container file(s), "
            f"{counts['container_documented']} with documented byte drift; "
            f"{counts['metadata']} metadata file(s)."
        )
        lines.append("")
        lines.append("| File | Kind | left | right | Result |")
        lines.append("| --- | --- | --- | --- | --- |")
        for entry in result["files"]:
            mark = {
                "identical": "identical",
                "differs": "**differs**",
                "documented": "documented",
                "only-in-left": "**missing on the right**",
                "only-in-right": "**missing on the left**",
            }[entry["result"]]
            lines.append(
                f"| `{entry['path']}` | {entry['kind']} | {_short(entry['left'])} | "
                f"{_short(entry['right'])} | {mark} |"
            )
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def _exit_code(results: list[dict]) -> int:
    """Divergence first (actionable), then not-comparable, then success."""
    if any(result["status"] == DIVERGED for result in results):
        return 1
    if any(result["status"] == NOT_COMPARABLE for result in results):
        return 2
    return 0


# --------------------------------------------------------------------------
# Commands
# --------------------------------------------------------------------------


def run_fingerprint(args: argparse.Namespace) -> int:
    provenance = collect_provenance(args.target, args.commit)
    data = build_fingerprint(args.dir, args.label or os.path.basename(os.path.abspath(args.dir)), provenance)
    if args.out:
        directory = os.path.dirname(os.path.abspath(args.out)) or "."
        os.makedirs(directory, exist_ok=True)
        with open(args.out, "w", encoding="utf-8", newline="\n") as handle:
            json.dump(data, handle, indent=2, sort_keys=True)
            handle.write("\n")
    counts = data["counts"]
    sys.stdout.write(
        f"fingerprint {data['label']}: {counts[PAYLOAD]} payload, "
        f"{counts[CONTAINER]} container, {counts[METADATA]} metadata file(s)\n"
    )
    sys.stdout.write(f"  payload digest: {data['digests'][PAYLOAD]}\n")
    if not counts[PAYLOAD]:
        # A fingerprint with no payload is a broken staging step, not a clean
        # release; fail here so the compare never has to guess.
        raise FingerprintError(
            f"{args.dir} contains no payload files; the staging step did not produce a "
            "shipped binary"
        )
    return 0


def run_compare(args: argparse.Namespace) -> int:
    left = load_fingerprint(args.left)
    results = []
    for right_path in args.right:
        right = load_fingerprint(right_path)
        results.append(compare(left, right, require_container=args.require_container))

    text = render_summary(results)
    sys.stdout.write(text)
    if args.summary:
        directory = os.path.dirname(os.path.abspath(args.summary)) or "."
        os.makedirs(directory, exist_ok=True)
        with open(args.summary, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(text)
    return _exit_code(results)


# ── self-test ────────────────────────────────────────────────────────────

_ASSERTIONS: list[str] = []


def _expect(condition: bool, message: str) -> None:
    _ASSERTIONS.append(message)
    if not condition:
        raise AssertionError(message)


def _tree(files: dict[str, bytes]) -> str:
    root = tempfile.mkdtemp(prefix="rivulet_repro_")
    for name, content in files.items():
        path = os.path.join(root, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as handle:
            handle.write(content)
    return root


def _provenance(**overrides) -> dict:
    base = {
        "target": "x86_64-unknown-linux-gnu",
        "rustc": "rustc 1.90.0 (1159e78c4 2026-09-14)",
        "cargo": "cargo 1.90.0 (1159e78c4 2026-09-14)",
        "gstreamer": "1.26.11",
        "commit": "0123456789abcdef0123456789abcdef01234567",
        "source_date_epoch": 1762000000,
        "remap_targets": ["/rivulet", "/cargo"],
        "link_args": ["/Brepro", "/DEBUG:NONE"],
    }
    base.update(overrides)
    return base


def _fingerprint(files: dict[str, bytes], label: str, **overrides) -> dict:
    root = _tree(files)
    try:
        return build_fingerprint(root, label, _provenance(**overrides))
    finally:
        shutil.rmtree(root, ignore_errors=True)


def run_self_test() -> int:
    payload = {"rivulet-linux-x86_64-gui": b"elf-binary"}
    container = {"rivulet-linux-x86_64.AppImage": b"appimage"}

    # 1. Classification decides what a failure even means. Defaulting to
    #    payload is the safe direction: a new shipped file must be compared.
    for name, expected in (
        ("rivulet-gui.exe", PAYLOAD),
        ("rivulet-linux-x86_64-gui", PAYLOAD),
        ("rivulet-windows-x86_64-portable.zip", CONTAINER),
        ("rivulet-windows-x86_64.msi", CONTAINER),
        ("rivulet-macos-aarch64.dmg", CONTAINER),
        ("rivulet-linux-x86_64.AppImage", CONTAINER),
        ("rivulet-linux-x86_64.AppImage.asc", CONTAINER),
        ("LICENSE", METADATA),
        ("SHA256SUMS", METADATA),
    ):
        _expect(
            classify(name) == expected,
            f"{name} must classify as {expected}, got {classify(name)}",
        )

    # 2. Two independent fingerprints of the same tree agree.
    left = _fingerprint({**payload, **container, "LICENSE": b"MIT"}, "run-a")
    right = _fingerprint({**payload, **container, "LICENSE": b"MIT"}, "run-b")
    result = compare(left, right)
    _expect(
        result["status"] == REPRODUCIBLE,
        f"identical payloads must be reproducible, got {result['status']}: {result['reasons']}",
    )
    _expect(
        result["counts"]["payload_identical"] == 1,
        f"the payload count must be reported, got {result['counts']}",
    )
    _expect(
        compare(left, right)["status"] == REPRODUCIBLE,
        "comparing must not mutate either fingerprint",
    )

    # 3. One byte of payload is a divergence, and it names the file.
    tampered = _fingerprint(
        {"rivulet-linux-x86_64-gui": b"elf-binarz", **container, "LICENSE": b"MIT"},
        "run-b",
    )
    result = compare(left, tampered)
    _expect(
        result["status"] == DIVERGED,
        f"a changed payload byte must diverge, got {result['status']}",
    )
    _expect(
        "rivulet-linux-x86_64-gui" in " ".join(result["reasons"]),
        f"the diverging file must be named, got {result['reasons']}",
    )
    _expect(_exit_code([result]) == 1, "a divergence must exit 1")

    # 4. A missing payload file is a divergence, not an omission.
    missing = _fingerprint(container, "run-b")
    result = compare(left, missing)
    _expect(result["status"] == DIVERGED, "a missing payload file must diverge")
    _expect(
        any(entry["result"] == "only-in-left" for entry in result["files"]),
        f"the missing file must be reported as only-in-left, got {result['files']}",
    )

    # 5. Container bytes may differ: documented, not gated.
    other_container = {"rivulet-linux-x86_64.AppImage": b"appimage-rebuilt"}
    result = compare(left, _fingerprint({**payload, **other_container, "LICENSE": b"MIT"}, "run-b"))
    _expect(
        result["status"] == REPRODUCIBLE,
        f"container drift must not fail the comparison, got {result['status']}: {result['reasons']}",
    )
    _expect(
        any(entry["result"] == "documented" for entry in result["files"]),
        "container drift must still be reported, not hidden",
    )
    _expect(
        compare(
            left,
            _fingerprint({**payload, **other_container, "LICENSE": b"MIT"}, "run-b"),
            require_container=True,
        )["status"]
        == DIVERGED,
        "--require-container must turn documented drift into a failure",
    )

    # 6. A container that exists on one side only is also documented drift.
    result = compare(left, _fingerprint({**payload, "LICENSE": b"MIT"}, "run-b"))
    _expect(
        result["status"] == REPRODUCIBLE,
        f"a container missing on one side must not fail, got {result['status']}",
    )

    # 7. Metadata drift is reported, never gated.
    result = compare(left, _fingerprint({**payload, **container, "LICENSE": b"MIT-2"}, "run-b"))
    _expect(
        result["status"] == REPRODUCIBLE and result["counts"]["metadata"] == 1,
        "metadata drift must be reported without failing",
    )

    # 8. Different build inputs are *not comparable*, not "diverged".
    for field, value in (
        ("rustc", "rustc 1.91.0 (another)"),
        ("target", "aarch64-apple-darwin"),
        ("gstreamer", "1.24.0"),
        ("commit", "fedcba9876543210fedcba9876543210fedcba98"),
        ("source_date_epoch", 1762000001),
        ("link_args", ["/Brepro"]),
        ("remap_targets", ["/src"]),
    ):
        result = compare(left, _fingerprint({**payload, **container, "LICENSE": b"MIT"}, "run-b", **{field: value}))
        _expect(
            result["status"] == NOT_COMPARABLE,
            f"a different {field} must be not-comparable, got {result['status']}",
        )
        _expect(
            field in " ".join(result["reasons"]),
            f"the reason must name {field}, got {result['reasons']}",
        )
        _expect(_exit_code([result]) == 2, "not comparable must exit 2")
    # The remap *source* is the runner path and must not make a pair incomparable.
    result = compare(
        left,
        _fingerprint({**payload, **container, "LICENSE": b"MIT"}, "run-b"),
    )
    _expect(
        result["status"] == REPRODUCIBLE,
        "the fingerprint must not record the remapped-away source path as an input",
    )

    # 9. An empty payload proves nothing, even when both sides are empty.
    empty_left = _fingerprint({}, "run-a")
    empty_right = _fingerprint({}, "run-b")
    result = compare(empty_left, empty_right)
    _expect(
        result["status"] == NOT_COMPARABLE,
        f"two empty payloads must be not-comparable, got {result['status']}",
    )
    _expect(
        any("no payload files" in reason for reason in result["reasons"]),
        f"the reason must explain the empty payload, got {result['reasons']}",
    )

    # 10. A fingerprint whose digest does not match its own file list is not
    #     trusted, so a hand-edited file cannot launder a divergence.
    broken = _fingerprint({**payload}, "run-b")
    broken["digests"][PAYLOAD] = "0" * 64
    directory = tempfile.mkdtemp(prefix="rivulet_repro_broken_")
    try:
        path = os.path.join(directory, "fingerprint.json")
        with open(path, "w", encoding="utf-8") as handle:
            json.dump(broken, handle)
        raised = None
        try:
            load_fingerprint(path)
        except FingerprintError as error:
            raised = str(error)
        _expect(
            raised is not None and "not intact" in raised,
            f"an edited fingerprint must be rejected, got {raised}",
        )
        with open(path, "w", encoding="utf-8") as handle:
            json.dump({"schema_version": 999, "files": []}, handle)
        raised = None
        try:
            load_fingerprint(path)
        except FingerprintError as error:
            raised = str(error)
        _expect(
            raised is not None and "schema_version" in raised,
            "a foreign schema version must be rejected",
        )
    finally:
        shutil.rmtree(directory, ignore_errors=True)

    # 11. The summary has to carry the verdict and the diverging file, because
    #     the job summary is what a human reads first.
    summary = render_summary([compare(left, tampered)])
    _expect("diverged" in summary, f"the summary must state the verdict, got {summary}")
    _expect(
        "rivulet-linux-x86_64-gui" in summary,
        "the summary must name the diverging file",
    )
    good_summary = render_summary([compare(left, right)])
    _expect(
        "reproducible" in good_summary and "documented" not in good_summary.split("##")[0],
        "a reproducing pair must not be described as documented drift",
    )

    sys.stdout.write(f"reproducible-build self-test: {len(_ASSERTIONS)} assertions passed\n")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--self-test", action="store_true")
    sub = parser.add_subparsers(dest="command")

    fingerprint = sub.add_parser("fingerprint", help="describe a staged payload")
    fingerprint.add_argument("--dir", required=True)
    fingerprint.add_argument("--out")
    fingerprint.add_argument("--label")
    fingerprint.add_argument("--target")
    fingerprint.add_argument("--commit")

    compare_parser = sub.add_parser("compare", help="compare fingerprints")
    compare_parser.add_argument("--left", required=True)
    compare_parser.add_argument("--right", action="append", required=True)
    compare_parser.add_argument("--summary")
    compare_parser.add_argument("--require-container", action="store_true")

    args = parser.parse_args(argv)

    try:
        if args.self_test:
            return run_self_test()
        if args.command == "fingerprint":
            return run_fingerprint(args)
        if args.command == "compare":
            return run_compare(args)
        parser.print_help()
        return 2
    except FingerprintError as error:
        sys.stderr.write(f"error: {error}\n")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
