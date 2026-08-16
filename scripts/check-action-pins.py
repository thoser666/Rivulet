#!/usr/bin/env python3
"""Check that every pinned action SHA matches the latest upstream release/branch.

Detects stale pins outside Dependabot's weekly schedule. Two distinct cases are
reported separately:

- **outdated within the major line** — the pinned major has a newer
  patch/minor release we have not taken (actionable, drop-in update).
- **newer major available** — a higher major version exists (may be an
  intentional pin and needs a decision).

Exit codes: non-zero when any pin is *outdated within its major* or cannot be
resolved. A *newer major* is reported but does not fail unless you pass
`--fail-on-major`.

Usage:
    scripts/check-action-pins.py [--fail-on-major]
"""

import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from action_pins import parse_workflows  # noqa: E402

SEMVER_RE = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)$")


def ls_remote(repo, *refs, tags=False):
    """Run `git ls-remote` against the repo and return its output lines."""
    cmd = ["git", "ls-remote"]
    if tags:
        cmd.append("--tags")
    cmd.append(f"https://github.com/{repo}.git")
    cmd.extend(refs)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError(f"git ls-remote failed for {repo}: {proc.stderr.strip()}")
    return proc.stdout.splitlines()


def peeled_sha(lines):
    """Return the dereferenced (^{}) commit SHA from ls-remote output, if any."""
    peeled = None
    plain = None
    for line in lines:
        sha, _, ref = line.partition("\t")
        if ref.endswith("^{}"):
            peeled = sha
        elif plain is None:
            plain = sha
    return peeled or plain


def resolve_ref(repo, refspec):
    sha = peeled_sha(ls_remote(repo, refspec))
    if not sha:
        raise RuntimeError(f"{repo}: ref {refspec!r} not found")
    return sha


def stable_tags(repo):
    """Return ``[(version_key, tag_name, commit_sha)]`` for stable semver tags."""
    tags = {}
    for line in ls_remote(repo, tags=True):
        sha, _, ref = line.partition("\t")
        name = ref.removeprefix("refs/tags/")
        if name.endswith("^{}"):
            name = name[:-3]
            tags[name] = sha  # dereferenced commit SHA wins
        else:
            tags.setdefault(name, sha)

    result = []
    for name, sha in tags.items():
        match = SEMVER_RE.match(name)
        if match:
            key = (int(match.group(1)), int(match.group(2)), int(match.group(3)))
            result.append((key, name, sha))
    return result


def check_action(action, sha, version):
    """Return ``(statuses, message)``.

    ``statuses`` is a subset of ``{"ok", "outdated", "major", "error"}``; an
    action can be both ``outdated`` (within its major) and ``major`` (a newer
    major exists).
    """
    ref_kind = None
    try:
        resolve_ref(action, f"refs/tags/{version}")
        ref_kind = "tag"
    except RuntimeError:
        pass
    if ref_kind is None:
        try:
            resolve_ref(action, f"refs/heads/{version}")
            ref_kind = "branch"
        except RuntimeError:
            pass
    if ref_kind is None:
        return {"error"}, f"{action}@{version}: ref does not resolve to a tag or branch"

    if ref_kind == "branch":
        tip = resolve_ref(action, f"refs/heads/{version}")
        if tip == sha:
            return {"ok"}, f"{action}@{version} is current (branch tip {sha})"
        return {"outdated"}, f"{action}@{version}: branch tip is {tip}, pinned {sha}"

    tags = stable_tags(action)
    if not tags:
        return {"error"}, f"{action}: no stable semver tags found"
    pinned = SEMVER_RE.match(version)
    if not pinned:
        return {"error"}, f"{action}: pinned version {version!r} is not semver"
    pinned_key = (int(pinned.group(1)), int(pinned.group(2)), int(pinned.group(3)))

    in_major = [tag for tag in tags if tag[0][0] == pinned_key[0]]
    if not in_major:
        return {"error"}, f"{action}: no tags for major v{pinned_key[0]}"
    major_key, major_name, major_sha = max(in_major)
    overall_key, overall_name, overall_sha = max(tags)

    statuses = {"ok"}
    parts = []
    if major_sha != sha:
        statuses = {"outdated"}
        parts.append(
            f"outdated in v{pinned_key[0]}: pinned {version} ({sha}), "
            f"latest {major_name} ({major_sha})"
        )
    else:
        parts.append(f"current in v{pinned_key[0]} ({major_name})")
    if overall_key[0] > pinned_key[0]:
        statuses.add("major")
        parts.append(f"newer major available: {overall_name} ({overall_sha})")

    return statuses, f"{action}: " + "; ".join(parts)


def main():
    pins = parse_workflows()
    if not pins:
        sys.exit("no third-party action pins found in .github/workflows")
    fail_on_major = "--fail-on-major" in sys.argv[1:]

    outdated = []
    major = []
    errors = []
    for action in sorted(pins):
        sha, version, _ = pins[action]
        try:
            statuses, message = check_action(action, sha, version)
        except RuntimeError as exc:
            print(f"ERROR {exc}", file=sys.stderr)
            errors.append(action)
            continue
        print(message)
        if "outdated" in statuses:
            outdated.append(action)
        if "major" in statuses:
            major.append(action)
        if "error" in statuses:
            errors.append(action)

    print(
        f"\nSummary: {len(outdated)} outdated within major, "
        f"{len(major)} with newer major available, {len(errors)} errors",
        file=sys.stderr,
    )

    failed = bool(errors or outdated or (major if fail_on_major else []))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
