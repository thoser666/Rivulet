# Release platforms

Rivulet should have one canonical release source and a small number of platform
integrations that improve discoverability without multiplying signing and
support costs. The canonical source remains [GitHub Releases](https://github.com/thoser666/rivulet/releases); all other channels should consume the same reproducible artifacts and only publish signed packages.

## Recommendation and order

| Stage | Milestone | Channel | Platforms | Recommendation | Current state |
| --- | --- | --- | --- | --- | --- |
| 1 | M5, supported by M7 | GitHub Releases | Windows, macOS, Linux | Keep as the source of truth for release notes, checksums, and updater downloads. | Active: MSI/portable ZIP, DMG, and AppImage are built by CI. |
| 2 | M5, supported by M7 | WinGet | Windows | Add after the MSI product identity and signing are stable. It provides native discovery and upgrades without another binary hosting system. | Readiness workflow validates the MSI and portable ZIP; manifest submission is still open. |
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

1. Reserve the stable package identifier and publisher identity.
2. Ensure the signed MSI has a stable `UpgradeCode`, product identity, and
   silent-install/uninstall behavior.
3. Generate a WinGet manifest with the release URL and SHA-256 hash.
4. Submit it to `microsoft/winget-pkgs` and let the community validation run.
5. Add an opt-in PR/dispatch workflow that submits only reviewed manifest
   changes; never upload unsigned binaries.

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
