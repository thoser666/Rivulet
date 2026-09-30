#!/usr/bin/env python3
"""Verify that the `develop` ruleset forbids direct pushes.

GitHub evaluates rulesets server-side on every ref update, so the strongest
non-destructive per-PR proof is to re-read the live ruleset configuration and
assert the invariants that implement "no direct commits to the primary
branch" (OSPS-AC-03.01):

- a ruleset matching `refs/heads/develop` exists and is `active`;
- it lists NO bypass actors (direct pushes are rejected for every actor,
  repository administrators included — there is no `RepositoryRole` or
  `User`/`Team`/`Integration` exception);
- it carries the `pull_request` rule (all commits land via a PR) together
  with `deletion` and `non_fast_forward` protection;
- its required status checks still name the merge gate checks.

A real push probe would be the only "stronger" signal, but a push that lands
would itself mutate `develop` exactly when the ruleset is broken — so the
authoritative read-only check below is the safe continuous gate. It needs no
authentication for a public repository (Metadata read is enough); a token
from `GITHUB_TOKEN`/`GH_TOKEN` is used when present.

Usage:
    python3 scripts/check-develop-ruleset.py            # live check (solo mode)
    python3 scripts/check-develop-ruleset.py --self-test  # offline logic test
    python3 scripts/check-develop-ruleset.py --team-mode  # live check, team mode

Solo vs. team mode (staged review rollout, see docs/team-onboarding-runbook.md):

- **solo** (default): the `pull_request` rule must require 0 approving
  reviews — the automated checks are the review, and a count >= 1 would
  hard-block every merge because a maintainer cannot approve their own PR.
- **team**: the `pull_request` rule must require >= 1 approving review.
  Flip the mode in the same session that raises the live approval count
  (runbook step 4); until then the guard keeps enforcing the solo state.
"""

import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
# The merge-gate contexts are not pinned blindly: each one must exist as a
# job `name:` in one of these workflows (aggregates included — e.g. the
# ruleset requires the `CI` gate job, not every leaf job inside ci.yml).
# Verifying the names against the real workflow YAML keeps the contract
# verifiable in-repo; a rename on either side fails loudly instead of
# silently drifting out of the required-checks list.
MERGE_GATE_WORKFLOWS = (
    ".github/workflows/ci.yml",
    ".github/workflows/security.yml",
    ".github/workflows/scorecard.yml",
)

REPO = os.environ.get("RIVULET_REPO") or os.environ.get("GITHUB_REPOSITORY") or "thoser666/Rivulet"
API = "https://api.github.com"
DEVELOP_REF = "refs/heads/develop"
REQUIRED_RULES = ("deletion", "non_fast_forward", "pull_request", "required_status_checks")
# The exact contexts the merge gate requires (must match the live ruleset and
# the ci_pinning expectations in rivulet-core/tests/ci_pinning.rs).
REQUIRED_CHECKS = (
    "CI",
    "Security",
    "OpenSSF Scorecard",
    "CodeQL (rust)",
    "Dependency Review",
    "Pinning-Tests",
)


def _scan_job_names_lines(text: str) -> set[str]:
    """No-PyYAML fallback: scan `jobs:`-section `name:` fields line-based.

    Matches exactly the structure this repo uses (top-level `jobs:` at column
    0, two-space job ids, their four-space `name:` fields). Template names are
    kept raw (callers handle `${{ ... }}` prefixes); a refactor away from that
    shape surfaces as an empty/partial set, i.e. a loud contract failure.
    """
    names: set[str] = set()
    in_jobs = False
    for line in text.splitlines():
        if line.startswith("jobs:"):
            in_jobs = True
            continue
        if not in_jobs:
            continue
        if line and not line[0].isspace():
            break
        if line.startswith("    name:") and ":" in line:
            value = line.split(":", 1)[1].strip()
            if value:
                names.add(value.strip("\"'").strip())
    return names


def _expand_matrix_name(name: str, job: dict) -> set[str]:
    """Expand `Foo (${{ matrix.key }})` job names against strategy.matrix."""
    keys = re.findall(r"\$\{\{\s*matrix\.([A-Za-z_][A-Za-z0-9_]*)\s*\}\}", name)
    if not keys:
        return {name}
    matrix = (job.get("strategy") or {}).get("matrix") or {}
    values = matrix.get(keys[0])
    if len(keys) == 1 and isinstance(values, list) and all(isinstance(v, str) for v in values):
        return {name.replace("${{ matrix." + keys[0] + " }}", v) for v in values}
    # Unresolvable template (multi-key or non-list matrix): keep the raw name.
    return {name}


def _job_check_names(workflow: Path) -> set[str]:
    """Return the status-check contexts one workflow's jobs produce.

    Structured (PyYAML) parse of the `jobs:` mapping's `name:` fields, with
    `${{ matrix.key }}` templates expanded against the job's strategy matrix
    (e.g. `CodeQL (${{ matrix.language }})` yields `CodeQL (rust)`). Falls
    back to the conservative line-based scan when PyYAML is unavailable.
    """
    text = workflow.read_text(encoding="utf-8")
    try:
        import yaml  # type: ignore
    except ImportError:
        return _scan_job_names_lines(text)
    data = yaml.safe_load(text) or {}
    jobs = data.get("jobs") or {}
    names: set[str] = set()
    for job in jobs.values():
        if not isinstance(job, dict):
            continue
        name = job.get("name")
        if not isinstance(name, str) or not name.strip():
            continue
        if "${{" in name:
            names.update(_expand_matrix_name(name, job))
        else:
            names.add(name.strip())
    return names


def merge_gate_contexts() -> set[str]:
    """Union of job-derived check contexts across MERGE_GATE_WORKFLOWS."""
    contexts: set[str] = set()
    for rel in MERGE_GATE_WORKFLOWS:
        contexts |= _job_check_names(REPO_ROOT / rel)
    return contexts


def verify_merge_gate_contract() -> None:
    """Fail loudly when REQUIRED_CHECKS drifts from the workflow job names.

    A required context counts as defined when it is an exact job name, or
    when it is produced by a matrix-expanded job name, or (in the no-PyYAML
    fallback) when a raw template job name's prefix matches.
    """
    defined = merge_gate_contexts()
    prefixes = tuple(
        name.split("${{")[0]
        for name in defined
        if "${{" in name
    )
    missing = [
        c
        for c in REQUIRED_CHECKS
        if c not in defined and not any(c.startswith(p) for p in prefixes)
    ]
    assert not missing, (
        f"merge-gate context(s) {missing} are not job names of {MERGE_GATE_WORKFLOWS} — "
        "update REQUIRED_CHECKS or the workflow job names in the same change"
    )

FAILURES: list[str] = []


def fail(message: str) -> None:
    FAILURES.append(message)
    print(f"  FAIL: {message}")


def ok(message: str) -> None:
    print(f"  ok: {message}")


def _request(url: str, token: str | None) -> dict:
    headers = {"Accept": "application/vnd.github+json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.load(resp)


def _ruleset_payloads(token: str | None) -> list[dict]:
    """Return (id, detail) for every repository ruleset."""
    listing = _request(f"{API}/repos/{REPO}/rulesets", token)
    details = []
    for entry in listing:
        rid = entry["id"]
        try:
            details.append(_request(f"{API}/repos/{REPO}/rulesets/{rid}", token))
        except urllib.error.HTTPError as exc:
            fail(f"cannot read ruleset {rid} detail (HTTP {exc.code}) - is the repo public or the token scoped?")
    return details


def evaluate(payloads: list[dict], team_mode: bool = False, required_checks: tuple = REQUIRED_CHECKS) -> bool:
    """Validate the given ruleset payloads; returns True when compliant.

    ``required_checks`` lets the self-test drive arbitrary merge-gate sets
    without touching the module-level default (which is parsed from the real
    ci.yml at import time).
    """
    develop = None
    for rs in payloads:
        included = []
        for cond in rs.get("conditions", {}).values():
            included.extend(cond.get("include", []))
        if DEVELOP_REF in included:
            develop = rs
            break
    if develop is None:
        fail(f"no repository ruleset covers {DEVELOP_REF}")
        return False

    name = develop.get("name", "?")
    if develop.get("enforcement") != "active":
        fail(f"ruleset `{name}` enforcement is {develop.get('enforcement')!r}, expected 'active'")
    else:
        ok(f"ruleset `{name}` is active and covers {DEVELOP_REF}")

    bypass = develop.get("bypass_actors", [])
    if bypass:
        who = ", ".join(f"{b.get('actor_type')}#{b.get('actor_id')} ({b.get('bypass_mode')})" for b in bypass)
        fail(f"ruleset `{name}` has bypass actors - direct pushes are NOT blocked for everyone: {who}")
    else:
        ok("ruleset lists no bypass actors - direct pushes are rejected for every actor")

    rule_types = [r.get("type") for r in develop.get("rules", [])]
    for expected in REQUIRED_RULES:
        if expected in rule_types:
            ok(f"rule `{expected}` present")
        else:
            fail(f"rule `{expected}` missing (have: {rule_types})")

    pr_params = {}
    for rule in develop.get("rules", []):
        if rule.get("type") == "pull_request":
            pr_params = rule.get("parameters", {})
    if "pull_request" in rule_types and team_mode:
        if (pr_params.get("required_approving_review_count") or 0) < 1:
            fail(
                "pull_request rule requires >= 1 approving review in team mode "
                f"(have {pr_params.get('required_approving_review_count')})"
            )
        else:
            ok("pull_request rule requires a human approving review (team mode)")
    elif "pull_request" in rule_types and pr_params.get("required_approving_review_count") != 0:
        fail(
            "pull_request rule must not require a second human approval "
            "(single-maintainer repo; automated checks are the review)"
        )
    elif "pull_request" in rule_types:
        ok("pull_request rule allows automated-review merges (approval count 0)")

    checks = []
    for rule in develop.get("rules", []):
        if rule.get("type") == "required_status_checks":
            for ctx in rule.get("parameters", {}).get("required_status_checks", []):
                checks.append(ctx.get("context"))
    missing = [c for c in required_checks if c not in checks]
    if missing:
        fail(f"required status checks missing: {missing}")
    else:
        ok("all merge-gate status checks are required")

    return not FAILURES


def run_live() -> int:
    team_mode = "--team-mode" in sys.argv
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    print(
        f"Checking rulesets for {REPO} ({'team' if team_mode else 'solo'} mode, "
        f"{'authenticated' if token else 'anonymous'})"
    )
    try:
        verify_merge_gate_contract()
        print("  ok: every merge-gate context is a job name in-repo")
    except AssertionError as exc:
        print(f"FATAL: {exc}")
        return 2
    try:
        payloads = _ruleset_payloads(token)
    except urllib.error.HTTPError as exc:
        if exc.code == 403 and not token:
            # On pull_request events the GITHUB_TOKEN lacks administration:read
            # so the rulesets API returns 403 for unauthenticated callers.
            # The same check runs on push to develop (where the token is
            # available), so this is safe to skip.
            print("SKIP: rulesets API returned HTTP 403 (unauthenticated on PR)")
            print("The check runs on push to develop where the token is available.")
            return 0
        print(f"FATAL: rulesets API returned HTTP {exc.code} for {REPO}")
        return 2
    except urllib.error.URLError as exc:
        print(f"FATAL: cannot reach the GitHub API: {exc}")
        return 2
    if not payloads:
        print("FATAL: no repository rulesets found")
        return 2
    evaluate(payloads, team_mode=team_mode)
    if FAILURES:
        print("\nRESULT: FAIL - the develop ruleset does NOT match the expected review policy")
        return 1
    print("\nRESULT: PASS - the develop ruleset blocks direct pushes (no bypass actors)")
    return 0


def self_test() -> int:
    """Offline logic test with canned payloads (no network)."""

    # The job-name contract: every REQUIRED_CHECKS context must actually be
    # produced by a merge-gate workflow's jobs (structured YAML parse with
    # matrix expansion) — a rename on either side fails loudly here instead
    # of drifting out of the required-checks list.
    verify_merge_gate_contract()
    print("  ok: merge-gate contract verified against workflow job names")

    def ruleset(**overrides) -> dict:
        base = {
            "name": "develop",
            "enforcement": "active",
            "conditions": {"ref_name": {"exclude": [], "include": [DEVELOP_REF]}},
            "bypass_actors": [],
            "rules": [
                {"type": "deletion"},
                {"type": "non_fast_forward"},
                {"type": "pull_request", "parameters": {"required_approving_review_count": 0}},
                {
                    "type": "required_status_checks",
                    "parameters": {
                        "required_status_checks": [
                            {"context": c} for c in REQUIRED_CHECKS
                        ]
                    },
                },
            ],
        }
        base.update(overrides)
        return base

    def ruleset_with(approval_count: int) -> dict:
        return ruleset(
            rules=[
                {"type": "deletion"},
                {"type": "non_fast_forward"},
                {"type": "pull_request", "parameters": {"required_approving_review_count": approval_count}},
                {
                    "type": "required_status_checks",
                    "parameters": {
                        "required_status_checks": [
                            {"context": c} for c in REQUIRED_CHECKS
                        ]
                    },
                },
            ]
        )

    scenarios = [
        ("compliant ruleset passes", [ruleset()], True, False),
        ("admin bypass actor fails", [ruleset(bypass_actors=[{"actor_type": "RepositoryRole", "actor_id": 5, "bypass_mode": "always"}])], False, False),
        ("disabled enforcement fails", [ruleset(enforcement="disabled")], False, False),
        ("pull_request rule removed fails", [ruleset(rules=[{"type": "deletion"}, {"type": "non_fast_forward"}, {"type": "required_status_checks", "parameters": {"required_status_checks": [{"context": c} for c in REQUIRED_CHECKS]}}])], False, False),
        ("wrong branch rule ignored", [ruleset(conditions={"ref_name": {"exclude": [], "include": ["refs/heads/other"]}})], False, False),
        ("no ruleset at all fails", [], False, False),
        ("solo mode: approval count 1 fails", [ruleset_with(1)], False, False),
        ("team mode: approval count 1 passes", [ruleset_with(1)], True, True),
        ("team mode: approval count 0 fails", [ruleset()], False, True),
    ]
    failed = 0
    for label, payloads, expect_pass, team_mode in scenarios:
        FAILURES.clear()
        got = evaluate(payloads, team_mode=team_mode)
        status = "ok" if got == expect_pass else "WRONG"
        if got != expect_pass:
            failed += 1
        print(f"  [{status}] {label}: expected {'pass' if expect_pass else 'fail'}, got {'pass' if got else 'fail'}")
    if failed:
        print(f"\nself-test: {failed} scenario(s) failed")
        return 1
    print("\nself-test: all scenarios behave as expected")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    return run_live()


if __name__ == "__main__":
    sys.exit(main())
