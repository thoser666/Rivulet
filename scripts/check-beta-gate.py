#!/usr/bin/env python3
"""Check beta-readiness against the six Beta-Gate criteria in README.md.

The Beta-Gate (see README → Roadmap → Beta-Gate) is a manual, criteria-based
decision: the project leaves alpha only when all six verifiable criteria are
met. This script evaluates them:

  1. M1 – Solid Recording complete          (no open roadmap checkboxes)
  2. M3 – Streaming complete                (no open roadmap checkboxes)
  3. Platform parity (M5 release blocker)   (3-OS CI build matrix + macOS
                                            parity checkbox in the roadmap)
  4. Code-signing secrets configured        (GitHub API, needs a token)
  5. CI fully green on develop              (GitHub API, needs a token)
  6. No known release-blocking bugs         (GitHub API, needs a token)

Criteria 4–6 are checked against the GitHub REST API and therefore need a
token (`GITHUB_TOKEN` in CI, `GH_TOKEN` locally, or `--token`). Without a
token they are reported as `unverified` and do not count as met. Note that a
workflow `GITHUB_TOKEN` can never read Actions secrets, so criterion 4 stays
`unverified` inside CI by design — it is confirmed locally with a PAT that
has read access to the repository secrets.

Output modes (the exit code is the same regardless of mode):

- (default)      one line per criterion plus a verdict on stderr.
- `--json`       a single JSON document on stdout (machine-readable).
- `--comment`    a compact Markdown dashboard for the GitHub Actions step
                 summary, an issue, or a PR comment.

Exit codes: `0` by default — this is an informational dashboard, and the
project is expected to be NOT READY while it is still in alpha. Pass `--fail`
to turn any not-met or unverified criterion into exit `1` (e.g. as a release
gate for a beta tag).

Usage:
    scripts/check-beta-gate.py [--fail] [--json | --comment]
        [--repo owner/repo] [--ref develop] [--token TOKEN]
"""

import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
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

REPO_ROOT = Path(__file__).resolve().parent.parent
README = REPO_ROOT / "README.md"
CI_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "ci.yml"

CHECKBOX_RE = re.compile(r"^\s*-\s*\[([ xX])\]\s*(.+)$")
OS_LIST_RE = re.compile(r"^\s*os:\s*\[\s*(.+?)\s*\]\s*$")
GITHUB_REMOTE_RE = re.compile(r"github\.com[/:]([^/]+)/([^/]+?)(?:\.git)?$")

# All secrets the signing automation needs (README → Releases → Code signing).
# Windows signing is satisfied by EITHER the PFX/signtool pair OR the SignPath
# Foundation secrets (free for open source, file-based signing request); macOS
# always needs the Apple set. Linux GPG is optional for the beta gate.
WINDOWS_PFX_SECRETS = ["WINDOWS_CERT_BASE64", "WINDOWS_CERT_PASSWORD"]
WINDOWS_SIGNPATH_SECRETS = [
    "SIGNPATH_API_TOKEN",
    "SIGNPATH_ORGANIZATION_ID",
    "SIGNPATH_PROJECT_SLUG",
    "SIGNPATH_SIGNING_POLICY_SLUG",
]
MACOS_SECRETS = [
    "MACOS_CERT_BASE64",
    "MACOS_CERT_PASSWORD",
    "APPLE_ID",
    "APPLE_APP_PASSWORD",
    "APPLE_TEAM_ID",
]
REQUIRED_SECRETS = WINDOWS_PFX_SECRETS + WINDOWS_SIGNPATH_SECRETS + MACOS_SECRETS


def windows_signing_satisfied(present):
    """Windows signing is configured with the PFX pair OR the SignPath set."""
    return all(s in present for s in WINDOWS_PFX_SECRETS) or all(
        s in present for s in WINDOWS_SIGNPATH_SECRETS
    )


def missing_signing_secrets(present):
    """Return the secrets still missing. Windows is satisfied by EITHER path:
    when one Windows alternative is complete, the other's secrets are NOT
    reported missing (only macOS is always mandatory)."""
    missing: list[str] = []
    if not windows_signing_satisfied(present):
        missing += [s for s in WINDOWS_PFX_SECRETS if s not in present]
        missing += [s for s in WINDOWS_SIGNPATH_SECRETS if s not in present]
    missing += [s for s in MACOS_SECRETS if s not in present]
    return missing

BLOCKER_LABEL = "release-blocker"

STATUS_ICONS = {"met": "✅", "not-met": "❌", "unverified": "❔"}


def read_milestone_section(text, needle):
    """Return the lines under the `###` header containing `needle` (up to the
    next `###` header)."""
    lines = text.splitlines()
    start = None
    for i, line in enumerate(lines):
        if line.strip().startswith("###") and needle in line:
            start = i
            break
    if start is None:
        return []
    end = len(lines)
    for i in range(start + 1, len(lines)):
        if lines[i].strip().startswith("###"):
            end = i
            break
    return lines[start:end]


def count_checkboxes(section_lines):
    """Return ``(open_items, done_items)`` for a roadmap section."""
    open_items = 0
    done_items = 0
    for line in section_lines:
        match = CHECKBOX_RE.match(line)
        if match:
            if match.group(1) in "xX":
                done_items += 1
            else:
                open_items += 1
    return open_items, done_items


def find_checkbox_state(section_lines, needle):
    """Return True/False for the checkbox whose text contains `needle`, or None
    if no such checkbox exists."""
    for line in section_lines:
        match = CHECKBOX_RE.match(line)
        if match and needle in match.group(2):
            return match.group(1) in "xX"
    return None


def ci_platforms():
    """Return the set of OSes in the `os: [ ... ]` matrix lists of ci.yml."""
    text = CI_WORKFLOW.read_text(encoding="utf-8")
    platforms = set()
    for line in text.splitlines():
        match = OS_LIST_RE.match(line)
        if match:
            for part in re.split(r"[,\s]+", match.group(1).strip()):
                if part:
                    platforms.add(part)
    return platforms


def resolve_repo(override):
    """Determine the ``owner/repo`` from --repo, GITHUB_REPOSITORY, or git."""
    if override:
        return override
    env = os.environ.get("GITHUB_REPOSITORY")
    if env:
        return env
    try:
        proc = subprocess.run(
            ["git", "remote", "get-url", "origin"],
            capture_output=True,
            text=True,
        )
        match = GITHUB_REMOTE_RE.search(proc.stdout.strip())
        if match:
            return f"{match.group(1)}/{match.group(2)}"
    except (OSError, subprocess.SubprocessError):
        pass
    return "unknown/unknown"


def gh_api(token, repo, path):
    """GET a GitHub REST endpoint; return parsed JSON or None on failure."""
    url = f"https://api.github.com/repos/{repo}/{path}"
    request = urllib.request.Request(
        url,
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
            "User-Agent": "rivulet-beta-gate-check",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            return json.load(response)
    except (urllib.error.HTTPError, urllib.error.URLError, OSError, ValueError):
        return None


def check_secrets(token, repo):
    """Return ``(present, missing)`` signing secrets, or ``(None, None)`` if
    unverifiable. Windows is satisfied by either the PFX pair or the SignPath
    Foundation set; macOS secrets are always mandatory."""
    data = gh_api(token, repo, "actions/secrets")
    if data is None or not isinstance(data, dict) or "secrets" not in data:
        return None, None
    present = {s["name"] for s in data["secrets"]}
    return present, missing_signing_secrets(present)


def check_ci_green(token, repo, ref):
    """Return True if the latest completed CI run on `ref` succeeded, or None
    if unverifiable."""
    data = gh_api(
        token,
        repo,
        f"actions/runs?branch={ref}&per_page=20&status=completed",
    )
    if data is None or not isinstance(data, dict) or "workflow_runs" not in data:
        return None
    ci_runs = [r for r in data["workflow_runs"] if r.get("name") == "CI"]
    if not ci_runs:
        return None
    return ci_runs[0].get("conclusion") == "success"


def check_no_blockers(token, repo):
    """Return True if no open issue is labeled `release-blocker`, or None if
    unverifiable."""
    data = gh_api(token, repo, f"issues?state=open&labels={BLOCKER_LABEL}&per_page=100")
    if data is None or not isinstance(data, list):
        return None
    blockers = [i for i in data if "pull_request" not in i]
    return len(blockers) == 0


def signpath_note(present):
    """Return the auto-signing notice if all four SIGNPATH_* secrets exist.

    ``present`` is the set of configured secret names (or None when the
    secrets list could not be read). Returns ``None`` when the notice does
    not apply — notably when the PFX path is configured instead, or when
    only some SignPath secrets are set (the workflow stays disabled then).
    """
    if present and all(s in present for s in WINDOWS_SIGNPATH_SECRETS):
        return (
            "🖋️ All four SIGNPATH_* secrets are set — the next release "
            "signs automatically via SignPath Foundation (EXEs + MSI)."
        )
    return None


def evaluate(repo, ref, token):
    """Return ``(results, verdict, notes)`` for the six criteria.

    ``notes`` carries informational one-liners for the dashboard (e.g. the
    SignPath auto-signing notice) — they never affect the verdict.
    """
    notes: list[str] = []
    readme = README.read_text(encoding="utf-8")

    m1_open, m1_done = count_checkboxes(
        read_milestone_section(readme, "M1 – Solid Recording")
    )
    m3_open, m3_done = count_checkboxes(
        read_milestone_section(readme, "M3 – Streaming")
    )

    platforms = ci_platforms()
    canonical = {
        "ubuntu-latest": "linux",
        "windows-latest": "windows",
        "macos-latest": "macos",
    }
    present = {canonical.get(p, p) for p in platforms}
    missing_platforms = sorted({"linux", "windows", "macos"} - present)
    m5_parity = find_checkbox_state(
        read_milestone_section(readme, "M5 – Ecosystem"), "Windows/macOS feature parity"
    )

    if missing_platforms:
        c3_status, c3_detail = "not-met", (
            f"CI build matrix missing: {', '.join(missing_platforms)}"
        )
    elif m5_parity is False:
        c3_status, c3_detail = "not-met", (
            "macOS capture parity not implemented (M5 roadmap checkbox is open)"
        )
    elif m5_parity is None:
        c3_status, c3_detail = "not-met", "M5 parity checkbox not found in README"
    else:
        c3_status, c3_detail = "met", (
            f"3-OS build matrix ({', '.join(sorted(present))}) + M5 parity checked"
        )

    results = [
        {
            "criterion": 1,
            "name": "M1 – Solid Recording complete",
            "status": "met" if m1_open == 0 else "not-met",
            "detail": f"{m1_open} open / {m1_done} done roadmap items",
        },
        {
            "criterion": 2,
            "name": "M3 – Streaming complete",
            "status": "met" if m3_open == 0 else "not-met",
            "detail": f"{m3_open} open / {m3_done} done roadmap items",
        },
        {"criterion": 3, "name": "Platform parity (M5 release blocker)", "status": c3_status, "detail": c3_detail},
    ]

    if token:
        present, missing = check_secrets(token, repo)
        if missing is None:
            results.append(
                {
                    "criterion": 4,
                    "name": "Code-signing secrets configured",
                    "status": "unverified",
                    "detail": "could not read the secrets list (check token permissions)",
                }
            )
        elif missing:
            results.append(
                {
                    "criterion": 4,
                    "name": "Code-signing secrets configured",
                    "status": "not-met",
                    "detail": "missing: " + ", ".join(sorted(missing)),
                }
            )
        else:
            windows = (
                "PFX"
                if all(s in present for s in WINDOWS_PFX_SECRETS)
                else "SignPath"
                if all(s in present for s in WINDOWS_SIGNPATH_SECRETS)
                else "none"
            )
            results.append(
                {
                    "criterion": 4,
                    "name": "Code-signing secrets configured",
                    "status": "met",
                    "detail": f"Windows signing via {windows}, macOS signing secrets complete",
                }
            )

        green = check_ci_green(token, repo, ref)
        if green is None:
            results.append(
                {
                    "criterion": 5,
                    "name": "CI fully green on develop",
                    "status": "unverified",
                    "detail": f"could not query the latest CI run on `{ref}`",
                }
            )
        else:
            results.append(
                {
                    "criterion": 5,
                    "name": "CI fully green on develop",
                    "status": "met" if green else "not-met",
                    "detail": (
                        f"latest completed CI run on `{ref}` succeeded"
                        if green
                        else f"latest completed CI run on `{ref}` did not succeed"
                    ),
                }
            )

        no_blockers = check_no_blockers(token, repo)
        if no_blockers is None:
            results.append(
                {
                    "criterion": 6,
                    "name": "No known release-blocking bugs",
                    "status": "unverified",
                    "detail": f"could not query open issues labeled `{BLOCKER_LABEL}`",
                }
            )
        else:
            results.append(
                {
                    "criterion": 6,
                    "name": "No known release-blocking bugs",
                    "status": "met" if no_blockers else "not-met",
                    "detail": (
                        f"no open issues labeled `{BLOCKER_LABEL}`"
                        if no_blockers
                        else f"open issues labeled `{BLOCKER_LABEL}` exist"
                    ),
                }
            )

        # SignPath notice: the moment all four SIGNPATH_* secrets exist, the
        # release workflow activates the SignPath path (it takes precedence
        # over the PFX path) and the next release signs automatically. This
        # note must never affect the verdict — it is pure good news.
        note = signpath_note(present)
        if note:
            notes.append(note)
    else:
        for criterion, name in (
            (4, "Code-signing secrets configured"),
            (5, "CI fully green on develop"),
            (6, "No known release-blocking bugs"),
        ):
            results.append(
                {
                    "criterion": criterion,
                    "name": name,
                    "status": "unverified",
                    "detail": "no token set (GITHUB_TOKEN / GH_TOKEN / --token)",
                }
            )

    not_met = [r for r in results if r["status"] == "not-met"]
    unverified = [r for r in results if r["status"] == "unverified"]
    if not_met:
        verdict = "NOT READY"
    elif unverified:
        verdict = "READY (unverified items)"
    else:
        verdict = "READY"
    return results, verdict, notes


def redact_sensitive_details(results):
    """Return a copy of results with sensitive details removed for logging/output."""
    redacted = []
    for r in results:
        item = dict(r)
        if item.get("criterion") == 4 and item.get("status") == "not-met":
            item["detail"] = "required signing secrets are not fully configured"
        redacted.append(item)
    return redacted


def render_comment(results, verdict, repo, ref, notes=()):
    """Return a compact Markdown dashboard for the step summary / an issue."""
    lines = ["## 🚦 Beta-Gate status", ""]
    if verdict == "READY":
        lines.append(f"✅ **BETA-READY** — all six criteria met on `{repo}` `{ref}`.")
    else:
        lines.append(f"⚠️ **{verdict}** — beta-gate check for `{repo}` `{ref}`.")
    lines += ["", "| # | Criterion | Status |", "| --- | --- | --- |"]
    for r in results:
        lines.append(
            f"| {r['criterion']} | {r['name']} | "
            f"{STATUS_ICONS[r['status']]} {r['status']} |"
        )
    lines.append("")
    for r in results:
        if r["status"] != "met":
            lines.append(f"- **{r['name']}:** {r['detail']}")
    if notes:
        lines += [""]
        lines += [f"- {n}" for n in notes]
    return "\n".join(lines) + "\n"


def run_self_test():
    """Verify the SignPath notice logic and the dashboard wiring locally.

    Returns 0 when all cases pass, 1 otherwise. Kept dependency-free so it
    runs the same way on a laptop and in CI.
    """
    failures = 0

    def case(name, ok):
        nonlocal failures
        print(("PASS" if ok else "FAIL") + f" [{name}]")
        if not ok:
            failures += 1

    # 1. All four SignPath secrets set -> notice fires.
    note = signpath_note(set(WINDOWS_SIGNPATH_SECRETS))
    case(
        "all four SIGNPATH secrets",
        note is not None and "signs automatically" in note,
    )

    # 2. Only some SignPath secrets -> workflow stays disabled, no notice.
    case(
        "partial SIGNPATH secrets",
        signpath_note({"SIGNPATH_API_TOKEN"}) is None,
    )

    # 3. PFX path configured instead -> no notice.
    case(
        "pfx instead of signpath",
        signpath_note(set(WINDOWS_PFX_SECRETS)) is None,
    )

    # 4. Secrets unreadable -> no notice, and no crash.
    case("unreadable secrets list", signpath_note(None) is None)

    # 5. The notice renders into the dashboard and never into a status cell.
    sample = render_comment(
        [
            {
                "criterion": 4,
                "name": "Code-signing secrets configured",
                "status": "met",
                "detail": "Windows signing via SignPath, macOS signing secrets complete",
            }
        ],
        "READY",
        "r/rivulet",
        "develop",
        notes=["note-a", "note-b"],
    )
    case(
        "notice renders as dashboard bullet",
        "note-a" in sample.split("| # | Criterion |", 1)[1]
        and "- note-b" in sample
        and sample.splitlines()[-1] == "- note-b",
    )

    if failures:
        print(f"{failures} self-test case(s) FAILED")
        return 1
    print("All self-test cases passed.")
    return 0


def main():
    args = sys.argv[1:]
    if "--help" in args or "-h" in args:
        print(__doc__.strip())
        return 0
    if "--self-test" in args:
        return run_self_test()

    fail = "--fail" in args
    as_json = "--json" in args
    as_comment = "--comment" in args or "--github-comment" in args
    repo = resolve_repo(next((args[i + 1] for i, a in enumerate(args) if a == "--repo"), None))
    ref = next((args[i + 1] for i, a in enumerate(args) if a == "--ref"), "develop")
    token = next((args[i + 1] for i, a in enumerate(args) if a == "--token"), None)
    token = token or os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")

    results, verdict, notes = evaluate(repo, ref, token)
    safe_results = redact_sensitive_details(results)
    not_met = [r for r in safe_results if r["status"] == "not-met"]
    unverified = [r for r in safe_results if r["status"] == "unverified"]
    failed = bool(fail and (not_met or unverified))

    if as_json:
        print(
            json.dumps(
                {
                    "ok": not failed,
                    "fail": fail,
                    "verdict": verdict,
                    "repo": repo,
                    "ref": ref,
                    "token_present": bool(token),
                    "criteria": safe_results,
                    "notes": list(notes),
                },
                indent=2,
            )
        )
    elif as_comment:
        print(render_comment(safe_results, verdict, repo, ref, notes))
    else:
        for r in safe_results:
            print(f"[{r['criterion']}] {r['name']}: {r['status']} — {r['detail']}")
        for note in notes:
            print(note)
        print(
            f"\nVerdict: {verdict} ({len(not_met)} not met, "
            f"{len(unverified)} unverified, on {repo} @ {ref})",
            file=sys.stderr,
        )

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
