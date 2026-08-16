#!/usr/bin/env python3
"""Check that every pinned action SHA matches the latest upstream release/branch.

Detects stale pins outside Dependabot's weekly schedule: for tag-pinned actions
it resolves the latest stable semver tag and compares its commit SHA to the
pinned one; for branch-pinned actions (e.g. dtolnay/rust-toolchain pinned to
`stable`) it compares against the branch tip. Exits non-zero if any pin is
outdated or cannot be resolved.

Usage:
    scripts/check-action-pins.py
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


def latest_stable_tag(repo):
    """Return ``(tag_name, commit_sha)`` of the highest stable semver tag."""
    tags = {}
    for line in ls_remote(repo, tags=True):
        sha, _, ref = line.partition("\t")
        name = ref.removeprefix("refs/tags/")
        if name.endswith("^{}"):
            name = name[:-3]
            tags[name] = sha  # dereferenced commit SHA wins
        else:
            tags.setdefault(name, sha)

    versions = []
    for name, sha in tags.items():
        match = SEMVER_RE.match(name)
        if match:
            key = (int(match.group(1)), int(match.group(2)), int(match.group(3)))
            versions.append((key, name, sha))
    if not versions:
        raise RuntimeError(f"{repo}: no stable semver tags found")
    _, name, sha = max(versions)
    return name, sha


def check_action(action, sha, version):
    """Return ``(status, message)`` where status is 'ok', 'stale' or 'error'."""
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
        return "error", f"{action}@{version}: ref does not resolve to a tag or branch"

    if ref_kind == "branch":
        tip = resolve_ref(action, f"refs/heads/{version}")
        if tip == sha:
            return "ok", f"{action}@{version} is current (branch tip {sha})"
        return "stale", f"{action}@{version}: branch tip is {tip}, pinned {sha}"

    latest_name, latest_sha = latest_stable_tag(action)
    if latest_sha == sha:
        return "ok", f"{action} is current ({latest_name} = {sha})"
    return "stale", f"{action}: pinned {version} ({sha}); latest {latest_name} ({latest_sha})"


def main():
    pins = parse_workflows()
    if not pins:
        sys.exit("no third-party action pins found in .github/workflows")

    stale = []
    for action in sorted(pins):
        sha, version, _ = pins[action]
        try:
            status, message = check_action(action, sha, version)
        except RuntimeError as exc:
            print(f"ERROR {exc}", file=sys.stderr)
            stale.append(action)
            continue
        print(message)
        if status != "ok":
            stale.append(action)

    if stale:
        print(
            f"\n{len(stale)} stale/unresolvable pin(s): {', '.join(stale)}",
            file=sys.stderr,
        )
        return 1
    print("\nAll action pins are current.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
