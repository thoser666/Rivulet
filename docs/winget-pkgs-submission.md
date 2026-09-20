# WinGet-Pkgs Submission Flow — Weekly Automated PRs

**Status:** Design (implementation-ready) · **Scope:** M5 distribution, M7
W4 reproducible inputs · **Prepared:** 2026-09-20

## Goal

Turn the weekly **Prepare WinGet manifest (weekly)** artifact into a real
submission to [`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs):
after every actual promotion, a PR is opened automatically that adds (or
updates) the `Rivulet.Rivulet` manifest for the promoted version. The
manifest content stays exactly the byte-verified output of
`packaging/windows/generate-winget-manifest.ps1` — the submission step never
regenerates or rewrites it.

## Current state

- The weekly promotion already renders the singleton manifest (v1.6) with
  the real MSI metadata (`ProductCode`, `UpgradeCode`, `InstallerSha256`)
  and publishes it as `winget-manifest-<tag>` (artifact, byte-exact
  re-render verified, Pester-tested generator).
- What is missing is only the last mile: fork → branch → PR on
  `microsoft/winget-pkgs`.

## Submission mechanism: `wingetcreate submit`

**Chosen:** `microsoft/winget-create` with its `submit` verb.

| Option | Verdict |
| --- | --- |
| `wingetcreate submit <manifest-dir> --token <PAT>` | ✅ Takes an **existing** manifest directory, validates it, forks upstream, pushes a branch, opens the PR. No manifest regeneration. One package+version per PR — exactly the upstream rule. |
| `Komac` | Strong automation tool, but its update/submit flow regenerates manifests from installer metadata — we would lose our Pester-pinned generator as the single source of truth. |
| Raw GitHub API (fork/contents/PR) | Full control but ~150 lines of fork-branch-PR plumbing to maintain; wingetcreate already encodes upstream's submission rules. |

wingetcreate is Microsoft's own manifest creator, so its submission
behavior tracks the repository's current validation rules.

## Flow (new job in `weekly-promotion.yml`)

```
promote (existing)
  └─ submit-winget  (new, needs: promote)
       if: up_to_date == 'false'            # real promotion happened
       1. gate: WINGET_SUBMIT_TOKEN set?    # absent → ::warning:: + skip
       2. download artifact winget-manifest-<tag> from this run
       3. dedup:
          a. upstream version dir exists?   # contents API → skip if 200
          b. open PR by us for this version? # gh search prs → skip
       4. wingetcreate submit <dir> --token …
       5. echo the PR URL into the step summary + artifact `winget-submit-plan-<tag>`
```

- **Same workflow, not a separate schedule:** reuses the promotion's
  `up_to_date` gate and its manifest artifact; the PR points at exactly
  the promoted release. No second cron to keep in sync.
- **Cadence:** at most one submission per promotion (in practice ≤ 1/week
  via the Monday cron). Every alpha release does **not** get a submission —
  only promoted (weekly) ones do. This keeps the firehose out of
  Microsoft's validation pipeline.

## Dedup rules

1. `GET /repos/microsoft/winget-pkgs/contents/manifests/r/Rivulet/Rivulet/<version>`
   → HTTP 200 means the version is already in the catalog: skip.
2. `gh search prs --repo microsoft/winget-pkgs --author <us> --state open
   "Rivulet.Rivulet version <version>"` → hit means a submission is already
   in flight: skip (never open duplicate PRs; upstream rejects them).
3. Both miss → submit.

A closed-but-unmerged PR (e.g. the 10-day `Needs-Author-Feedback` timeout)
passes rule 2, so the next promotion re-submits automatically.

## Token & CLA (one-time human steps)

1. **PAT:** a fine-grained PAT is preferred if it works for forks;
   the documented-fallback is a **classic PAT with only the `public_repo`
   scope** (wingetcreate needs to fork upstream, push a branch to the fork,
   and open the PR — it never touches private repos). Store it as the
   repository secret `WINGET_SUBMIT_TOKEN` on `thoser666/Rivulet`. Rotate
   at 90 days like `SCOOP_BUCKET_TOKEN`.
2. **CLA:** the first PR triggers the Microsoft CLA check; `thoser666`
   signs once via the PR comment flow. One-time, then automated.
3. **First submission note:** the first PR ever for this package goes
   through manual moderator review (new-package review); subsequent
   version updates are usually auto-approved after the Azure validation
   pass.

## Failure modes & lifecycle

- `wingetcreate` failure → job fails, `::error::` annotation, the manifest
  artifact from the prepare job remains for a manual fallback.
- Upstream labels (`Needs-Author-Feedback`, `Binary-Validation-Error`,
  …) arrive as bot comments on the PR; the maintainer acts on GitHub
  notifications. The step summary of every promotion lists the current
  in-flight PR URL (fetched in the dedup step), so status is visible
  without extra tooling.
- Hash mismatch / availability errors are upstream realities; our manifest
  hashes come from the release `SHA256SUMS`, so a mismatch means the asset
  changed after publishing — investigate the release, never "fix" the PR
  by editing hashes.

## Rollout

- **Phase A (this change):** the job ships **disabled by default** —
  without `WINGET_SUBMIT_TOKEN` it logs a `::warning::` and skips,
  mirroring the Scoop-bucket rollout pattern that worked.
- **Phase B (activation):** set the secret. The next promotion submits the
  current promoted version.
- **Signing note:** the MSI is currently unsigned; winget-pkgs accepts
  unsigned installers (SmartScreen warning on install). Per the M5 gate
  the *preferred* activation point is right after the first signed
  release — the flow is identical either way, activation is one secret.

## Security considerations

- The PAT lives only as a repo secret; it is never echoed (Actions masks
  it) and grants no access to private repositories (`public_repo`).
- The submitted manifest is the byte-verified artifact of the same run —
  a mismatch between prepared and submitted manifest is impossible.
- Fork/branch/PR happen inside wingetcreate against upstream public
  content; no script injection surface (the token is passed as a CLI
  argument via an env-indirection, never inline in shell expressions).

## Quality gate / pinning

New `ci_pinning` guard `winget_pkgs_submission_is_designed_and_gated`:

- the job exists in `weekly-promotion.yml`, is gated on
  `needs.promote.outputs.up_to_date == 'false'` **and** on the
  `WINGET_SUBMIT_TOKEN` secret,
- it downloads the `winget-manifest-` artifact (no manifest regeneration
  in the submit path) and calls `wingetcreate submit`,
- both dedup checks (upstream version dir + open-PR search) are present,
- the design doc exists and is referenced from `docs/release-platforms.md`.

## References

- [Submit your manifest to the repository](https://learn.microsoft.com/en-us/windows/package-manager/package/repository)
- [winget-create (`submit`)](https://github.com/microsoft/winget-create)
- [WinGet Releaser action](https://github.com/marketplace/actions/winget-releaser) (alternative pattern, not used: regenerates manifests)
