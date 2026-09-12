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
| 2 | M5 | AUR | Linux (Arch) | No signing required; the PKGBUILD downloads the AppImage from GitHub Releases and extracts it. AUR submissions are external and not moderated per-version. | PKGBUILD wired (`packaging/aur/PKGBUILD`); CI validates the PKGBUILD against the real release via the **Distribution Readiness → aur** dry-run job. AUR push is external. |
| 2 | M5, supported by M7 | Flathub | Linux | Prefer this over maintaining distribution-specific packages. It gives Linux users a familiar, sandboxed, updateable installation. | Manifest wired (packaging/flatpak/org.rivulet.Rivulet.yml, pinned offline-cargo build + CI build/lint job); submission PR and permissions/appstream review are still open (external). |
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
6. **Weekly preparation (one release per week):** the **Weekly release
   promotion** workflow re-renders and byte-verifies the manifest from the
   real MSI of the promoted `weekly-latest` release and publishes the
   validated payload as a `winget-manifest-<tag>` artifact. Opening the
   winget-pkgs PR therefore stays a human, copy-paste step (external
   review), but it is scoped to exactly one reviewed manifest per week —
   never a per-alpha chore. Add an opt-in PR/dispatch workflow only after
   external reviews are flowing; never upload unsigned binaries.

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
Scoop every submission is a **moderated PR per version**. It consumes the
**weekly-latest** slow lane: one reviewed package per week, never a
per-alpha version, so the moderation queue sees exactly one review scope
per week (see step 3 below).

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
3. **Weekly submission (one release per week):** the **Weekly release
   promotion** workflow re-renders and byte-verifies the package for the
   promoted `weekly-latest` release and publishes the validated payload as
   a `chocolatey-package-<tag>` artifact. Upload **one** reviewed package
   per week with `choco push` — the community repository rejects churn, so
   never submit per-alpha versions; the weekly-latest artifact is the single
   review scope per week.
4. When winget goes live later, Chocolatey stays as the second Windows
   option (winget serves MSI users, Chocolatey serves portable users who
   prefer choco); promote both from the same `weekly-latest` target.

### AUR (Arch User Repository)

The AUR is the community package repository for Arch Linux and derivatives
(Manjaro, EndeavourOS, …). It accepts PKGBUILDs that download pre-built
binaries from upstream — no signing required, no moderation queue per
version. This makes it a **fast-follow** channel: available immediately
when the PKGBUILD is published.

The PKGBUILD (`packaging/aur/PKGBUILD`) downloads the AppImage asset from
GitHub Releases, extracts it with `--appimage-extract` (works without FUSE
during build), and installs the contents to `/opt/rivulet/` with binary
symlinks in `/usr/bin/`, a desktop file, and the 512×512 icon.

1. Trigger the **Distribution Readiness → aur** workflow on a release tag:
   it validates the PKGBUILD version matches the release, the required
   assets (AppImage + icon) exist, and the `.install` file is referenced.
2. Push the PKGBUILD to the AUR git repo
   (`https://aur.archlinux.org/rivulet.git`) — external, not automated.
3. **Weekly status (one release per week):** the **Weekly release
   promotion** workflow checks whether the committed PKGBUILD already pins
   the promoted version and reports the exact bump (`pkgver` + `sha256sums`
   + push) when it does not. The AUR push stays external and manual; the
   weekly run keeps the todo visible and validated.
4. Users install with `yay -S rivulet` or `paru -S rivulet`.

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
4. It **prepares every active channel** for that single promoted release,
   once per week, on the same tag + digest notes:
   - **Scoop**: renders and byte-verifies `rivulet.json` and updates the
     bucket automatically (with `SCOOP_BUCKET_TOKEN`) or publishes the
     manifest as an artifact (without).
   - **WinGet**: renders and byte-verifies the manifest from the real MSI
     (via `generate-winget-manifest.ps1`, including its Pester suite) and
     publishes the validated payload as a `winget-manifest-<tag>` artifact.
   - **Chocolatey**: renders and byte-verifies the package (via
     `generate-chocolatey-package.ps1`, including its Pester suite) and
     publishes the validated payload as a `chocolatey-package-<tag>`
     artifact.
   - **AUR**: validates PKGBUILD structure and reports whether it already
     pins the promoted version or hands off the exact bump (`pkgver` +
     `sha256sums` + push).
   The external pushes (winget-pkgs PR, `choco push`, AUR push) stay human
   and reviewed; the artifacts make each one a copy-paste of a
   pre-validated payload, scoped to exactly one release per week.

To promote manually (e.g. right after a big feature lands):

```bash
gh workflow run weekly-promotion.yml -f release_tag v0.65.0-alpha.164
```

To see where the slow lane points:

```bash
git fetch --tags --force origin && git rev-parse weekly-latest^{commit}
```

### Flathub

The chosen stable Flatpak id is `org.rivulet.Rivulet`, built from the
`org.freedesktop.Platform`/`org.freedesktop.Sdk` 25.08 runtime with the
`org.freedesktop.Sdk.Extension.rust-stable` SDK extension.

1. The manifest `packaging/flatpak/org.rivulet.Rivulet.yml` lists the
   application module, the desktop file, the AppStream metainfo, and the icon;
   their `finish-args` are intentional (screen/audio capture, Vulkan/DRM,
   PipeWire/PulseAudio, network for stream ingest + telemetry opt-in, `$HOME`
   for recordings/config) and must be signed off by a permissions review.
2. Cargo is fully offline in the build: the crate archives are pinned in
   `packaging/flatpak/cargo/cargo-sources.json` (1239 crates from `Cargo.lock`,
   generated by the official `flatpak-cargo-generator` at the commit pinned in
   `packaging/flatpak/generate-cargo-sources.sh`) and merged into the manifest
   as flatpak sources (URL + SHA-256, extracted to `cargo/vendor/...`), while
   `packaging/flatpak/cargo/config.toml` maps crates-io to the vendored
   directory so cargo never touches the network. Re-run the script whenever
   `Cargo.lock` changes; CI fails if the committed file drifts.
3. `packaging/flatpak/org.rivulet.Rivulet.desktop` and
   `packaging/flatpak/org.rivulet.Rivulet.metainfo.xml` (`launchable`
   desktop-id matches the desktop file; `project_license`, `developer_name`,
   release entry). `flatpak-builder` runs `appstream-compose` on every build,
   so AppStream validation is part of the CI job.
4. CI job `flatpak-build.yml` validates the manifest end to end on every push:
   installs `flatpak` + `flatpak-builder` from apt, installs the 25.08 runtime/
   SDK and the `rust-stable` extension, runs the cargo-offline build, the
   export and bundle, then the official Flathub lint
   (`org.flathub.flatpak-builder-lint`)
   for manifest, appstream, and desktop. The
   **Distribution Readiness → flathub** dry-run adds the real-release asset
   prerequisite to the checklist.
5. Open the Flathub submission PR (the manifest as submitted must point at the
   upstream git tag, which the PR fork adjusts) and verify the app on a clean
   supported desktop session.
6. Add a post-release check that the Flathub version matches the GitHub tag.

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
