#!/usr/bin/env python3
"""Verify nightly.yml's Linux apt package lists stay in lockstep with ci.yml.

The nightly build broke for over a week because `libasound2-dev` was added to
ci.yml's Linux dependency step (when the audio crates joined the workspace)
but never mirrored into nightly.yml — nothing compared the two lists. This
guard parses the `sudo apt-get install` steps of both workflows and asserts:

1. The `lints` jobs' "Install GStreamer dependencies for clippy" steps must
   install the *exact same* package set in both workflows. Clippy resolves
   the same crate graph in both jobs, so any difference is drift.
2. The `build_and_test` jobs' "Install Linux dependencies (if applicable)"
   steps must install the same package set, *except* for packages listed in
   CI_ONLY_BUILD_PACKAGES below: test-only tools (currently the Xvfb/xdotool
   set used by ci.yml's Linux game-capture verification step) that a nightly
   build without that step legitimately does not need.

A new system dependency added to one workflow without the other therefore
fails this check (and the pre-push fast-guard stage) instead of the next
2 a.m. nightly run.

Exit codes: ``0`` = all compared lists match, ``1`` = drift detected
(mismatches printed), ``2`` = a workflow or an expected step could not be
parsed.

Usage:
    scripts/check-apt-parity.py [--json] [--self-test]
"""

import argparse
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CI_YML = REPO_ROOT / ".github" / "workflows" / "ci.yml"
NIGHTLY_YML = REPO_ROOT / ".github" / "workflows" / "nightly.yml"

# Anchors: the first line of the dependency step and the line that terminates
# the `run:` block. The terminator must be the *next* step heading in both
# workflows; a refactor that renames either anchor fails parsing (exit 2),
# which is the intended loud outcome.
ANCHORS = {
    "lints": {
        "step": "Install GStreamer dependencies for clippy",
        "terminator": "- name:",
        "message": "clippy dependency step",
    },
    "build": {
        "step": "Install Linux dependencies (if applicable)",
        "terminator": "- name:",
        "message": "Linux build dependency step",
    },
}

# Packages that only ci.yml's build_and_test job needs for its extra Linux
# verification steps (the Xvfb/xdotool game-capture enumeration test) and
# that nightly.yml's build without those steps may omit. Everything else
# must be identical between the two lists. Keep this set minimal: adding a
# package here means *documenting* why nightly does not need it.
CI_ONLY_BUILD_PACKAGES = frozenset({"xdotool", "xvfb", "x11-apps", "x11-utils"})

INSTALL_RE = re.compile(
    r"apt-get\s+(?:-o\s+\S+\s+)*install(?:\s+-\S+)*\s+\S*[\\\n]", re.MULTILINE
)


def parse_error(workflow, role, message):
    """Print a parse failure with repo-relative context and return exit code 2."""
    rel = workflow.relative_to(REPO_ROOT)
    print(f"ERROR: could not parse {rel} {role}: {message}", file=sys.stderr)
    return 2


def step_slice(text, step_name, terminator):
    """Return the text from ``step_name`` up to the next ``terminator`` line,
    or None if the anchor is missing."""
    idx = text.find(step_name)
    if idx < 0:
        return None
    end = text.find(terminator, idx + len(step_name))
    if end < 0:
        return None
    return text[idx:end]


def extract_packages(block):
    """Extract the package set of one apt-get install block.

    Handles backslash-newline continuations and the leading ``-y`` /
    ``--no-install-recommends`` flags. Package names are validated so a
    parser regression surfaces as a parse error, not as a bogus comparison.
    """
    match = INSTALL_RE.search(block)
    if not match:
        return None
    tail = block[match.end():]
    # Cut at any line that is no longer part of the install command (blank
    # line, non-continuation command, or an unindented YAML key).
    lines = []
    for line in tail.splitlines():
        if not line.strip() or not (line.startswith(" ") or line.endswith("\\")):
            if lines and not line.strip().endswith("\\") and not line.startswith(" "):
                break
            if not line.strip():
                break
        lines.append(line)
        if not line.rstrip().endswith("\\"):
            break
    packages = set()
    for line in lines:
        for token in line.replace("\\", " ").split():
            if token in {"-y", "--no-install-recommends", "sudo", "apt-get"}:
                continue
            if not re.fullmatch(r"[a-z0-9][a-z0-9+.-]*", token):
                return None
            packages.add(token)
    return packages or None


def collect(workflow, role):
    """Parse both dependency steps of one workflow; return a dict of sets."""
    text = workflow.read_text(encoding="utf-8")
    result = {}
    for key, anchor in ANCHORS.items():
        block = step_slice(text, anchor["step"], anchor["terminator"])
        if block is None:
            return None, parse_error(
                workflow,
                anchor["message"],
                f"step '{anchor['step']}' (or its terminator) not found",
            )
        packages = extract_packages(block)
        if packages is None:
            return None, parse_error(
                workflow, anchor["message"], "no apt-get install packages parsed"
            )
        result[key] = packages
    return result, None


def compare(ci, nightly):
    """Return the list of drift findings between the two parsed workflows."""
    findings = []

    def diff(where, a, b, allowed):
        only_a = sorted(a - b - allowed)
        only_b = sorted(b - a - allowed)
        if only_a:
            findings.append(f"{where}: only in ci.yml, missing from nightly.yml: {', '.join(only_a)}")
        if only_b:
            findings.append(f"{where}: only in nightly.yml, missing from ci.yml: {', '.join(only_b)}")

    if ci["lints"] != nightly["lints"]:
        diff("lints clippy dependency step", ci["lints"], nightly["lints"], frozenset())
    if ci["build"] - CI_ONLY_BUILD_PACKAGES != nightly["build"] - CI_ONLY_BUILD_PACKAGES:
        diff(
            "Linux build dependency step (excluding CI-only tools "
            + ", ".join(sorted(CI_ONLY_BUILD_PACKAGES)) + ")",
            ci["build"],
            nightly["build"],
            CI_ONLY_BUILD_PACKAGES,
        )
    return findings


def self_test():
    """Guard the extraction logic against silent regressions."""
    sample = """\
      - name: Install Linux dependencies (if applicable)
        if: runner.os == 'Linux'
        run: |
          sudo apt-get update -y
          sudo apt-get install -y --no-install-recommends \\
            build-essential \\
            pkg-config \\
            libasound2-dev \\
            xdotool \\
            xvfb

      - name: Next step
        run: echo done
"""
    block = step_slice(sample, "Install Linux dependencies (if applicable)", "- name:")
    assert block is not None, "step_slice must find the anchor"
    assert step_slice(sample, "Not a real step", "- name:") is None, \
        "step_slice must return None for a missing anchor"
    packages = extract_packages(block)
    assert packages == {
        "build-essential", "pkg-config", "libasound2-dev", "xdotool", "xvfb",
    }, f"extraction drifted: {packages}"
    assert extract_packages("sudo apt-get update -y\n") is None, \
        "extract_packages must reject blocks without an install command"

    ci = {"lints": {"a", "b"}, "build": {"a", "b", "xdotool", "xvfb"}}
    same = {"lints": {"a", "b"}, "build": {"a", "b"}}
    assert compare(ci, same) == [], "CI-only tools must not be reported"
    drift = {"lints": {"a", "b"}, "build": {"a", "b", "newpkg", "xdotool"}}
    findings = compare(drift, same)
    assert len(findings) == 1 and "newpkg" in findings[0], \
        f"a genuinely missing package must be reported: {findings}"
    lint_drift = {"lints": {"a", "b", "c"}, "build": {"a", "b", "xdotool"}}
    findings = compare(lint_drift, same)
    assert len(findings) == 1 and "lints clippy dependency step" in findings[0], \
        f"lints drift must be reported even when both sides install a/b: {findings}"
    print("check-apt-parity: self-test passed")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="emit findings as JSON")
    parser.add_argument(
        "--self-test", action="store_true", help="run the internal extraction tests"
    )
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    ci, err = collect(CI_YML, "ci.yml")
    if err is not None:
        return err
    nightly, err = collect(NIGHTLY_YML, "nightly.yml")
    if err is not None:
        return err

    findings = compare(ci, nightly)
    if args.json:
        print(json.dumps({"findings": findings}, indent=2))
    if findings:
        if not args.json:
            print(
                "apt-parity drift between .github/workflows/ci.yml and "
                ".github/workflows/nightly.yml:"
            )
            for finding in findings:
                print(f"  - {finding}")
            print(
                "Every Linux system dependency added to one workflow must be "
                "mirrored in the other (see scripts/check-apt-parity.py)."
            )
        return 1
    if not args.json:
        print("check-apt-parity: ci.yml and nightly.yml Linux package lists match")
    return 0


if __name__ == "__main__":
    sys.exit(main())
