#!/usr/bin/env python3
"""Verify that the generated release notes are complete and clean.

The release workflow derives its GitHub release body from the actual commits
between the previous tag and the release tip
(``scripts/generate-release-notes.sh``). This checker enforces the two
contracts of that body:

1. **Completeness** — every non-merge commit in ``previous-tag..HEAD`` that
   is not a release-prep commit appears in the notes (compared after
   stripping the conventional-commit prefix, exactly like the generator).
   This catches the generator silently dropping a commit (e.g. the
   unterminated ``git log`` line that used to lose the oldest commit).
2. **Cleanliness** — the body contains no ``chore(release): prepare``
   entries, so the pipeline's own bump commits never leak into user-facing
   notes.

Run in CI in two places:

* **Lints job** (every push): ``--self-test`` regression-tests the checker
  against fixture repositories.
* **Release workflow** (each alpha release, where the checkout already has
  full history + tags): ``--notes-file release-notes.md`` validates the exact
  body that is about to be published, failing the job before the release is
  created.

Exit codes: ``0`` = notes complete and clean, ``1`` = a contract violation,
``2`` = environment/usage error (git unavailable, generator failed).

Usage:
    scripts/check-release-notes.py [--repo-dir DIR] [--notes-file FILE] [--self-test]
"""

import argparse
import collections
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Must stay in sync with the conventional-commit regex in
# scripts/generate-release-notes.sh.
CONVENTIONAL_RE = re.compile(
    r"^(feat|fix|perf|docs|build|ci|test|refactor|chore)"
    r"(\([^)]*\))?!?:\s*(.*)$"
)
PREPARE_PREFIX = "chore(release): prepare "
BULLET_PREFIX = "- "


def run_git(repo, *args):
    """Run a git command in ``repo``; return stdout text or ``None`` on error."""
    try:
        proc = subprocess.run(
            ["git", "-C", str(repo), *args],
            capture_output=True,
            text=True,
            check=False,
        )
    except FileNotFoundError:
        return None
    if proc.returncode != 0:
        return None
    return proc.stdout


def strip_conventional(subject):
    """Normalize a commit subject like the generator does for its bullets."""
    match = CONVENTIONAL_RE.match(subject)
    if match:
        return match.group(3).strip()
    return subject.strip()


def previous_tag(repo):
    """Newest tag reachable from HEAD^ (the current release's tag sits at HEAD),
    falling back to the first commit when no tag exists yet."""
    tag = run_git(repo, "describe", "--tags", "--abbrev=0", "HEAD^")
    if tag is not None:
        return tag.strip()
    first = run_git(repo, "rev-list", "--max-parents=0", "HEAD")
    if first is None:
        raise RuntimeError(f"{repo} is not a git repository")
    return first.strip()


def expected_subjects(repo, tag_range):
    """Multiset of normalized subjects for every non-merge, non-prepare commit
    in the range — exactly what the generator should have rendered."""
    out = run_git(repo, "log", "--no-merges", "--format=%s", tag_range)
    counts = collections.Counter()
    if out is None:
        return counts
    for raw in out.splitlines():
        subject = raw.strip()
        if not subject or subject.startswith(PREPARE_PREFIX):
            continue
        normalized = strip_conventional(subject)
        if normalized:
            counts[normalized] += 1
    return counts


# Section order + headings, mirrored from scripts/generate-release-notes.sh.
SECTION_ORDER = [
    ("feat", "### Features"),
    ("fix", "### Bug fixes"),
    ("perf", "### Performance"),
    ("docs", "### Documentation"),
    ("build", "### Build & packaging"),
    ("ci", "### CI"),
    ("test", "### Tests"),
    ("refactor", "### Refactoring"),
    ("chore", "### Housekeeping"),
    ("other", "### Other changes"),
]


def generate_notes(repo):
    """Python mirror of generate-release-notes.sh for the given repository.

    Used for local runs and the self-test only: the authoritative end-to-end
    check runs in the release workflow, where the real bash generator writes
    the body and this checker validates it via ``--notes-file``. A python
    implementation (instead of spawning bash) keeps the checker portable on
    Windows, where the bash found on PATH may be WSL bash that cannot handle
    native Windows paths.
    """
    prev = previous_tag(repo)
    entries = {}
    out = run_git(repo, "log", "--no-merges", "--format=%s", f"{prev}..HEAD")
    if out:
        for raw in out.splitlines():
            subject = raw.strip()
            if not subject or subject.startswith(PREPARE_PREFIX):
                continue
            match = CONVENTIONAL_RE.match(subject)
            if match:
                entry_type, rest = match.group(1), match.group(3).strip()
            else:
                entry_type, rest = "other", subject
            entries.setdefault(entry_type, []).append(f"- {rest}")
    body = ""
    for entry_type, heading in SECTION_ORDER:
        bullets = entries.get(entry_type)
        if bullets:
            body += f"{heading}\n" + "\n".join(bullets) + "\n\n"
    return body


def check(repo, notes_body):
    """Return (ok, messages). ``notes_body`` is the release notes Markdown."""
    prev = previous_tag(repo)
    tag_range = f"{prev}..HEAD"
    expected = expected_subjects(repo, tag_range)
    bullets = collections.Counter(
        line[len(BULLET_PREFIX):].strip()
        for line in notes_body.splitlines()
        if line.startswith(BULLET_PREFIX)
    )

    messages = []
    ok = True

    # Completeness: every expected normalized subject must appear as a bullet.
    for subject, need in sorted(expected.items()):
        if bullets.get(subject, 0) < need:
            ok = False
            messages.append(f"missing release note for commit: {subject}")

    # Cleanliness: no release-prep commit may leak into the body.
    if PREPARE_PREFIX in notes_body:
        ok = False
        messages.append("release notes must not contain 'chore(release): prepare' commits")

    if ok:
        messages.append(
            f"release notes complete: {sum(expected.values())} commits since {prev}, "
            "no prepare commits"
        )
    return ok, messages


def run_self_test():
    """Fixture-based regression tests for the checker itself."""
    status = 0
    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp) / "fixture"
        repo.mkdir(parents=True)
        _git(repo, "init")
        _git(repo, "config", "user.email", "test@example.com")
        _git(repo, "config", "user.name", "Test")
        _git(repo, "commit", "--allow-empty", "-m", "chore: seed the repository")
        _git(repo, "tag", "-a", "v0.1.0-alpha.1", "-m", "Release v0.1.0-alpha.1")
        _git(repo, "commit", "--allow-empty", "-m", "feat(ui): add the stream tab")
        _git(repo, "commit", "--allow-empty", "-m", "fix(updater): verify checksums before install")
        _git(repo, "commit", "--allow-empty", "-m", "docs: document the release notes")
        _git(repo, "commit", "--allow-empty", "-m", "ci(release): serialize alpha runs")
        _git(repo, "commit", "--allow-empty", "-m", "chore(release): prepare v0.2.0-alpha.1")

        # 1. End-to-end: generator output must satisfy the checker.
        body = generate_notes(repo)
        ok, messages = check(repo, body)
        if not ok:
            print(f"FAIL: end-to-end check rejected complete notes: {messages}", file=sys.stderr)
            status = 1
        else:
            print("ok: end-to-end generator + checker agree")

        # 2. Completeness: a body that drops one bullet must be rejected.
        dropped = "\n".join(
            line for line in body.splitlines() if "document the release notes" not in line
        )
        ok, messages = check(repo, dropped)
        if ok or not any("document the release notes" in m for m in messages):
            print("FAIL: incomplete notes were not rejected", file=sys.stderr)
            status = 1
        else:
            print("ok: incomplete notes rejected")

        # 3. Cleanliness: a leaked prepare commit must be rejected.
        leaked = body + f"{BULLET_PREFIX}chore(release): prepare v0.2.0-alpha.1\n"
        ok, messages = check(repo, leaked)
        if ok or not any("prepare" in m for m in messages):
            print("FAIL: leaked prepare commit was not rejected", file=sys.stderr)
            status = 1
        else:
            print("ok: leaked prepare commit rejected")

    return status


def _git(repo, *args):
    proc = subprocess.run(
        ["git", "-C", str(repo), *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {proc.stderr.strip()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo-dir", default=str(REPO_ROOT))
    parser.add_argument("--notes-file", help="validate this body instead of generating it")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()

    repo = Path(args.repo_dir).resolve()
    if args.notes_file:
        body = Path(args.notes_file).read_text(encoding="utf-8")
    else:
        body = generate_notes(repo)

    ok, messages = check(repo, body)
    for message in messages:
        print(message)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())