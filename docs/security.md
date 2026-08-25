# GitHub Security Controls

Rivulet uses GitHub-native security controls plus repository-owned CI checks. The
controls are intentionally layered: repository settings prevent credential leaks,
while workflows scan source and dependency changes.

## Enabled Repository Features

The following settings are enabled for `thoser666/Rivulet`:

- **Secret Scanning** detects credentials in the repository and its history.
- **Push Protection** blocks supported secrets before they are pushed.
- **Dependabot Security Updates** is enabled for vulnerable dependency updates.

Secret Scanning and Push Protection are repository settings, not workflow files.
They must therefore be checked through the GitHub UI or API after repository
migration, fork creation, or permission changes:

```bash
gh api repos/thoser666/rivulet \
  --jq '.security_and_analysis | {
    secret_scanning: .secret_scanning.status,
    push_protection: .secret_scanning_push_protection.status,
    dependabot_updates: .dependabot_security_updates.status
  }'
```

Expected output:

```json
{"secret_scanning":"enabled","push_protection":"enabled","dependabot_updates":"enabled"}
```

The API command reports configuration state only. It does not expose secret
values. A token with repository administration permission is required to change
these settings.

## CI Checks

`.github/workflows/security.yml` runs:

- **CodeQL** for Rust on pushes to `develop`, pull requests, weekly scheduled
  scans, and manual dispatch. SARIF results are uploaded to GitHub Code
  Scanning.
- **Dependency Review** on pull requests. The check fails for high-severity or
  critical dependency advisories and posts a summary to the pull request.
- **OpenSSF Scorecard** on pushes to `develop`, pull requests, weekly scheduled
  scans, and manual dispatch. The workflow publishes the result to the OpenSSF
  API, stores the SARIF artifact for 14 days, and uploads a separate
  `openssf-scorecard` category to GitHub Code Scanning.

Every third-party action in the security workflows is pinned to a full commit
SHA. The pins are listed in [`ci-action-pins.md`](ci-action-pins.md) and enforced
by `rivulet-core/tests/ci_pinning.rs`.

Scorecard requires `id-token: write` for signed, repository-bound publication.
The workflow sets `persist-credentials: false` during checkout and does not
request write access to repository contents. Review the first score regularly;
Scorecard findings are improvement signals, not a substitute for threat
modeling or maintainer review.

## Required Repository Settings

Configure these settings in **Settings -> Rules -> Rulesets** or the equivalent
branch protection UI for `develop`:

- Require the normal CI checks and the `Security` checks before merging.
- Require CodeQL alerts to be resolved when CodeQL is made a blocking check.
- Keep pull-request approvals enabled for changes to workflows and packaging.
- Keep Actions permissions restricted to the minimum required by each workflow.

The first CodeQL runs should be reviewed before making alerts a merge blocker.
False positives and generated/native integration code should be triaged and
tracked rather than silently dismissed.

## Secret Handling

- Keep signing certificates, Apple credentials, and stream keys in GitHub
  Actions Secrets or environment-specific secret stores.
- Never place secrets in `README.md`, fixtures, test output, release assets, or
  issue comments.
- Remove personal paths and tokens from logs before attaching them to issues.
- Rotate a credential immediately if Push Protection reports it or if it appears
  in a commit, even when the commit is later removed.

Push Protection does not replace credential rotation, least-privilege tokens, or
review of external release assets. It also cannot detect every proprietary or
newly issued credential format.

## Incident Response

1. Stop the affected workflow or release if a credential may be exposed.
2. Revoke and rotate the credential at its provider.
3. Inspect the alert under **Security -> Secret scanning** and identify every
   affected commit, branch, artifact, and log.
4. Remove the value from the repository history where appropriate, but do not
   treat history rewriting as a substitute for rotation.
5. Record the remediation in the related issue without copying the secret.

For dependency findings, review the advisory, affected transitive path, and
available upgrade. If an upgrade is not immediately possible, document the
scope, mitigation, and owner before accepting a temporary exception.
