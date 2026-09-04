## What does this change?

<!-- One or two sentences: what and why. Link the issue/roadmap item when
     applicable (e.g. "Closes #123" or "M4 roadmap — file splitting"). -->

## Definition of Done

Check each item that applies (see the
[Definition of Done](https://github.com/thoser666/Rivulet/blob/develop/CONTRIBUTING.md#definition-of-done-acceptable-contributions)):

- [ ] **Tests** — new functionality adds automated tests; **bug fixes add a
      regression test** that fails on the unfixed code (policy:
      [docs/regression-testing.md](https://github.com/thoser666/Rivulet/blob/develop/docs/regression-testing.md)).
      Pure wiring/automation changes pin the contract in
      `rivulet-core/tests/ci_pinning.rs` instead.
- [ ] **i18n** — every new user-visible string exists in the EN **and** DE
      locale tables (`rivulet-core/src/i18n.rs`).
- [ ] **Docs** — user-visible behavior is reflected in `README.md`, the
      relevant `docs/*.md` page, and `CHANGELOG.md`.
- [ ] **Checks green** — `cargo fmt --all --check`, `cargo clippy --workspace
      --all-targets -- -D warnings`, and the test suites pass.
- [ ] **Conventional commit** — the squash commit uses the Conventional Commits
      format; a `fix(security):` closing a public vulnerability references its
      CVE/GHSA/RUSTSEC in the subject.
- [ ] **No secrets** — stream keys, tokens, certificates, and personal paths
      never appear in commits, logs, docs, or this description.

## Developer Certificate of Origin

All commits in this pull request carry a `Signed-off-by` trailer matching the
commit author (DCO, <https://developercertificate.org>), enforced by
`scripts/check-dco.py` in CI.
