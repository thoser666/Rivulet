# Regression-Test Policy & Tracking

Rivulet treats a bug fix as incomplete until the fix is protected against
regression. This page is the project's regression-test policy and its tracking
mechanism.

## Policy

- **Every bug fix ships a regression test.** A change that fixes a reported bug
  (a `fix:` commit, an issue with a `bug` label, or a defect found during
  development) MUST add or update an automated test that fails on the unfixed
  code and passes with the fix.
- **Where the test lives.** The test goes into the suite that covers the
  changed code: unit tests in the affected `rivulet-*` crate, contract/content
  guards in `rivulet-core/tests/ci_pinning.rs`, GUI contract tests in
  `rivulet-gui/tests/ui_smoke.rs`, or an end-to-end/loopback smoke test under
  `scripts/`.
- **Exceptions.** If a regression test is genuinely infeasible (e.g., the bug
  requires a hardware codec, a live platform ingest, or a manual device that CI
  cannot reach), the PR must say so explicitly and name the follow-up tracking
  item instead of silently skipping the test.
- **Verification.** `fix:` commits are expected to touch a test file in the
  same change set. CI runs the full suites; a fix without a test is flagged in
  review (Definition of Done in
  [`CONTRIBUTING.md`](../CONTRIBUTING.md#definition-of-done-acceptable-contributions)).

## Tracking

Regression coverage is tracked in two places:

1. **Commit/PR trail.** Every merged fix that added a regression test is
   visible in git history (the fix commit plus its test). This is the primary,
   machine-verifiable record.
2. **Issue annotations.** Bug issues that are closed by a fix with a regression
   test carry the `regression-tested` label (or a comment linking the test),
   so the issue list itself shows coverage.

### Current status

The repository has no publicly reported *resolved vulnerabilities* in the last
12 months; the regression policy applies to all functional bug fixes. Recent
bug-fix PRs (reconnect backoff rebuild, screenshot redaction, Windows recorder
finalize, obs-websocket auth close-code, YouTube/Kick/Twitch chat smoke) each
shipped an accompanying regression test in the same change set. As of the M8
stability work the suite also covers the safeguarded Python sanity-check wrapper
(`check_sanity.py`) and the starter auditable command wrapper (`scripts/run.cmd`):
see `rivulet-core/tests/check_sanity.rs` and `rivulet-core/tests/run_cmd.rs`.

The same file also guards the M8 delivery claims: `ci_pinning` pins the seven
`rivulet-core/examples/` files against three surfaces at once — the `ci.yml`
step that **runs** them (M8 #293: an example that only compiles is not
delivered), the `Examples` table in the crate docs, and the README M8
overview. The example list is read from `rivulet-core/examples/`, so an
eighth example fails the guard until CI and both docs mention it; the names
are matched as whole tokens so renaming an example in the workflow cannot
pass on a substring.

## Why this exists (OpenSSF Silver)

The OpenSSF Best Practices **silver** badge criterion `regression_tests_added50`
requires that "the project add regression tests to an automated test suite for
at least 50% of the bugs fixed within the last six months." This policy is the
project's answer to that criterion: by requiring a regression test on every fix,
coverage is at or near 100% by construction instead of being measured
retroactively.
