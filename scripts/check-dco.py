#!/usr/bin/env python3
"""Check that commits carry a Developer Certificate of Origin sign-off.

Rivulet uses the DCO (https://developercertificate.org): every commit that
lands through a pull request must carry a ``Signed-off-by`` trailer matching
the commit author, which asserts the author is legally allowed to make the
contribution under the project's MIT license.

Usage:
    python3 scripts/check-dco.py                 # check branch vs origin/develop
    python3 scripts/check-dco.py --base develop  # check branch vs local develop
    python3 scripts/check-dco.py --self-test     # run the built-in test fixture

Exit code 0 = every commit in the range is signed off; 1 = violations found;
2 = usage/self-test error.
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile

SIGNOFF_RE = re.compile(
    r"^Signed-off-by:\s*(.+?)\s*<([^<>]+)>$", re.MULTILINE
)


def git(*args: str) -> str:
    """Run git and return stdout, raising SystemExit on failure."""
    proc = subprocess.run(
        ["git", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        sys.stderr.write(
            f"check-dco: git {' '.join(args)} failed:\n{proc.stderr}"
        )
        raise SystemExit(2)
    return proc.stdout


def commit_author(sha: str) -> str:
    """Return 'Name <email>' of the author of `sha`."""
    return git("log", "-1", "--format=%an <%ae>", sha).strip()


def commit_body(sha: str) -> str:
    """Return the full commit message of `sha`."""
    return git("log", "-1", "--format=%B", sha)


def is_signed_off(sha: str) -> bool:
    """True when the commit carries a Signed-off-by trailer whose identity
    matches the commit author (name AND email)."""
    author = commit_author(sha)
    body = commit_body(sha)
    matches = SIGNOFF_RE.findall(body)
    if not matches:
        return False
    # Accept when any trailer matches the author exactly; trailers from third
    # parties are deliberately NOT accepted as a substitute.
    return any(f"{name} <{email}>" == author for name, email in matches)


def commits_in_range(base: str, head: str) -> list[str]:
    """List commit SHAs in base..head (exclusive of base, inclusive of head)."""
    out = git("log", "--format=%H", f"{base}..{head}")
    return [line.strip() for line in out.splitlines() if line.strip()]


def ref_exists(ref: str) -> bool:
    """True when `ref` resolves in the current repository."""
    proc = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", ref],
        capture_output=True,
        text=True,
        check=False,
    )
    return proc.returncode == 0


def default_range() -> tuple[str, str] | None:
    """Determine (base, head) from the current branch when possible."""
    head = git("rev-parse", "--abbrev-ref", "HEAD").strip()
    if head == "HEAD":
        return None  # detached head; require an explicit base
    for base in ("origin/develop", "develop"):
        # Only use the base if it exists and the branch is ahead of it.
        if ref_exists(base):
            ahead = [
                line.strip()
                for line in git(
                    "log", "--format=%H", f"{base}..HEAD"
                ).splitlines()
                if line.strip()
            ]
            if ahead:
                return base, "HEAD"
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="base ref (default: origin/develop)")
    parser.add_argument("--self-test", action="store_true",
                        help="run the built-in test fixture instead of git")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    rng = None
    if args.base:
        if not ref_exists(args.base):
            sys.stderr.write(f"check-dco: base ref '{args.base}' not found\n")
            return 2
        rng = (args.base, "HEAD")
    else:
        rng = default_range()
    if rng is None:
        sys.stderr.write(
            "check-dco: no base found (detached HEAD or no develop ref); "
            "pass --base <ref>\n"
        )
        return 2

    base, head = rng
    shas = commits_in_range(base, head)
    if not shas:
        print(f"check-dco: no commits in {base}..{head}")
        return 0

    violations = [sha for sha in shas if not is_signed_off(sha)]
    if violations:
        sys.stderr.write(
            f"check-dco: {len(violations)} commit(s) without a matching "
            "Signed-off-by trailer:\n"
        )
        for sha in violations:
            sys.stderr.write(
                f"  {sha}  {commit_author(sha)}\n"
                f"      add:  git commit --amend -s\n"
            )
        return 1

    print(f"check-dco: {len(shas)} commit(s) all signed off")
    return 0


def self_test() -> int:
    """Verify the checker on a synthetic repository in a temp dir."""
    script = os.path.abspath(__file__)
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run(["git", "init", "-q", tmp], check=True)
        subprocess.run(
            ["git", "-C", tmp, "config", "user.name", "Dco Tester"],
            check=True,
        )
        subprocess.run(
            ["git", "-C", tmp, "config", "user.email", "dco@example.com"],
            check=True,
        )
        subprocess.run(
            ["git", "-C", tmp, "config", "commit.gpgsign", "false"],
            check=True,
        )

        def commit(msg: str) -> str:
            with open(
                os.path.join(tmp, "f.txt"), "a", encoding="utf-8"
            ) as fh:
                fh.write(msg + "\n")
            subprocess.run(["git", "-C", tmp, "add", "f.txt"], check=True)
            subprocess.run(
                ["git", "-C", tmp, "commit", "-q", "-m", msg], check=True
            )
            return subprocess.run(
                ["git", "-C", tmp, "rev-parse", "HEAD"],
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()

        base_sha = commit("base commit")
        commit("feat: signed\n\nSigned-off-by: Dco Tester <dco@example.com>")

        def run_check() -> int:
            return subprocess.run(
                [sys.executable, script, "--base", base_sha],
                cwd=tmp,
                capture_output=True,
                text=True,
            ).returncode

        # Range base..HEAD contains exactly one signed commit -> pass.
        ok_good = run_check() == 0

        # An unsigned commit on top must flip the check to a failure.
        commit("fix: unsigned commit")
        ok_bad = run_check() == 1

        if ok_good and ok_bad:
            print("check-dco self-test: OK")
            return 0
        print(
            f"check-dco self-test: FAIL "
            f"(signed_range_passes={ok_good}, unsigned_rejected={ok_bad})"
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
