#!/usr/bin/env python3
"""Verify the Linux apt package lists stay in lockstep across ALL workflows.

The nightly build broke for over a week because `libasound2-dev` was added to
ci.yml's Linux dependency step (when the audio crates joined the workspace)
but never mirrored into nightly.yml — nothing compared the two lists. This
guard parses every workflow's `apt-get install` steps and asserts that
equivalent jobs install the same package set (see GROUPS):

1. The `lints` jobs' clippy dependency steps (ci.yml, nightly.yml) must
   install the *exact same* package set — clippy resolves the same crate
   graph in both jobs, so any difference is drift.
2. The Linux *build* dependency steps (ci.yml/nightly.yml `build_and_test`,
   build-package.yml release build, security.yml CodeQL build) must install
   the same set, *except* for packages in CI_ONLY_BUILD_PACKAGES: test-only
   tools (the Xvfb/xdotool set used by ci.yml's game-capture verification
   step) that a pure build legitimately does not need.
3. The fuzz build dependency steps (ci.yml fuzz smoke, fuzz-deep.yml) must
   install the same pkg-config/GStreamer/PipeWire subset.
4. The flatpak tooling steps (flatpak-build.yml, distribution-readiness.yml)
   must install the same flatpak/flatpak-builder set.

A new system dependency added to one workflow without its equivalents
therefore fails this check (and the pre-push fast-guard stage) instead of
the next 2 a.m. nightly run or the next CodeQL rebuild.

Exit codes: ``0`` = all compared lists match, ``1`` = drift detected
(mismatches printed), ``2`` = a workflow or an expected step could not be
parsed.

The workflows are parsed structurally (PyYAML): steps are located by their
structured `name:` field inside the real `jobs:`/`steps:` mapping, not by
text-slicing between step headings, so reordering or re-indenting steps
cannot silently change what is compared.

Usage:
    scripts/check-apt-parity.py [--json] [--self-test]
"""

import argparse
import json
import re
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - surfaced loudly by collect()
    yaml = None

REPO_ROOT = Path(__file__).resolve().parent.parent
CI_YML = REPO_ROOT / ".github" / "workflows" / "ci.yml"
NIGHTLY_YML = REPO_ROOT / ".github" / "workflows" / "nightly.yml"

# Anchors: the `name:` of the dependency step inside the structured workflow
# YAML. Steps are located by parsing the real `jobs:`/`steps:` structure
# (PyYAML), not by slicing raw text between step headings — so reordering,
# re-indenting or renaming a step fails loudly (exit 2) instead of silently
# comparing the wrong block.
ANCHORS = {
    "lints": {
        "step": "Install GStreamer dependencies for clippy",
        "message": "clippy dependency step",
    },
    "build": {
        "step": "Install Linux dependencies (if applicable)",
        "message": "Linux build dependency step",
    },
}

# ci.yml additionally hosts the fuzz-smoke step whose apt subset must stay
# in lockstep with fuzz-deep.yml's dedicated step.
CI_EXTRA_ANCHORS = {
    "fuzz": {
        "step": "Run fuzz smoke (256 runs per target)",
        "message": "fuzz smoke dependency step",
    },
}

# Additional checked workflows, keyed by workflow file stem. Each entry maps
# a checker role to the anchor of the apt-get install step to parse. Every
# role participates in exactly one equivalence group (see GROUPS in
# compare()); a group's members must install the same package set.
EXTRA_WORKFLOWS = {
    "build-package": {
        "build": {
            "step": "Install Linux release dependencies",
            "message": "Linux release dependency step",
        },
    },
    "security": {
        "build": {
            "step": "Install GStreamer development dependencies",
            "message": "CodeQL build dependency step",
        },
    },
    "fuzz-deep": {
        "fuzz": {
            "step": "Install GStreamer/PipeWire build dependencies",
            "message": "deep-fuzz dependency step",
        },
    },
    "flatpak-build": {
        "flatpak_tools": {
            "step": "Install flatpak and flatpak-builder",
            "message": "flatpak tooling step",
        },
        "flatpak_builder_build": {
            "step": "Build the pinned recent flatpak-builder (screenshot mirroring)",
            "message": "flatpak-builder build step",
        },
    },
    "distribution-readiness": {
        "flatpak_tools": {
            "step": "Install flatpak and flatpak-builder",
            "message": "flatpak tooling step",
        },
    },
}

# Packages that only ci.yml's build_and_test job needs for its extra Linux
# verification steps (the Xvfb/xdotool game-capture enumeration test) and
# that nightly.yml's build without those steps may omit. Everything else
# must be identical between the two lists. Keep this set minimal: adding a
# package here means *documenting* why nightly does not need it.
CI_ONLY_BUILD_PACKAGES = frozenset({"xdotool", "xvfb", "x11-apps", "x11-utils"})

INSTALL_RE = re.compile(
    r"apt-get\s+(?:-o\s+\S+\s+)*install(?:\s+-\S+)*(?:\s+|(?=[\\]))",
    re.MULTILINE,
)

# Packages that are never counted: the apt-get flags and wrapper words.
NON_PACKAGE_TOKENS = {"-y", "--no-install-recommends", "sudo", "apt-get"}


def parse_error(workflow, role, message):
    """Print a parse failure with repo-relative context and return exit code 2."""
    rel = workflow.relative_to(REPO_ROOT)
    print(f"ERROR: could not parse {rel} {role}: {message}", file=sys.stderr)
    return 2


def workflow_steps(data):
    """Yield every step mapping of one parsed workflow document."""
    jobs = (data or {}).get("jobs") or {}
    for job in jobs.values():
        if not isinstance(job, dict):
            continue
        for step in job.get("steps") or []:
            if isinstance(step, dict):
                yield step


def extract_packages(run):
    """Extract the package set of one `run:` block containing apt-get install.

    The block is the structured YAML value (a single string, continuations
    already folded). Package names are validated so a parser regression
    surfaces as a parse error, not as a bogus comparison.
    """
    match = INSTALL_RE.search(run)
    if not match:
        return None
    tail = run[match.end():]
    packages = set()
    # The install command ends at the first line without a trailing
    # backslash continuation; later lines of the same `run:` block belong
    # to other commands and must not leak into the package set.
    for line in tail.splitlines():
        for token in line.replace("\\", " ").split():
            if token in NON_PACKAGE_TOKENS:
                continue
            if not re.fullmatch(r"[a-z0-9][a-z0-9+.-]*", token):
                return None
            packages.add(token)
        if not line.rstrip().endswith("\\"):
            break
    return packages or None


def collect_from_data(data, anchors, workflow):
    """Parse every anchored dependency step from one parsed document; return a
    dict of sets keyed by checker role."""
    result = {}
    for key, anchor in anchors.items():
        step_name = anchor["step"]
        run = next(
            (
                step.get("run")
                for step in workflow_steps(data)
                if step.get("name") == step_name and isinstance(step.get("run"), str)
            ),
            None,
        )
        if run is None:
            return None, parse_error(
                workflow,
                anchor["message"],
                f"step '{step_name}' (or its run block) not found",
            )
        packages = extract_packages(run)
        if packages is None:
            return None, parse_error(
                workflow, anchor["message"], "no apt-get install packages parsed"
            )
        result[key] = packages
    return result, None


def collect(workflow, anchors):
    """Parse every anchored dependency step of one workflow file; return a
    dict of sets keyed by checker role."""
    if yaml is None:
        print("ERROR: PyYAML is required for structured workflow parsing", file=sys.stderr)
        return None, 2
    try:
        data = yaml.safe_load(workflow.read_text(encoding="utf-8"))
    except yaml.YAMLError as exc:
        return None, parse_error(workflow, "workflow", f"invalid YAML: {exc}")
    return collect_from_data(data, anchors, workflow)


# Equivalence groups: all members of a group must install the same package
# set. `allowed` exempts packages whose absence is documented (see
# CI_ONLY_BUILD_PACKAGES).
GROUPS = [
    {
        "name": "lints clippy dependency step",
        "members": [("ci.yml", "lints"), ("nightly.yml", "lints")],
        "allowed": frozenset(),
    },
    {
        "name": "Linux build dependency step",
        "members": [
            ("ci.yml", "build"),
            ("nightly.yml", "build"),
            ("build-package.yml", "build"),
            ("security.yml", "build"),
        ],
        "allowed": CI_ONLY_BUILD_PACKAGES,
    },
    {
        "name": "fuzz build dependency step (pkg-config/GStreamer/PipeWire subset)",
        "members": [("ci.yml", "fuzz"), ("fuzz-deep.yml", "fuzz")],
        "allowed": frozenset(),
    },
    {
        "name": "flatpak tooling step (flatpak + flatpak-builder)",
        "members": [
            ("flatpak-build.yml", "flatpak_tools"),
            ("distribution-readiness.yml", "flatpak_tools"),
        ],
        "allowed": frozenset(),
    },
]


def compare(parsed):
    """Return the list of drift findings across all equivalence groups.

    ``parsed`` maps workflow file name -> {role -> package set}.
    """
    findings = []
    for group in GROUPS:
        sets = []
        for file_name, role in group["members"]:
            sets.append((file_name, parsed[file_name][role]))
        reference_file, reference = sets[0]
        reference -= group["allowed"]
        for file_name, packages in sets[1:]:
            packages -= group["allowed"]
            if packages == reference:
                continue
            missing_here = sorted(reference - packages)
            missing_reference = sorted(packages - reference)
            if missing_here:
                findings.append(
                    f"{group['name']}: {file_name} is missing {', '.join(missing_here)} "
                    f"(present in {reference_file})"
                )
            if missing_reference:
                findings.append(
                    f"{group['name']}: {file_name} installs extra packages "
                    f"{', '.join(missing_reference)} not present in {reference_file}"
                )
    return findings


def self_test():
    """Guard the extraction logic against silent regressions."""

    def load(sample):
        return yaml.safe_load(sample)

    def find_run(data, step_name):
        return next(
            (
                step.get("run")
                for step in workflow_steps(data)
                if step.get("name") == step_name and isinstance(step.get("run"), str)
            ),
            None,
        )

    sample = """\
jobs:
  build:
    steps:
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
    data = load(sample)
    run = find_run(data, "Install Linux dependencies (if applicable)")
    assert run is not None, "structured parse must find the anchor step"
    assert find_run(data, "Not a real step") is None, \
        "a missing anchor must yield no run block"
    packages = extract_packages(run)
    assert packages == {
        "build-essential", "pkg-config", "libasound2-dev", "xdotool", "xvfb",
    }, f"extraction drifted: {packages}"
    assert extract_packages("sudo apt-get update -y\n") is None, \
        "extract_packages must reject blocks without an install command"

    # Later commands in the same run block must not leak tokens (the
    # flatpak workflows install inline and then configure remotes).
    inline = extract_packages(
        "sudo apt-get install -y flatpak flatpak-builder\n"
        "sudo flatpak remote-add --if-not-exists flathub example"
    )
    assert inline == {"flatpak", "flatpak-builder"}, \
        f"inline install form must parse: {inline}"

    # Two packages per line with inline continuations (build-package form):
    # the trailing backslash must not leak an 'n' token (regression guard).
    multi = extract_packages(
        "sudo apt-get install -y --no-install-recommends \\\n"
        "            build-essential pkg-config \\\n"
        "            libxcb1-dev libxcb-render0-dev \\\n"
        "            libasound2-dev"
    )
    assert multi == {
        "build-essential", "pkg-config", "libxcb1-dev",
        "libxcb-render0-dev", "libasound2-dev",
    }, f"multi-per-line form must parse cleanly: {multi}"
    assert "n" not in multi, "the continuation backslash must not leak tokens"

    # Structured collection end-to-end: the anchor must match by step name
    # (the structured `name:` field), not by text position.
    parsed, err = collect_from_data(
        load(sample),
        {"build": {"step": "Install Linux dependencies (if applicable)", "message": "x"}},
        CI_YML,
    )
    assert err is None and parsed == {
        "build": {"build-essential", "pkg-config", "libasound2-dev", "xdotool", "xvfb"},
    }, f"structured collection drifted: {parsed}, {err}"
    missing, err = collect_from_data(
        load(sample),
        {"build": {"step": "No such step anywhere", "message": "x"}},
        CI_YML,
    )
    assert missing is None and err == 2, "a missing anchor must fail loudly (exit 2)"

    # Group comparison on the new parsed-dict model.
    def wf(lints, build, fuzz, tools):
        return {
            "ci.yml": {"lints": set(lints), "build": set(build), "fuzz": set(fuzz)},
            "nightly.yml": {"lints": set(lints), "build": set(build)},
            "build-package.yml": {"build": set(build)},
            "security.yml": {"build": set(build)},
            "fuzz-deep.yml": {"fuzz": set(fuzz)},
            "flatpak-build.yml": {
                "flatpak_tools": set(tools),
                "flatpak_builder_build": set(tools),
            },
            "distribution-readiness.yml": {"flatpak_tools": set(tools)},
        }

    base = wf({"a"}, {"a"}, {"p"}, {"flatpak"})
    assert compare(base) == [], f"consistent sets must pass: {compare(base)}"

    drifted = wf({"a"}, {"a"}, {"p"}, {"flatpak"})
    drifted["fuzz-deep.yml"]["fuzz"] = {"p", "libnew"}
    findings = compare(drifted)
    assert len(findings) == 1 and "libnew" in findings[0] and "fuzz" in findings[0], \
        f"fuzz-group drift must be reported: {findings}"

    # Real CI-only tools: ci.yml may install them without the others following.
    ci_only = wf({"a"}, {"a", "xdotool", "xvfb"}, {"p"}, {"flatpak"})
    findings = compare(ci_only)
    assert findings == [], \
        f"CI-only allowed packages must not be reported: {findings}"

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

    workflows = {"ci.yml": CI_YML, "nightly.yml": NIGHTLY_YML}
    for stem, anchors in EXTRA_WORKFLOWS.items():
        workflows[f"{stem}.yml"] = REPO_ROOT / ".github" / "workflows" / f"{stem}.yml"

    parsed = {}
    for file_name, path in workflows.items():
        if file_name == "ci.yml":
            anchors = {**ANCHORS, **CI_EXTRA_ANCHORS}
        elif file_name == "nightly.yml":
            anchors = ANCHORS
        else:
            anchors = EXTRA_WORKFLOWS[file_name.removesuffix(".yml")]
        data, err = collect(path, anchors)
        if err is not None:
            return err
        parsed[file_name] = data

    findings = compare(parsed)
    if args.json:
        print(json.dumps({"findings": findings}, indent=2))
    if findings:
        if not args.json:
            print("apt-parity drift across Linux dependency steps:")
            for finding in findings:
                print(f"  - {finding}")
            print(
                "Every Linux system dependency used by equivalent jobs must be "
                "mirrored across the workflows (see scripts/check-apt-parity.py)."
            )
        return 1
    if not args.json:
        print("check-apt-parity: Linux package lists match across all workflows")
    return 0


if __name__ == "__main__":
    sys.exit(main())
