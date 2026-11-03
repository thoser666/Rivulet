# First beta: v0.66.0 — what is still missing

## Beta-gate prerequisites

The gate itself lives in `scripts/check-beta-gate.py` and is evaluated with a
token:

```bash
GH_TOKEN="$(gh auth token)" python3 scripts/check-beta-gate.py
```

**State on 2026-11-03: `NOT READY (1 not met)`.** Five of the six criteria are
met; only criterion 4 is open:

| # | Criterion | Status | What is missing |
| --- | --- | --- | --- |
| 4 | Code-signing secrets configured | **not-met** | one complete Windows signing path (2 or 4 secrets) |
| 1, 2, 3, 5, 6 | M1/M3 complete, platform parity, CI green on `develop`, no release blockers | ✅ met | — |

### The beta scope is Windows and Linux

`v0.66.0` ships Windows and Linux. **macOS is not part of this beta scope** and
therefore not part of the gate: `scripts/check-beta-gate.py` requires
`WINDOWS_PFX_SECRETS` **or** `WINDOWS_SIGNPATH_SECRETS` and keeps macOS out of
the requirement on purpose (`REQUIRED_SECRETS = ...  # macOS excluded from beta
scope`). The five `MACOS_*`/`APPLE_*` secrets only become necessary once macOS
enters the scope — Apple Developer Program, $99/year, tracked in issue #50 and
described in [`docs/code-signing.md`](../code-signing.md). Do not buy that
membership for `v0.66.0`.

**Missing secrets — one of the two Windows paths is enough:**

```
Path A (PFX):      WINDOWS_CERT_BASE64, WINDOWS_CERT_PASSWORD
Path B (SignPath): SIGNPATH_API_TOKEN, SIGNPATH_ORGANIZATION_ID,
                   SIGNPATH_PROJECT_SLUG, SIGNPATH_SIGNING_POLICY_SLUG
```

Path B is free for qualifying open source, but the SignPath Foundation
application was rejected in October 2026 (too few stars/downloads). The
realistic options today are therefore a purchased OV certificate
(≈ $100–200/year, Path A) or Azure Artifact Signing ($9.99/month); the free
comparison and the exact steps are in [`docs/code-signing.md`](../code-signing.md).

## What happens once the gate is met

1. Set one complete Windows path: Path A (`WINDOWS_CERT_BASE64` +
   `WINDOWS_CERT_PASSWORD`) **or** Path B (all four `SIGNPATH_*`). The two paths
   are mutually exclusive — `build-package.yml` prefers SignPath and falls back
   to signtool.
2. Re-run the gate: `GH_TOKEN="$(gh auth token)" python3 scripts/check-beta-gate.py`
   → the verdict flips to `READY`.
3. Create the manual tag and push it: `git tag v0.66.0` (no alpha suffix), then
   `git push origin v0.66.0`.
4. CI builds the tag through the beta/RC/stable path in `ci.yml`;
   `build-package.yml` signs the packages with the configured path and
   publishes them.
5. The release appears as a **pre-release** (`v0.66.0`).

## Notes

- **Linux needs no signing secret**: the AppImage ships a detached GPG
  signature (`LINUX_GPG_PRIVATE_KEY`, optional passphrase), which the gate does
  not require. Scoop and the other package channels need no signing at all.
- **The gate is manual and criteria-based, never date-driven** — the six
  criteria are defined in the [README](../README.md#beta-gate) and evaluated by
  the script; the release channel only describes *how* the tag is built.
- **`GITHUB_TOKEN` in Actions can never read Actions secrets**, so criterion 4
  stays `unverified` in CI by design and is confirmed locally with a PAT that
  can read repository secrets.
- **Alpha releases are unaffected**: `Release (Alpha)` is triggered by
  `workflow_run` after a green CI run, not by a tag.

## Checklist

- [ ] One complete Windows path (A **or** B, not both)
- [ ] `GH_TOKEN="$(gh auth token)" python3 scripts/check-beta-gate.py` → `READY`
- [ ] `git tag v0.66.0`
- [ ] `git push origin v0.66.0`
- [ ] CI run → `build-package.yml` signs and publishes
- [ ] Release is listed as a pre-release (`v0.66.0`)

## Links

- [Code signing: `docs/code-signing.md`](../code-signing.md) — all paths with
  step-by-step instructions
- [Release pipeline: `.github/workflows/build-package.yml`](../../.github/workflows/build-package.yml)
  — secret definitions and the SignPath action
- [Beta-gate script: `scripts/check-beta-gate.py`](../../scripts/check-beta-gate.py)
  — criteria 1–6
- [Beta-gate criteria in the README](../../README.md#beta-gate)
