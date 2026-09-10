# Release platforms

Rivulet should have one canonical release source and a small number of platform
integrations that improve discoverability without multiplying signing and
support costs. The canonical source remains [GitHub Releases](https://github.com/thoser666/rivulet/releases); all other channels should consume the same reproducible artifacts and only publish signed packages.

## Recommendation and order

| Stage | Milestone | Channel | Platforms | Recommendation | Current state |
| --- | --- | --- | --- | --- | --- |
| 1 | M5, supported by M7 | GitHub Releases | Windows, macOS, Linux | Keep as the source of truth for release notes, checksums, and updater downloads. | Active: MSI/portable ZIP, DMG, and AppImage are built by CI. |
| 2 | M5, supported by M7 | WinGet | Windows | Add after the MSI product identity and signing are stable. It provides native discovery and upgrades without another binary hosting system. | Manifest generation/validation are wired (packaging/windows/generate-winget-manifest.ps1 + dry-run job); submission to `microsoft/winget-pkgs` is still open (external review). |
| 2 | M5, supported by M7 | Scoop | Windows | Add now: Scoop requires no code signing, so this is the only native Windows package manager available before signing lands. The bucket repo hosts a generated, hash-pinned manifest. | Active: bucket `thoser666/scoop-bucket` (`scoop bucket add rivulet https://github.com/thoser666/scoop-bucket`); manifest generated + byte-verified by the **Distribution Readiness → scoop** dry-run job. |
| 2 | M5, supported by M7 | Flathub | Linux | Prefer this over maintaining distribution-specific packages. It gives Linux users a familiar, sandboxed, updateable installation. | Open: a Flatpak manifest, permissions review, and Flathub submission are needed. |
| 3 | M5 | Homebrew Cask | macOS | Useful for developer-oriented installs; publish only signed/notarized DMGs. | Readiness workflow validates the DMG; cask/tap submission is still open. |
| 3 | M5 | Steam | Windows, macOS | Worth preparing for the gaming-streamer audience, but treat it as a secondary channel rather than the update authority. | Open: Steam App ID, depots, SteamPipe credentials, store metadata, and a Steam-specific package layout. |
| 4 | M5 | Microsoft Store | Windows | Consider later for enterprise trust and discoverability. It requires MSIX packaging and Partner Center identity management. | Not ready: the current pipeline produces MSI/ZIP, not MSIX. |

WinGet and Flathub are the next M5 integrations; M7 supplies the reproducible
package and verification foundation they depend on. Steam is a good strategic
option once the product has a stable beta and a predictable update cadence; it
should not be used to bypass GitHub's release checks. Homebrew Cask is low effort
after notarization is reliable. The Microsoft Store should wait until an
MSIX-based installer is justified.

## Prepared workflow

`.github/workflows/distribution-readiness.yml` is a manual, dry-run-first
workflow. Select an existing GitHub release tag and a channel in **Actions →
Distribution Readiness**. It verifies that the channel's expected release
assets exist and writes a plan to the step summary. It does not submit anything
to a store and deliberately fails if `dry_run` is set to `false`; this prevents
accidental publication before credentials and manifests have been reviewed.

Once a channel is ready, its publishing implementation should be added as a
separate, explicitly permissioned job with:

- a dedicated token or environment approval;
- immutable artifact URLs and SHA-256 verification;
- a staging/dry-run mode that remains the default;
- a post-publish verification of the store listing and version;
- no access to signing secrets unless that channel actually signs an artifact.

## Channel-specific activation checklist

### WinGet

The chosen stable identity is `Rivulet.Rivulet` (publisher `Rivulet`). The
manifest is a `ManifestType: "singleton"` file (v1.6) consuming the GitHub
release asset `rivulet-windows-x86_64.msi` with its SHA-256, the MSI
`ProductCode` and the stable `UpgradeCode`.

1. Reserve the stable package identifier and publisher identity (`Rivulet.Rivulet` is reserved).
2. Ensure the signed MSI has a stable `UpgradeCode`, product identity, and
   silent-install/uninstall behavior.
3. Generate/validate the winget manifest with
   `packaging/windows/generate-winget-manifest.ps1` (canonical asset URL,
   SHA-256, MSI `ProductCode`/`UpgradeCode`), pinned by Pester tests
   (`generate-winget-manifest.tests.ps1`).
4. Trigger the **Distribution Readiness → winget** workflow: it runs the
   Pester tests, generates and verifies the manifest against the real release
   MSI, and dry-run reports the payload SHA-256 and package identity.
5. Submit the generated `<identifier>/<version>/<identifier>.yaml` folder as
   a PR to `microsoft/winget-pkgs` and let the community validation run.
6. Add an opt-in PR/dispatch workflow that submits only reviewed manifest
   changes; never upload unsigned binaries.

### Scoop

The only native Windows package manager that does **not** require code
signing — the channel is usable before SignPath approval lands. The manifest
(`bucket/rivulet.json` in [thoser666/scoop-bucket](https://github.com/thoser666/scoop-bucket))
consumes the portable ZIP asset with its SHA-256; the SHA-256 is taken from
the release's own `SHA256SUMS` asset, so a manifest can never reference an
unverified binary.

1. Trigger the **Distribution Readiness → scoop** workflow on a release tag:
   it runs the Pester tests (`generate-scoop-manifest.tests.ps1`), renders
   `rivulet.json` from the real release (SHA-256 via `SHA256SUMS`), and
   byte-verifies the render.
2. Copy the generated `rivulet.json` into the bucket repo's `bucket/`
   directory and push — publishing is a git commit, no external review.
3. Users install with `scoop bucket add rivulet
   https://github.com/thoser666/scoop-bucket` and `scoop install
   rivulet/rivulet`; updates come from `checkver.github`.
4. When winget goes live later, keep both: Scoop serves portable users and
   no-admin installs, winget serves MSI users.
5. **Automatic weekly updates**: the **Weekly release promotion** workflow
   (Mondays 07:09 UTC) regenerates `rivulet.json` for the promoted release
   and pushes it to the bucket via `SCOOP_BUCKET_TOKEN` (fine-grained PAT,
   contents:write on the bucket repo). Without the secret it publishes the
   manifest as a workflow artifact with a warning instead of failing — set
   the secret once and the bucket updates itself.

### Chocolatey

The Chocolatey **community repository** accepts unsigned installers (with a
moderator warning), so it is usable before SignPath approval — but unlike
Scoop every submission is a **moderated PR per version**. That makes it a
milestone channel, not a fast-follow: submit the first beta, not weekly
alphas, or the moderation queue rejects the churn.

The package is generated from the portable ZIP asset exactly like the Scoop
manifest: `generate-chocolatey-package.ps1` takes the SHA-256 from the
release's own `SHA256SUMS` asset, normalizes the version for Chocolatey
(dots are forbidden in prerelease suffixes: `0.65.0-alpha.163` →
`0.65.0-alpha163`), and emits `rivulet.nuspec` +
`tools/chocolateyInstall.ps1` wrapping `Install-ChocolateyZipPackage`.

1. Trigger the **Distribution Readiness → chocolatey** workflow on a release
   tag: it runs the Pester tests (`generate-chocolatey-package.tests.ps1`),
   renders the package from the real release, and byte-verifies the render.
2. Submit the generated package directory with `choco push` (API key from
   the community repository account) or open the package PR against the
   community repository — both are external and moderated.
3. When winget goes live later, Chocolatey stays as the second Windows
   option (winget serves MSI users, Chocolatey serves portable users who
   prefer choco); promote both from the same `weekly-latest` target.

### Promotion cadence: fast lane vs. weekly-latest

Rivulet publishes a release on every green push — that firehose is the
**fast lane** (GitHub Releases + the in-app updater). Package-manager
channels must not consume it: moderators reject per-release churn and store
listings read better with one coarser changelog per week. The **Weekly
release promotion** workflow (`.github/workflows/weekly-promotion.yml`,
Mondays 07:09 UTC, manual dispatch supported) is the **slow lane**:

1. It picks the **newest published release** (published == green: the
   release pipeline gates publishing on CI success) and additionally
   verifies the release commit has no failed check runs — never promotes a
   red release.
2. It moves the **`weekly-latest` tag** onto that release's commit. The tag
   is the slow-lane pointer. It deliberately does **not** create a release:
   the in-app updater reads the `/releases` endpoint and must keep following
   the fast lane.
3. It generates one **digest changelog** covering everything since the
   previous promotion (`generate-release-notes.sh --from-tag <prev>
   --digest`): features stay listed individually, everything else rolls up
   into per-section counts — a natural week of changes, not 30 alpha bullets.
4. It renders and byte-verifies the Scoop manifest for the promoted release
   and updates the bucket automatically (with `SCOOP_BUCKET_TOKEN`) or
   publishes the manifest as an artifact (without). WinGet/Chocolatey/Cask
   can later consume the same promotion target: the promoted tag + digest
   notes are exactly what their manifest PRs need.

To promote manually (e.g. right after a big feature lands):

```bash
gh workflow run weekly-promotion.yml -f release_tag v0.65.0-alpha.164
```

To see where the slow lane points:

```bash
git fetch --tags --force origin && git rev-parse weekly-latest^{commit}
```

### Flathub

1. Create a Flatpak manifest with explicit permissions and runtime choice.
2. Package the application using the upstream source or a reproducible build,
   not an opaque locally generated binary.
3. Add desktop file, AppStream metadata, icon, and sandbox capability review.
4. Open the Flathub submission PR and verify the app on a clean supported
   desktop session.
5. Add a post-release check that the Flathub version matches the GitHub tag.

### Homebrew Cask

1. Require a notarized macOS DMG and stable app bundle identifier.
2. Generate the cask from the GitHub release URL and SHA-256.
3. Submit/update the cask in the approved tap and verify quarantine behavior.
4. Keep the cask as a consumer of GitHub Releases; do not build a second
   unsigned artifact.

### Steam

1. Obtain a Steam App ID and configure Windows/macOS depots in Steamworks.
2. Define a SteamPipe content root and a versioned build script.
3. Store the Steam deployment token in an environment-protected Actions secret.
4. Upload only after GitHub's release, signing, and smoke checks are green.
5. Verify first-run behavior, update behavior, plugin paths, and crash-log
   locations in the Steam library installation.

Steam's version is free and can coexist with the normal installer, but Steam
Cloud, Workshop, and Steamworks-specific features are not prerequisites for
Rivulet's first release there.

### Microsoft Store

1. Decide whether MSIX is worth maintaining in addition to MSI.
2. Configure Partner Center identity, publisher certificate, and package
   identity.
3. Build and validate MSIX on a clean Windows runner.
4. Submit through a protected environment with manual approval.
5. Verify Store propagation and updater behavior before announcing the release.

## Version and update policy

GitHub Releases is the source of truth for version numbers and changelogs.
External channels must publish the exact same version and should lag a GitHub
release rather than create a competing version. Alpha releases remain GitHub-
first; WinGet, Flathub, Homebrew, Steam, and Microsoft Store should initially
publish only beta/stable releases after signing and rollback procedures have
been exercised.
