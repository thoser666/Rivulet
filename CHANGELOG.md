# Changelog

## [0.11.0-alpha.1] - 2026-08-17
- feat(gui): surface engine and capture errors in the UI
- feat(gui): abort a Windows recording with an error when no frame arrives within 5 s (pipeline never started)
- fix(gui): stop the disconnect probe from swallowing capture frames
- fix(gui): move AtomicBool/Ordering/Arc imports to unconditional scope
- fix(ci): add --entrypoint bash to ImageMagick docker run
- test(gui): add recording state and format_bytes tests
- docs: update README, CHANGELOG, and LINUX_BUILD

## [0.10.0-alpha.1] - 2026-08-17



- feat(updater): add download progress bar

- fix(gui): only stop recording when capture thread actually disconnects

- fix(gui): stop the disconnect probe from swallowing capture frames (no MP4 was written)

- feat(gui): surface engine and capture errors in the UI instead of only on the console
- fix(updater): show MSI installer UI instead of silent install

- fix(gui): enable eframe links feature for hyperlink_to



## [0.9.0-alpha.1] - 2026-08-17



- feat(gui): embed real Rivulet icon in the Windows executable



## [0.8.0-alpha.1] - 2026-08-17



- feat(audio): complete M1 audio module (filters + monitoring)

- ci(actions): add --json and --comment output to the pin checker

- ci(actions): bump pinned actions to current major versions

- ci(actions): separate same-major staleness from newer majors

- ci(actions): detect stale action pins outside Dependabot's schedule

- ci(actions): generate the action-pin table from the workflows

- ci(dependabot): auto-approve and auto-merge Dependabot PRs

- docs(ci): add central reference mapping action SHAs to upstream versions

- ci(dependabot): keep SHA-pinned GitHub Actions current

- ci(workflows): pin third-party actions to full commit SHAs

- ci(assets): pin the ImageMagick image by content digest

- chore(assets): add docker wrapper for regenerating assets

- ci(assets): pin asset generation to a fixed ImageMagick image

- ci(assets): verify generated assets match committed files

- ci(release): attach social preview images as release assets

- docs(assets): add 1200x630 OpenGraph fallback image

- docs(assets): add GitHub social preview (OpenGraph) image

- docs(assets): refine thumbnail contrast and colors

- chore(assets): add macOS app icon and wire it into the bundle

- chore(assets): generate thumbnail and icon reproducibly

- docs(assets): add app thumbnail and Linux AppImage icon

- test(ci): add shellcheck for macOS scripts and Pester tests for sign.ps1

- refactor(ci): extract build/package/sign steps into a reusable workflow

- ci: lint GitHub Actions workflows with actionlint

- ci(packaging): smoke-test code signing with self-signed certificates

- ci(packaging): complete code signing for Windows and macOS

- chore(core): translate remaining German log and error messages to English



## [0.7.0-alpha.1] - 2026-08-16



- feat(packaging): let users choose the Rivulet install directory in the MSI

- fix(updater): enable the ureq tls backend (rustls) for HTTPS requests

- fix(core): make recording metric tests deterministic with injectable clock

- fix(packaging): add Start Menu and Desktop shortcuts to the Windows MSI



## [0.6.0-alpha.1] - 2026-08-16



- feat(updater): add auto-update via GitHub Releases (check, download, install)

- chore: update copyright year in LICENSE



## [0.5.0-alpha.1] - 2026-08-16



- feat(core): track recording performance metrics (FPS, encoder load, file size)

- docs(readme): document M9 bot coexistence with the Vivid bot



## [0.4.0-alpha.1] - 2026-08-16



- feat(core): add i18n layer and switch project to English

- docs(readme): expand roadmap with OBS differentiation (M6-M8)



## [0.3.0-alpha.1] - 2026-08-16



- feat(core): hardware video encoding (NVENC/QuickSync/AMF) with auto-detection

- chore(deps): update eframe requirement from 0.35.0 to 0.36.1



## [0.2.4-alpha.1] - 2026-08-15



- fix(ci): declare MSI package as x64 platform (ICE80)



## [0.2.3-alpha.1] - 2026-08-15



- fix(ci): Win64 mark on harvested MSI components



## [0.2.2-alpha.1] - 2026-08-14



- fix(ci): 64-bit mark in MSI harvest (ICE80)

- chore(ci): fix GitHub Actions warnings



## [0.2.1-alpha.1] - 2026-08-14



- fix(ci): convert MSI version to numeric format



## [0.2.0-alpha.1] - 2026-08-14



- feat(ci): manual alpha release trigger via workflow_dispatch

- fix(ci): fix AppImage and MSI packaging



## [0.1.0-alpha.1] - 2026-08-14



- feat(ci): fix release versioning (first release + manifest update)



## [0.0.0-alpha.1] - 2026-08-14



- Initial release
