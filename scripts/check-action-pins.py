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
`--fail-on-major`. *Rolling branch drift* is reported but does not fail unless
you pass `--fail-on-branch-drift`.

Rolling branch pins
-------------------
Some actions are pinned to a moving branch rather than a release tag
(`dtolnay/rust-toolchain` -> `stable`). For those, "the branch tip is newer
than our SHA" is not staleness in the same sense as a missed release: the SHA
still fixes the action *code* that runs, and the compiler version comes from
`rust-toolchain.toml`, not from the branch. Any upstream push would therefore
redden the nightly without anything being broken, and Dependabot cannot cover
the pin because a branch has no release to propose. It is reported separately
as `branch-drift` and stays informational unless `--fail-on-branch-drift` is
passed.

Output modes (the exit code is the same regardless of mode):

- (default)      one human-readable line per action, plus a summary on stderr.
- `--json`       a single JSON document on stdout (machine-readable).
- `--comment`    a compact Markdown notification suitable for a GitHub issue,
                 PR comment, or the GitHub Actions step summary.

Usage:
    scripts/check-action-pins.py [--fail-on-major] [--fail-on-branch-drift]
                                  [--json | --comment]
"""

import json
import re
import subprocess
import sys
from pathlib import Path

# Emoji/Unicode in the --comment output must survive non-UTF-8 consoles (e.g.
# cp1252 on Windows); reconfiguring stdout/stderr to UTF-8 is a no-op on Linux.
for _stream in (sys.stdout, sys.stderr):
    _reconfigure = getattr(_stream, "reconfigure", None)
    if _reconfigure is not None:
        try:
            _reconfigure(encoding="utf-8")
        except (ValueError, OSError):
            pass

sys.path.insert(0, str(Path(__file__).resolve().parent))

from action_pins import parse_workflows  # noqa: E402

SEMVER_RE = re.compile(r"^(\d+)\.(\d+)(?:\.(\d+))?$")


def semver_key(version):
    """Parse ``v?X.Y.Z`` or ``v?X.Y`` into a comparable 3-tuple.

    Some actions tag two-segment releases (e.g. SignPath's ``v2.3``); treating
    the missing patch as 0 keeps such pins comparable instead of erroring.
    """
    match = SEMVER_RE.match(version.removeprefix("v"))
    if not match:
        return None
    major, minor = int(match.group(1)), int(match.group(2))
    patch = int(match.group(3)) if match.group(3) else 0
    return (major, minor, patch)


def ls_remote(repo, *refs, tags=False):
    """Run `git ls-remote` against the repo and return its output lines."""
    cmd = ["git", "ls-remote"]
    if tags:
        cmd.append("--tags")
    cmd.append(f"https://github.com/{repo}.git")
    cmd.extend(refs)
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"git ls-remote timed out for {repo}") from exc
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
        key = semver_key(name)
        if key is not None:
            result.append((key, name, sha))
    return result


def action_repo(action):
    """Return the GitHub ``owner/repo`` for a (possibly compound) action ref.

    Compound actions are referenced as ``owner/repo/path/to/action`` while the
    upstream tags/branches live on the ``owner/repo`` repository itself, so the
    sub-path must be stripped before resolving refs.
    """
    return "/".join(action.split("/")[:2])


def check_action(action, sha, version):
    """Return a JSON-serializable dict describing the pin's status.

    ``statuses`` is a list drawn from ``{"ok", "outdated", "major", "error",
    "branch-drift"}``; an action can be both ``outdated`` (within its major) and
    ``major`` (a newer major exists).
    """
    result = {
        "action": action,
        "pinned_version": version,
        "pinned_sha": sha,
    }

    repo = action_repo(action)
    ref_kind = None
    try:
        resolve_ref(repo, f"refs/tags/{version}")
        ref_kind = "tag"
    except RuntimeError:
        pass
    if ref_kind is None:
        try:
            resolve_ref(repo, f"refs/heads/{version}")
            ref_kind = "branch"
        except RuntimeError:
            pass
    if ref_kind is None:
        result.update(
            kind="unknown",
            statuses=["error"],
            message=f"{action}@{version}: ref does not resolve to a tag or branch",
        )
        return result

    if ref_kind == "branch":
        tip = resolve_ref(repo, f"refs/heads/{version}")
        result.update(kind="branch", branch_tip=tip)
        if tip == sha:
            result.update(
                statuses=["ok"],
                message=f"{action}@{version} is current (branch tip {sha})",
            )
        else:
            # Not an error: a branch pin is a moving target by construction, and
            # the pinned SHA still fixes the action code that runs. Reported so
            # the bump is visible, fatal only behind --fail-on-branch-drift.
            result.update(
                statuses=["branch-drift"],
                message=(
                    f"{action}@{version}: branch tip is {tip}, pinned {sha} "
                    "(rolling branch, informational)"
                ),
            )
        return result

    tags = stable_tags(repo)
    if not tags:
        result.update(kind="tag", statuses=["error"], message=f"{action}: no stable semver tags found")
        return result
    pinned_key = semver_key(version)
    if pinned_key is None:
        result.update(
            kind="tag",
            statuses=["error"],
            message=f"{action}: pinned version {version!r} is not semver",
        )
        return result

    in_major = [tag for tag in tags if tag[0][0] == pinned_key[0]]
    if not in_major:
        result.update(
            kind="tag",
            statuses=["error"],
            message=f"{action}: no tags for major v{pinned_key[0]}",
        )
        return result
    major_key, major_name, major_sha = max(in_major)
    overall_key, overall_name, overall_sha = max(tags)

    statuses = ["ok"]
    parts = []
    if major_sha != sha:
        statuses = ["outdated"]
        parts.append(
            f"outdated in v{pinned_key[0]}: pinned {version} ({sha}), "
            f"latest {major_name} ({major_sha})"
        )
    else:
        parts.append(f"current in v{pinned_key[0]} ({major_name})")
    if overall_key[0] > pinned_key[0]:
        statuses.append("major")
        parts.append(f"newer major available: {overall_name} ({overall_sha})")

    result.update(
        kind="tag",
        statuses=statuses,
        message=f"{action}: " + "; ".join(parts),
        latest_in_major={"version": major_name, "sha": major_sha},
        latest_overall={"version": overall_name, "sha": overall_sha},
    )
    return result


def summarize(results):
    return {
        "total": len(results),
        "current": sum(1 for r in results if r["statuses"] == ["ok"]),
        "outdated": sum(1 for r in results if "outdated" in r["statuses"]),
        "major": sum(1 for r in results if "major" in r["statuses"]),
        "errors": sum(1 for r in results if "error" in r["statuses"]),
        "branch_drift": sum(1 for r in results if "branch-drift" in r["statuses"]),
    }


def fatal_count(summary, fail_on_major, fail_on_branch_drift):
    """Return how many findings must fail the run.

    Shared by the comment renderer and the exit code so the step summary can
    never claim green while the job still fails (or the reverse). Rolling
    branch drift only counts when the caller opted in via
    ``--fail-on-branch-drift``.
    """
    total = summary["errors"] + summary["outdated"]
    if fail_on_major:
        total += summary["major"]
    if fail_on_branch_drift:
        total += summary["branch_drift"]
    return total


def short_status(result):
    """Compact human label for the comment table."""
    parts = []
    if "error" in result["statuses"]:
        parts.append("unresolvable")
    if "outdated" in result["statuses"] and result.get("latest_in_major"):
        parts.append(f"outdated (latest `{result['latest_in_major']['version']}`)")
    if "branch-drift" in result["statuses"]:
        tip = result.get("branch_tip", "")
        # Whether drift is fatal is decided by --fail-on-branch-drift and
        # stated in the headline, so the cell stays a neutral fact.
        parts.append(f"branch drift (tip `{tip[:7]}`)")
    if "major" in result["statuses"] and result.get("latest_overall"):
        parts.append(f"newer major `{result['latest_overall']['version']}`")
    return ", ".join(parts)


def render_comment(results, summary, fail_on_major=False, fail_on_branch_drift=False):
    """Return a compact Markdown notification for an issue/PR/step summary."""
    fatal = fatal_count(summary, fail_on_major, fail_on_branch_drift)
    drift = summary["branch_drift"]

    if fatal == 0:
        lines = [f"✅ All {summary['total']} action pins are current."]
        if drift:
            # Green, but the bump stays visible: a rolling branch pin that
            # moved is not a missed release.
            lines += [
                "",
                f"ℹ️ {drift} rolling-branch pin(s) moved upstream "
                "(informational — the pinned SHA still fixes the action code):",
                "",
                "| Action | Pinned | Status |",
                "| --- | --- | --- |",
            ]
            for result in results:
                if "branch-drift" in result["statuses"]:
                    lines.append(
                        f"| `{result['action']}` | `{result['pinned_version']}` | "
                        f"{short_status(result)} |"
                    )
        return "\n".join(lines) + "\n"

    lines = ["## ⚠️ Action pins need attention", ""]
    if drift:
        role = (
            "fatal here, because `--fail-on-branch-drift` is set"
            if fail_on_branch_drift
            else "informational unless `--fail-on-branch-drift` is set"
        )
        lines += [
            f"ℹ️ {drift} rolling-branch pin(s) moved upstream — {role}.",
            "",
        ]
    lines += [
        (
            f"**{summary['outdated']} outdated within major · "
            f"{summary['major']} newer major · {summary['errors']} errors"
            + (f" · {drift} rolling-branch drift**" if drift else "**")
            + f" (of {summary['total']})"
        ),
        "",
        "| Action | Pinned | Status |",
        "| --- | --- | --- |",
    ]
    for result in results:
        if result["statuses"] != ["ok"]:
            lines.append(
                f"| `{result['action']}` | `{result['pinned_version']}` | "
                f"{short_status(result)} |"
            )
    return "\n".join(lines) + "\n"


def main():
    args = sys.argv[1:]
    if "--help" in args or "-h" in args:
        print(__doc__.strip())
        return 0

    pins = parse_workflows()
    if not pins:
        sys.exit("no third-party action pins found in .github/workflows")

    fail_on_major = "--fail-on-major" in args
    fail_on_branch_drift = "--fail-on-branch-drift" in args
    as_json = "--json" in args
    as_comment = "--comment" in args or "--github-comment" in args

    results = []
    for action in sorted(pins):
        sha, version, _ = pins[action]
        try:
            results.append(check_action(action, sha, version))
        except RuntimeError as exc:
            results.append(
                {
                    "action": action,
                    "pinned_version": version,
                    "pinned_sha": sha,
                    "kind": "unknown",
                    "statuses": ["error"],
                    "message": f"ERROR {exc}",
                }
            )

    summary = summarize(results)
    outdated = summary["outdated"]
    major = summary["major"]
    errored = summary["errors"]
    failed = bool(fatal_count(summary, fail_on_major, fail_on_branch_drift))

    if as_json:
        print(
            json.dumps(
                {
                    "ok": not failed,
                    "fail_on_major": fail_on_major,
                    "fail_on_branch_drift": fail_on_branch_drift,
                    "summary": summary,
                    "actions": results,
                },
                indent=2,
            )
        )
    elif as_comment:
        print(render_comment(results, summary, fail_on_major, fail_on_branch_drift))
    else:
        for result in results:
            print(result["message"])
        print(
            f"\nSummary: {outdated} outdated within major, "
            f"{major} with newer major available, {errored} errors, "
            f"{summary['branch_drift']} rolling-branch drift",
            file=sys.stderr,
        )

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
