# Rivulet Assurance Case

This document is the security **assurance case** for Rivulet: it argues why the
project's security requirements are met, based on the threat model and
hardening documented in [`docs/security.md`](../security.md). It satisfies the
OpenSSF Best Practices silver criterion `assurance_case`.

## 1. Security requirements

From [`docs/security.md`](../security.md) and [`SECURITY.md`](../../SECURITY.md):

- Untrusted input from external parties must not corrupt the capture,
  encoding, or shared-memory pipelines.
- Secrets (stream keys, OAuth tokens, signing certificates, Discord client
  ID) must never be logged, leaked into screenshots, or shipped in artifacts.
- The supply chain must be defensible: pinned CI actions, pinned toolchain,
  dependency monitoring, SHA256SUMS-verified updates.
- Releases must be verifiable and the update path must fail closed on
  tampered artifacts.

## 2. Threat model

The threat model is documented in [`docs/security.md`](../security.md) (see the
"Threat model" section). The principal adversaries:

- **Malicious or compromised input sources** — IRC chat lines, SDP/RTMP
  session data, updater manifests/JSON, shared-memory frame headers written by
  injected hook DLLs (Vulkan/OpenGL/DXGI), OBS WebSocket requests.
- **Local rogue processes** — anything that can write to the shared-memory
  segment or read logs.
- **Supply-chain attackers** — dependency compromise, malicious CI actions,
  tampered release artifacts.
- **Network attackers** — TLS interception of chat/update/Discord traffic.

## 3. Trust boundaries

- **Boundary 1: untrusted parsers.** Chat protocols (Twitch/Kick/YouTube),
  SDP, updater JSON, OBS WebSocket v5 payloads, and shared-memory headers are
  all parsed at a defined boundary. Code inside the boundary must treat all
  data as hostile (see §4).
- **Boundary 2: shared-memory capture.** The capture channel defined in
  `rivulet-core/src/capture_channel.rs` transfers frame data from injected
  hooks into the Rivulet process. Frame headers carry size/sequence/format
  fields that are validated before use; the segment is not trusted for
  lengths or pointers.
- **Boundary 3: the network.** All network TLS (chat, update, Discord IPC)
  goes through rustls; no Rivulet-owned certificate logic exists.
- **Boundary 4: release artifacts.** Users receive artifacts from GitHub
  Releases over HTTPS; integrity is pinned by the SHA256SUMS manifest the
  updater verifies before install.

## 4. Secure-design argument

- **Principle of least privilege** is applied to CI: workflows request the
  minimal GitHub token scopes, signing/stream keys live in secrets and are
  never compiled into binaries (see [`docs/security.md`](../security.md) and
  [`docs/ci-action-pins.md`](../ci-action-pins.md)).
- **Fail-closed updates.** The updater verifies SHA256SUMS before applying an
  update; a missing or mismatching manifest aborts the update.
- **Secrets hygiene** is enforced by contract tests (the screenshot test
  redacts `stream_key`), DCO sign-offs, and the secret-detection CI jobs.
- **Memory safety by construction.** The project is Rust; the only unsafe code
  is confined to the capture/hook modules where it is necessary, and those
  regions are scrutinized in review and covered by fuzzing targets.
- **Formal validation at trust boundaries.** Untrusted parsers reject invalid
  input instead of tolerating it (allowlist style), and are fuzzed by the
  cargo-fuzz targets exercised in CI and the weekly deep campaign.

## 5. Common implementation weaknesses countered

| Weakness | Mitigation |
|---|---|
| Buffer overflows / use-after-free | Rust memory safety; fuzz targets (`fuzz/fuzz_targets/`) with ASan-style coverage in CI (`scripts/fuzz-smoke.sh`) |
| Injection (IRC command/format string) | IRC line parsing is tested and fuzzed; chat input is validated before sending |
| Integer/ratchet errors in SDP/session data | SDP parser is fuzzed; session state is validated in `stream_runtime` |
| Path traversal / bad file names | File-management and recording-split logic validates generated paths (see `rivulet-core/src/file_management.rs`, `container.rs`) |
| Secret leakage | GUI screenshot contract test redacts secrets; DIAGNOSTICS scrubbing; secrets stored in config, never logs |
| Tampered artifacts | SHA256SUMS manifest generated at release time and verified by the updater (fail-closed) |
| Weak crypto defaults | No own crypto; TLS via rustls; see crypto claims in the OpenSSF entry |

## 6. Residual risks

- **Self-signed / CI-generated codesigning** is exercised but production
  artifacts are not yet signed with a user-verifiable key chain (`signed_releases`
  remains open in the OpenSSF entry).
- **Single maintainer** reduces review depth (bus factor 1); the governance
  contingency in [`GOVERNANCE.md`](../../GOVERNANCE.md) is the mitigation.
- Statement coverage (`test_statement_coverage80`) is gated at ≥ 80 % for
  `rivulet-core` (`scripts/coverage-gate.sh`, CI `coverage` job); platform
  hook crates are not covered headlessly.

This assurance case is maintained alongside `docs/security.md`; the guard
`ossf_silver_gap_closures_are_pinned` in `rivulet-core/tests/ci_pinning.rs`
pins this document and the coverage gate to the tree.