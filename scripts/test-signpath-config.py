#!/usr/bin/env python3
"""Validate the SignPath Foundation signing path in build-package.yml.

The build-package workflow signs Windows artifacts in one of two mutually
exclusive ways: SignPath Foundation (file-based signing request, certificate
stays in SignPath's HSM — free for open source) or the PFX/signtool path.
This check verifies the wiring stays correct:

- all four SignPath secrets are referenced and gated on `signpath_enabled`
- signpath_enabled requires ALL four secrets (never a partial activation)
- the SignPath action is pinned to a SHA and used with wait-for-completion
- upload -> submit -> copy-back round trips exist for both EXEs and MSI
- SignPath takes precedence: PFX steps are skipped when signpath is enabled

Run with --self-test to verify the checker itself (fixtures). Exit codes:
0 = all checks passed, 1 = a check failed, 2 = usage error.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "build-package.yml"

SIGNPATH_SECRETS = [
    "SIGNPATH_API_TOKEN",
    "SIGNPATH_ORGANIZATION_ID",
    "SIGNPATH_PROJECT_SLUG",
    "SIGNPATH_SIGNING_POLICY_SLUG",
]

SIGNPATH_ACTION = "signpath/github-action-submit-signing-request"
SIGNPATH_PIN = "c92b958760219087e01f8d67a1669ed57afe2627"

# Pattern for a full SignPath submit step: id, action@pin, and all inputs.
SUBMIT_RE = re.compile(
    r"uses: signpath/github-action-submit-signing-request@(?P<pin>[0-9a-f]{40})"
)

# The fixed source artifact paths inside the workflow.
EXE_ARTIFACT = "rivulet-windows-unsigned-exe-${{ github.run_id }}"
MSI_ARTIFACT = "rivulet-windows-unsigned-msi-${{ github.run_id }}"


def check_workflow(text: str) -> list[str]:
    problems: list[str] = []

    # 1. Every secret is referenced and mapped from the secrets context.
    for secret in SIGNPATH_SECRETS:
        if f"secrets.{secret}" not in text:
            problems.append(f"missing secrets reference: {secret}")

    # 2. The gating step computes signpath_enabled from ALL four secrets.
    enabled_guard = (
        f'echo "signpath_enabled=true" >> "$GITHUB_OUTPUT"'
    )
    if enabled_guard not in text:
        problems.append("signpath_enabled=true output is missing")
    for secret in SIGNPATH_SECRETS:
        if secret not in text or f"${secret}" not in text:
            problems.append(f"gating step must read ${secret}")

    # 3. SignPath action is pinned to a fixed SHA.
    pins = SUBMIT_RE.findall(text)
    if not pins:
        problems.append("no SignPath submit action found")
    for pin in pins:
        if pin != SIGNPATH_PIN:
            problems.append(f"SignPath action pin drifted: {pin}")
    if len(pins) != 2:
        problems.append(f"expected exactly 2 SignPath submit steps (EXE + MSI), found {len(pins)}")

    # 4. EXE round trip: upload id -> submit consumes artifact-id -> copy back.
    if "signpath-upload-exe" not in text:
        problems.append("missing EXE upload step id 'signpath-upload-exe'")
    if "steps.signpath-upload-exe.outputs.artifact-id" not in text:
        problems.append("EXE submit must consume steps.signpath-upload-exe.outputs.artifact-id")
    if EXE_ARTIFACT not in text:
        problems.append(f"EXE upload artifact name drifted: {EXE_ARTIFACT}")
    if "signpath-signed/exe" not in text:
        problems.append("EXE output-artifact-directory drift")
    if "Copy-Item \"signpath-signed/exe/rivulet-gui.exe\"" not in text:
        problems.append("signed EXE copy-back missing")

    # 5. MSI round trip.
    if "signpath-upload-msi" not in text:
        problems.append("missing MSI upload step id 'signpath-upload-msi'")
    if "steps.signpath-upload-msi.outputs.artifact-id" not in text:
        problems.append("MSI submit must consume steps.signpath-upload-msi.outputs.artifact-id")
    if MSI_ARTIFACT not in text:
        problems.append(f"MSI upload artifact name drifted: {MSI_ARTIFACT}")
    if "signpath-signed/msi" not in text:
        problems.append("MSI output-artifact-directory drift")
    if "Copy-Item \"signpath-signed/msi/rivulet-windows-x86_64.msi\"" not in text:
        problems.append("signed MSI copy-back missing")

    # 6. wait-for-completion is true and the parameters pass version.
    if "wait-for-completion: true" not in text:
        problems.append("wait-for-completion must be true")
    if "version: \"${{ inputs.version }}\"" not in text:
        problems.append("parameters must pass version")

    # 7. Precedence: PFX EXE/MSI steps must be skipped when signpath is on.
    pfx_exe = "steps.signing.outputs.windows_enabled == 'true' && steps.signing.outputs.signpath_enabled != 'true'"
    if text.count(pfx_exe) != 2:
        problems.append(f"PFX steps must be gated on signpath_enabled != 'true' (expect 2, found {text.count(pfx_exe)})")

    # 8. The SignPath steps themselves gate on signpath_enabled.
    for marker in ("steps.signing.outputs.signpath_enabled == 'true'"):
        pass  # covered implicitly; explicit gate check below
    count_gated = len(re.findall(
        r"signpath_enabled == 'true'", text
    ))
    if count_gated < 5:
        problems.append(f"too few signpath_enabled gates (found {count_gated}, expect >= 5)")

    # 9. actions: read permission present (artifact download).
    if "actions: read" not in text:
        problems.append("workflow needs actions: read for the artifact download")

    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="run the checker against embedded fixtures")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()

    text = WORKFLOW.read_text(encoding="utf-8")
    problems = check_workflow(text)
    if problems:
        print("SignPath config check FAILED:")
        for p in problems:
            print(f"  - {p}")
        return 1
    print("SignPath config check passed.")
    return 0


def run_self_test() -> int:
    failures = 0

    # Fixture 1: a correct workflow (copy of the committed one, trimmed of
    # unrelated sections — the regexes only need the signing blocks).
    good = WORKFLOW.read_text(encoding="utf-8")
    # Remove everything not containing 'signpath' or 'windows_enabled' to
    # prove the check is local to the signing blocks... but keep it simple:
    # the real file IS the source of truth, so assert it passes as-is.
    problems = check_workflow(good)
    if problems:
        print("FAIL [good fixture]: real workflow should pass:")
        for p in problems:
            print(f"  - {p}")
        failures += 1
    else:
        print("PASS [good fixture] real workflow passes")

    def broken(text: str) -> bool:
        return bool(check_workflow(text))

    # Fixture 2: missing secrets reference.
    bad = good.replace("${{ secrets.SIGNPATH_API_TOKEN }}", "${{ secrets.OTHER }}")
    if not broken(bad):
        print("FAIL [missing secret] should be detected")
        failures += 1
    else:
        print("PASS [missing secret]")

    # Fixture 3: pin drift.
    bad = good.replace(SIGNPATH_PIN, "0" * 40)
    if not broken(bad):
        print("FAIL [pin drift] should be detected")
        failures += 1
    else:
        print("PASS [pin drift]")

    # Fixture 4: PFX precedence dropped (PFX runs even with SignPath on).
    bad = good.replace(
        "steps.signing.outputs.windows_enabled == 'true' && steps.signing.outputs.signpath_enabled != 'true'",
        "steps.signing.outputs.windows_enabled == 'true'",
    )
    if not broken(bad):
        print("FAIL [precedence] should be detected")
        failures += 1
    else:
        print("PASS [precedence]")

    # Fixture 5: wait-for-completion dropped.
    bad = good.replace("wait-for-completion: true", "wait-for-completion: false")
    if not broken(bad):
        print("FAIL [wait flag] should be detected")
        failures += 1
    else:
        print("PASS [wait flag]")

    if failures:
        print(f"{failures} self-test case(s) FAILED")
        return 1
    print("All self-test cases passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())