# Security Policy for Rivulet

Rivulet is a screen recorder and live-streaming application. This policy covers
responsible disclosure of security vulnerabilities, supported release channels,
and the project's security controls.

## Supported Versions

Rivulet is distributed through GitHub Releases. The canonical (signed, stable)
build is the most recent **stable** or **beta** release. Alpha builds track every
feature push and are intended for testing, not security-sensitive deployments.

| Channel | Tag pattern | Support |
| --- | --- | --- |
| Stable / Beta | `vX.Y.Z` / `vX.Y.Z-beta.N` | Full security support |
| Alpha | `vX.Y.Z-alpha.N` | Best-effort; fixes land in the next release |

If you are running an alpha build, upgrade to the latest release before treating
an issue as a supported vulnerability.

## Reporting a Vulnerability

Please **do not open a public GitHub issue** for a security vulnerability. Use a
private channel:

- Private disclosure via a security advisory:
  https://github.com/thoser666/Rivulet/security/advisories/new
- For maintainers, the repository's `docs/security.md` incident-response
  procedure applies; it also records how incidents are handled after triage.

Include, if possible:

1. A short description of the issue and its impact.
2. The affected version(s) and platform(s) (Windows / Linux / macOS).
3. Steps to reproduce.
4. Whether it is triggered remotely or only by a trusted local user/input.

Stream keys, tokens, and personal logs must not appear in the report; scrub them
first. Never paste a live stream key into an issue, log, screenshot, or advisory.

## Scope

The following are **in scope** for this policy:

- Memory-safety and correctness bugs in the capture, encoding, audio, streaming,
  update, and launcher components that can lead to code execution, privilege
  escalation, or data loss.
- Unsafe deserialization or parsing of crafted input (media files, SDP, URLs,
  configuration, plugin payloads).
- Remote streaming/network transport issues (RTMP/RTMPS, SRT/RIST, WHIP/WebRTC).
- Supply-chain and CI integrity issues (action pinning, package signing,
  dependency poisoning).

The following are **out of scope** / expected limitations:

- Bugs that require the attacker to already have local code execution.
- Missing native-feature parity that is an advertised roadmap item (for example
  macOS capture, native browser adapters, per-track VOD routing).
- Missing OS-backed credential storage, which is a documented follow-up in
  [`docs/m3-streaming-completion-report.md`](docs/m3-streaming-completion-report.md).

## Security Controls

Rivulet uses layered GitHub-native and CI controls. Details and the reproducibility
commands are documented in [`docs/security.md`](docs/security.md); the highlights:

- Secret Scanning and Push Protection are enabled in the repository settings.
- Dependabot Security Updates and Renovate keep Cargo dependencies and GitHub
  Actions up to date.
- Third-party GitHub Actions are pinned to full commit SHAs and verified by CI
  (`rivulet-core/tests/ci_pinning.rs`).
- CodeQL / GitHub code scanning, Dependabot alerts, and the OpenSSF Scorecard run
  on the repository.
- Release artifacts are built in CI; code signing is activated when the signing
  secrets are configured.

## Incident Handling and Timeline

Reporters will receive an acknowledgement, a triage decision, and a fix target.
For in-scope vulnerabilities we aim to:

1. Acknowledge within **5 business days**.
2. Provide an initial triage within **2 weeks**.
3. Coordinate disclosure with the reporter before releasing a public fix.

Security-sensitive content assembled for the fix is handled per the incident
procedure in [`docs/security.md`](docs/security.md).

## Coordinated Disclosure

We follow coordinated disclosure. Please allow us time to fix and release before
publicly disclosing the vulnerability. In return, we credit you in the release
notes or advisory when you request it and share the fix details.