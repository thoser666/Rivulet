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
    python3 scripts/check-develop-ruleset.py            # live check
    python3 scripts/check-develop-ruleset.py --self-test  # offline logic test
"""

import json
import os
import sys
import urllib.error
import urllib.request

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


def evaluate(payloads: list[dict]) -> bool:
    """Validate the given ruleset payloads; returns True when compliant."""
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
    if "pull_request" in rule_types and pr_params.get("required_approving_review_count") != 0:
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
    missing = [c for c in REQUIRED_CHECKS if c not in checks]
    if missing:
        fail(f"required status checks missing: {missing}")
    else:
        ok("all merge-gate status checks are required")

    return not FAILURES


def run_live() -> int:
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    print(f"Checking rulesets for {REPO} ({'authenticated' if token else 'anonymous'})")
    try:
        payloads = _ruleset_payloads(token)
    except urllib.error.HTTPError as exc:
        print(f"FATAL: rulesets API returned HTTP {exc.code} for {REPO}")
        return 2
    except urllib.error.URLError as exc:
        print(f"FATAL: cannot reach the GitHub API: {exc}")
        return 2
    if not payloads:
        print("FATAL: no repository rulesets found")
        return 2
    evaluate(payloads)
    if FAILURES:
        print("\nRESULT: FAIL - the develop ruleset does NOT block direct pushes")
        return 1
    print("\nRESULT: PASS - the develop ruleset blocks direct pushes (no bypass actors)")
    return 0


def self_test() -> int:
    """Offline logic test with canned payloads (no network)."""

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

    scenarios = [
        ("compliant ruleset passes", [ruleset()], True),
        ("admin bypass actor fails", [ruleset(bypass_actors=[{"actor_type": "RepositoryRole", "actor_id": 5, "bypass_mode": "always"}])], False),
        ("disabled enforcement fails", [ruleset(enforcement="disabled")], False),
        ("pull_request rule removed fails", [ruleset(rules=[{"type": "deletion"}, {"type": "non_fast_forward"}, {"type": "required_status_checks", "parameters": {"required_status_checks": [{"context": c} for c in REQUIRED_CHECKS]}}])], False),
        ("wrong branch rule ignored", [ruleset(conditions={"ref_name": {"exclude": [], "include": ["refs/heads/other"]}})], False),
        ("no ruleset at all fails", [], False),
    ]
    failed = 0
    for label, payloads, expect_pass in scenarios:
        FAILURES.clear()
        got = evaluate(payloads)
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
