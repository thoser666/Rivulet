# SignPath Foundation Application — ready-to-submit draft

Copy each field into the application form at <https://signpath.org/products/foundation>
(Foundation → *Apply*). Fields marked **[fill in]** need your personal data;
everything else is verified against the repo as of Sep 2026. Submitting grants
SignPath the usual Foundation terms: free OV-level Authenticode signing for
qualifying OSS, test certificate first, production certificate after a
build-system review.

---

## Project name

Rivulet

## Project description

> Rivulet is a free, open-source screen-recording and live-streaming
> application for Windows, macOS and Linux — a modern, privacy-friendly
> alternative to OBS Studio. It is written in Rust with a GStreamer-based
> pipeline engine, captures displays, windows and games, records to MP4/FLV
> (with a separate VOD audio track), streams to Twitch, YouTube and Kick
> simultaneously, and integrates Twitch/Kick/YouTube chat plus Discord Rich
> Presence into a single streaming workspace. It ships a portable ZIP, an
> MSI installer (Windows), a DMG (macOS) and an AppImage (Linux), all
> published as GitHub Releases together with a SHA256SUMS manifest and
> GPG signatures.

Keep it 2–3 sentences if the form limits length; the first sentence alone is
sufficient.

## Project URL(s)

- Repository: <https://github.com/thoser666/rivulet>
- Releases: <https://github.com/thoser666/rivulet/releases>
- Website/docs: [fill in if you have one; otherwise "README serves as documentation"]

## License

MIT License — <https://github.com/thoser666/rivulet/blob/develop/LICENSE>

## Primary language / platform

Rust (edition 2021). Desktop application for Windows, macOS, Linux.
Signed artifacts requested: **Windows** — `rivulet-gui.exe` (GUI),
`rivulet.exe` (CLI), `rivulet-updater.exe` (updater), plus the `.msi`
installer.

## Build system and its openness

> The entire build and release pipeline is public and reproducible from the
> repository. All binaries are produced by GitHub Actions workflows whose
> definitions live in the repo (`.github/workflows/`): CI runs the full test
> suite on every push, and the release workflow (`build-package.yml`,
> triggered via a reusable workflow from `ci.yml`) builds the Windows/macOS/
> Linux artifacts from source and publishes them to GitHub Releases. Nothing
> is built on private machines; no opaque pre-built blobs enter the
> pipeline. The only external inputs are pinned by SHA in
> `docs/ci-action-pins.md` (auto-generated and enforced by a test suite,
> `rivulet-core/tests/ci_pinning.rs`), and the GStreamer runtime is
> installed from pinned, self-mirrored MSI packages
> (`scripts/mirror-gstreamer-msi.sh`).

This is the part the build review cares most about — the honest summary is:
public CI, SHA-pinned actions, mirrored runtime installers, no human
touchpoints between source and artifact.

## Distribution

GitHub Releases (<https://github.com/thoser666/rivulet/releases>) with a
SHA256SUMS manifest per release; the updater verifies the checksum before
executing anything. Installers: MSI (Windows), DMG (macOS), AppImage
(Linux).

## Security posture (optional but strengthens the application)

- OpenSSF Best Practices badge — **passing** level achieved:
  <https://www.bestpractices.dev/projects/14447>
- Release workflow: automatic notes from commits, completeness-checked,
  SHA256SUMS attached
- Fuzzing targets for untrusted parsers (Twitch IRC, SDP, updater JSON)
  with a CI smoke run plus a scheduled weekly deep-fuzz run
- Static analysis (CodeQL) and dependency review gate every PR

## Do you already sign your releases?

No — Windows artifacts are currently unsigned; the signing automation is
already implemented and secret-gated in `build-package.yml`
(`signpath/github-action-submit-signing-request@v2.3`, pinned to SHA), so
signing activates automatically once the Foundation credentials are
configured. This is exactly what we are applying for.

## Contact

- GitHub username: thoser666
- Name: **[fill in]**
- Email: **[fill in]** — use an address you check; Foundation correspondence
  (test certificate, build review) goes here
- Preferred contact: GitHub issues / email

## After approval — what you'll configure (already documented)

`docs/code-signing.md` § "Windows — SignPath Foundation": create the
SignPath project + signing policy with an artifact configuration whose root
matches the workflow uploads (a `<zip-file>` root for the EXE bundle
containing the three EXEs, `<msi-file>` for the installer), create a CI API
token with *Submitter* permission, then set the four secrets
`SIGNPATH_API_TOKEN`, `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`,
`SIGNPATH_SIGNING_POLICY_SLUG`. The next release signs automatically.
