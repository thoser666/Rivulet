# Backfilling orphaned alpha releases

## What is an orphaned release?

The alpha release pipeline ([`release.yml`](../.github/workflows/release.yml))
runs on every `feat:` push to `develop` and consists of two independent steps:

1. **Determine version & tag** — computes the next version, bumps
   `Cargo.toml`/`CHANGELOG.md`, commits `chore(release): prepare vX.Y.Z-alpha.N`
   and **pushes the tag** `vX.Y.Z-alpha.N`.
2. **Build + GitHub release** — builds the platform packages and, only when
   **all three** platform builds succeed, creates the formal GitHub Release.

Because the tag is pushed *before* the builds run, a failing platform build
leaves the repository with a **tag that has no GitHub Release** — an
*orphaned* tag. The updater and the changelog chain then skip that version.

```
feat: ...  ──▶  tag vX.Y.Z-alpha.N ──▶  build linux ✓  build windows ✓  build mac ✗
                                            │                                │
                                            └──── GitHub release SKIPPED ◀──┘
```

## When is a backfill needed?

- A release workflow run shows `Create Alpha GitHub Release` as **skipped**.
- A version appears in the git tags but not in
  `https://github.com/thoser666/rivulet/releases`.
- The changelog chain has a gap (e.g. `v0.17.0` → `v0.19.0` with `v0.18.0`
  missing).

## Prerequisites

- `gh` CLI installed and authenticated (`gh auth status`) with **write** access
  to the repository (needed for `gh release create`/`upload`).
- Local git checkout with the tags fetched (`git fetch --tags origin`).
- ~3 GB free disk space: the Windows artifact (MSI + portable ZIP) is ~1.3 GB
  per tag and uploads are performed from the local cache.

## Quick start

```bash
# 1. Detect orphaned tags (read-only, no changes)
scripts/backfill-releases.sh --check

# 2. Preview what a real run would do (no downloads, no changes)
scripts/backfill-releases.sh --dry-run v0.18.0-alpha.1

# 3. Backfill ALL orphaned alpha tags
scripts/backfill-releases.sh

# 3b. ... or only specific tags
scripts/backfill-releases.sh v0.18.0-alpha.1 v0.19.0-alpha.1
```

## How it works

For each tag the script:

1. **Skips** tags that already have a published release; **resumes** draft
   releases (uploads are `--clobber`, so interrupted uploads are repaired).
2. **Resolves the original workflow run**: the tag points at the
   `chore(release): prepare ...` commit, whose parent is the head commit of the
   triggering push — the run's `headSha`. Tags without a matching run (e.g.
   ancient bootstrap tags that predate the automation) are reported and
   skipped.
3. **Downloads the artifacts** (`gh run download`) into
   `/tmp/backfill-releases/<tag>/`. Each platform artifact is only downloaded
   when its expected file set is incomplete, so interrupted downloads are
   completed instead of repeated. A platform that never built (e.g. macOS)
   is not an error — it is simply reported as missing.
4. **Creates a draft release** with notes stating which platforms are missing
   and linking the original run.
5. **Uploads** every artifact present locally plus the social preview images
   (`docs/opengraph.png`, `docs/social-preview.png`).
6. **Publishes** the draft (`--draft=false`).

Work is cached in `/tmp/backfill-releases/` (override with
`RIVULET_BACKFILL_WORK=/path`), the log is written to
`/tmp/backfill-releases/backfill.log`, and a lock file prevents two instances
from running concurrently (parallel runs once corrupted a draft release).

## Safety & idempotency

- **Re-runnable**: published tags are skipped, downloads resume, uploads
  overwrite (`--clobber`).
- **No destructive operations**: the script never deletes tags or releases;
  it only creates/edits draft releases and uploads assets.
- **Partial artifact sets are preserved**: a tag whose macOS build failed gets
  the Linux/Windows packages it actually produced — with a transparent note in
  the release body. Nothing is fabricated or rebuilt.
- **Dry run** (`--dry-run`) performs no downloads and no API mutations.

## Limitations

- **Artifacts expire** after GitHub's retention period (default 90 days).
  Backfills must happen while the workflow-run artifacts are still stored.
- **Only what was built is available**: if a platform build failed, that
  package is permanently missing for the tag. The release notes document this
  explicitly.
- **Bootstrap-era tags** (`v0.0.0`–`v0.2.3`, before working packaging CI) have
  no usable artifacts; the script skips them with a note.

## Troubleshooting

| Symptom | Cause / fix |
| --- | --- |
| `Another backfill instance is already running` | A previous run is still active (e.g. a timed-out foreground call keeps its child running). Wait for it to finish or remove the lock (`/tmp/backfill-releases/.lock` / `.lockdir`) after confirming no `gh run download`/`gh release upload` process is alive. |
| `no release workflow run found for this tag` | The tag predates the release automation or the local tags are stale. Run `git fetch --tags origin` and retry. |
| `WARNING: ... still missing after download` | The download was interrupted mid-extraction. Re-run the script; it re-downloads the incomplete artifact. |
| `no artifacts available for this tag — skip` | The original run failed on every platform (no artifacts stored). Nothing can be backfilled. |
| Upload `401/403` | `gh` is authenticated with a token lacking `repo` write scope or the run is for a fork. Check `gh auth status`. |

## Related

- [Release pipeline](../.github/workflows/release.yml) — where tags and
  releases are created.
- [`scripts/backfill-releases.sh`](../scripts/backfill-releases.sh) — the
  detection + backfill tool.
- [Releases section of the README](../README.md#-releases) — channel overview.
