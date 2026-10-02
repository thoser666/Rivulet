#!/usr/bin/env python3
"""Fail when the repository has open CodeQL alerts.

The `ci_pinning` guards are static: they read the checked-out sources and
assert an invariant about them. That is the right place to catch a *new*
misconfiguration, but it can never notice an alert that GitHub already knows
about — alert #85 (`TokenPermissionsID` on the weekly promotion workflow) sat
open for days while every check was green, because nothing in this repository
ever asked the alerts API what it thought of us.

This script is that missing half. It fails on open alerts at or above a
severity threshold, minus an explicit, documented allowlist. An alert is not
allowed to linger: fix it, or dismiss it with a written reason (the state then
becomes `dismissed` and disappears from this check), or — if it genuinely
cannot be resolved — add it to `ALLOWED_OPEN_ALERTS` below together with the
condition under which the entry must be removed.

Output modes (the exit code is the same for all of them):
  (default)   one line per failing alert, plus a summary on stderr.
  --json      a single JSON document on stdout (machine-readable).
  --comment   a compact Markdown block for the step summary / an issue.

Usage:
  scripts/check-code-scanning-alerts.py [--repo OWNER/NAME] [--fail-on error,warning]
                                         [--json | --comment] [--self-test]

Requires `gh` and an authenticated token (`GH_TOKEN`, or `GITHUB_TOKEN` inside
Actions). `--self-test` needs neither: it exercises the classification logic
with in-memory fixtures and never touches the network.
"""

import argparse
import json
import os
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

SEVERITY_ORDER = ["none", "note", "warning", "error"]

# Alerts that are knowingly left open, mapped to the reason they cannot be
# resolved. Keep this empty unless a real, documented exception exists — each
# entry is an accepted, unfixed security finding, so it needs a justification
# *and* a condition for removal.
ALLOWED_OPEN_ALERTS: dict[int, str] = {}

DEFAULT_FAIL_ON = "error,warning"


def default_repo() -> str:
    """Resolve OWNER/NAME from the environment Actions provides."""
    for name in ("GITHUB_REPOSITORY", "GH_REPO"):
        value = os.environ.get(name)
        if value and "/" in value:
            return value
    return ""


def severity_at_least(severity: str, threshold: str) -> bool:
    """True when `severity` is at or above `threshold`.

    Unknown values are treated as *not* failing: a rule severity the API adds
    later must not silently turn every run red before anyone triaged it.
    """
    if severity not in SEVERITY_ORDER or threshold not in SEVERITY_ORDER:
        return False
    return SEVERITY_ORDER.index(severity) >= SEVERITY_ORDER.index(threshold)


def parse_thresholds(raw: str) -> list[str]:
    """Parse a `error,warning` style list, dropping junk and duplicates."""
    out: list[str] = []
    for part in raw.split(","):
        value = part.strip().lower()
        if value and value not in out:
            out.append(value)
    return out


def describe(alert: dict) -> str:
    """One-line human description of an alert."""
    number = alert.get("number", "?")
    rule = (alert.get("rule") or {}).get("id", "unknown-rule")
    severity = (alert.get("rule") or {}).get("severity", "unknown")
    location = alert.get("most_recent_instance", {}).get("location") or {}
    path = location.get("path", "unknown path")
    line = location.get("start_line")
    where = f"{path}:{line}" if line else path
    return f"#{number} [{severity}] {rule} — {where}"


def classify(alerts, allowed=None, thresholds=None):
    """Split alerts into (failures, tolerated).

    `failures` are open alerts at or above the lowest threshold that are not in
    `allowed`; `tolerated` are the allowlisted ones (kept for reporting so a
    stale entry stays visible).
    """
    allowed = ALLOWED_OPEN_ALERTS if allowed is None else allowed
    thresholds = parse_thresholds(DEFAULT_FAIL_ON) if thresholds is None else thresholds
    lowest = min(
        (SEVERITY_ORDER.index(t) for t in thresholds if t in SEVERITY_ORDER),
        default=SEVERITY_ORDER.index("error"),
    )
    failures, tolerated = [], []
    for alert in alerts:
        if alert.get("state", "open") != "open":
            continue
        severity = (alert.get("rule") or {}).get("severity", "")
        if severity not in SEVERITY_ORDER:
            continue
        if SEVERITY_ORDER.index(severity) < lowest:
            continue
        if alert.get("number") in allowed:
            tolerated.append(alert)
        else:
            failures.append(alert)
    return failures, tolerated


def fetch_open_alerts(repo: str):
    """Read every open code-scanning alert via the GitHub API.

    Uses `gh api --paginate`; the concatenated response is decoded with a raw
    decoder so it works whether gh emits one JSON array per page or slurped
    output, without depending on a particular gh version.
    """
    endpoint = f"repos/{repo}/code-scanning/alerts"
    proc = subprocess.run(
        [
            "gh",
            "api",
            "-H",
            "Accept: application/vnd.github+json",
            "--paginate",
            f"{endpoint}?state=open&per_page=100",
        ],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"gh api failed ({proc.returncode}): {proc.stderr.strip() or 'no stderr'}"
        )
    raw = proc.stdout.strip()
    if not raw:
        return []
    decoder = json.JSONDecoder()
    alerts, index = [], 0
    while index < len(raw):
        while index < len(raw) and raw[index].isspace():
            index += 1
        if index >= len(raw):
            break
        chunk, index = decoder.raw_decode(raw, index)
        if isinstance(chunk, list):
            alerts.extend(a for a in chunk if isinstance(a, dict))
        elif isinstance(chunk, dict):
            # `--slurp` style output: a list of page documents.
            for value in chunk.values():
                if isinstance(value, list):
                    alerts.extend(a for a in value if isinstance(a, dict))
    return alerts


def render_comment(failures, tolerated) -> str:
    """Markdown block for the Actions step summary."""
    lines = ["## Code scanning alerts", ""]
    if failures:
        lines.append(f"**{len(failures)} open alert(s) must be resolved or dismissed:**")
        lines.append("")
        for alert in failures:
            lines.append(f"- {describe(alert)}")
    else:
        lines.append("No open alerts at or above the failing threshold. ✅")
    if tolerated:
        lines += ["", f"_{len(tolerated)} allowlisted open alert(s) (stale entries are a bug):_"]
        for alert in tolerated:
            reason = ALLOWED_OPEN_ALERTS.get(alert.get("number"), "no reason recorded")
            lines.append(f"- {describe(alert)} — {reason}")
    return "\n".join(lines)


def self_test():
    """Guard the classification logic against silent regressions."""

    def alert(number, severity="error", state="open", path="a.yml"):
        return {
            "number": number,
            "state": state,
            "rule": {"id": "RuleID", "severity": severity},
            "most_recent_instance": {"location": {"path": path, "start_line": 7}},
        }

    # Only open alerts count; a dismissed/fixed one must never fail the gate.
    assert classify([alert(1, state="dismissed"), alert(2, state="fixed")])[0] == []
    # Severity threshold: the default fails on warning and above.
    assert classify([alert(3, severity="error")])[0][0]["number"] == 3
    assert classify([alert(4, severity="warning")])[0][0]["number"] == 4
    assert classify([alert(5, severity="note")])[0] == [], "note is below the threshold"
    assert classify([alert(6, severity="none")])[0] == [], "none is below the threshold"
    # An unknown severity must not turn every run red before it is triaged.
    assert classify([alert(7, severity="critical")])[0] == []
    # The threshold list is configurable and order-independent.
    assert classify([alert(8, severity="warning")], thresholds=["error"])[0] == []
    assert classify([alert(9, severity="warning")], thresholds=parse_thresholds("note,warning"))[0]
    assert parse_thresholds("ERROR, error ,,bogus") == ["error", "bogus"]
    # Allowlisted alerts are reported but do not fail.
    failures, tolerated = classify([alert(10)], allowed={10: "documented reason"})
    assert failures == [] and tolerated[0]["number"] == 10
    # The shipped allowlist must carry a reason for every entry.
    for number, reason in ALLOWED_OPEN_ALERTS.items():
        assert reason.strip(), f"allowlisted alert #{number} has no justification"
    # describe() must carry number, rule, severity and location.
    text = describe(alert(11, severity="warning", path="wf.yml"))
    assert "#11 [warning] RuleID — wf.yml:7" == text, text
    # An empty alert list is clean.
    assert classify([]) == ([], [])

    print("check-code-scanning-alerts: self-test passed")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", default=default_repo(), help="OWNER/NAME (default: $GITHUB_REPOSITORY)")
    parser.add_argument("--fail-on", default=DEFAULT_FAIL_ON, help="comma-separated severities (default: %(default)s)")
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    parser.add_argument("--comment", action="store_true", help="Markdown output for the step summary")
    parser.add_argument("--self-test", action="store_true", help="run the built-in tests and exit")
    args = parser.parse_args(argv)

    if args.self_test:
        self_test()
        return 0

    if not args.repo:
        print("check-code-scanning-alerts: no repository — pass --repo or set GITHUB_REPOSITORY", file=sys.stderr)
        return 2

    try:
        alerts = fetch_open_alerts(args.repo)
    except (RuntimeError, OSError) as error:
        print(f"check-code-scanning-alerts: {error}", file=sys.stderr)
        return 2

    failures, tolerated = classify(alerts, thresholds=parse_thresholds(args.fail_on))
    if args.json:
        print(json.dumps({
            "repository": args.repo,
            "failures": [describe(a) for a in failures],
            "tolerated": [describe(a) for a in tolerated],
        }, indent=2))
    elif args.comment:
        print(render_comment(failures, tolerated))
    else:
        for alert in failures:
            print(f"open code-scanning alert: {describe(alert)}")
        for alert in tolerated:
            print(f"allowlisted open alert: {describe(alert)}")
        if not failures:
            print(f"check-code-scanning-alerts: no open alerts at or above {args.fail_on} in {args.repo}")

    if failures:
        print(
            "check-code-scanning-alerts: every open alert must be fixed or dismissed with a "
            "written reason; see docs/security.md (Code scanning alert management).",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    sys.exit(main())