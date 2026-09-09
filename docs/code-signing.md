# Code Signing

Release packages are signed automatically **when the matching secrets are
configured**; without secrets, unsigned packages are built (the default, so
forks and development builds keep working). Signing never fails a release —
it is a best-effort enrichment of the alpha channel and a hard requirement
of the [Beta-Gate](../README.md#beta-gate) (criterion 4) for beta/RC/stable.

| Platform | Artifact | Script / Action | Secrets |
|---|---|---|---|
| Windows (PFX) | `rivulet-gui.exe`, `rivulet.exe`, `rivulet-updater.exe`, `.msi` | `packaging/windows/sign.ps1` (signtool) | `WINDOWS_CERT_BASE64`, `WINDOWS_CERT_PASSWORD` |
| Windows (SignPath) | `rivulet-gui.exe`, `rivulet.exe`, `rivulet-updater.exe`, `.msi` | `signpath/github-action-submit-signing-request@v2.3` (file-based) | `SIGNPATH_API_TOKEN`, `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG` |
| macOS | `.app` (hardened runtime), `.dmg` (notarized + stapled) | `packaging/macos/codesign-app.sh`, `packaging/macos/sign-notarize.sh` | `MACOS_CERT_BASE64`, `MACOS_CERT_PASSWORD`, `APPLE_ID`, `APPLE_APP_PASSWORD`, `APPLE_TEAM_ID` |
| Linux | `.AppImage` (detached `.asc` signature) | `packaging/linux/sign-gpg.sh` (GPG) | `LINUX_GPG_PRIVATE_KEY` (optional `LINUX_GPG_PASSPHRASE`) |

**Windows precedence:** when the four SignPath secrets are configured, the
SignPath Foundation path is used and the PFX path is skipped (they are
mutually exclusive). Otherwise, when the two PFX secrets are configured,
signtool signs locally. With neither, Windows artifacts stay unsigned.

## Free signing options for open source

Not every platform has a free route — macOS deliberately has none. The
honest comparison (prices verified September 2026):

| Option | Platform | Cost | Certificate level | Caveats |
|---|---|---|---|---|
| **SignPath Foundation** | Windows | **Free** for qualifying OSS | OV Authenticode | Application + build review required; signing is file-based via their API (the free tier does not expose hash-based Crypto Providers — that is the paid Code Signing Gateway); SmartScreen reputation still needs download volume; certificate stays in SignPath's HSM |
| **Azure Artifact Signing** (formerly Trusted Signing) | Windows | **$9.99/month** Basic (no free tier; ~$200 one-time Azure trial credit is a trial, not permanent) | OV (non-EV) | EV restricted to businesses registered 3+ years in US/CA/EU; identity service managed by Microsoft |
| **Purchased OV certificate** (Sectigo, Certum, …) | Windows | ≈ **$100–200/year** | OV | SmartScreen reputation builds over downloads; may require re-validation per year |
| **Purchased EV certificate** (DigiCert, GlobalSign, …) | Windows | ≈ **$300/year** | EV | Immediate SmartScreen reputation; requires hardware token or cloud HSM, which complicates CI |
| **Self-signed certificate** | Windows | Free | none (untrusted) | No SmartScreen improvement — every user sees "unknown publisher"; useful only for local smoke tests (this is what `signing-e2e.yml` uses) |
| **Apple Developer Program** | macOS | **$99/year** — no free route | Developer ID | Gatekeeper blocks unsigned apps entirely; notarization is tied to the paid membership. There is no self-signed workaround that users can open |
| **GPG key pair** | Linux | **Free** | OpenPGP | Already implemented (`sign-gpg.sh`); generate a key, set one secret, done |

**Recommendation for Rivulet:** apply at **SignPath Foundation** (free,
OV-level, Microsoft-documented for open source) for Windows, keep macOS on
the paid Apple Developer Program when beta approaches (unavoidable platform
tax), and stay on the existing free GPG signing for Linux.

**SignPath artifacts:** `rivulet-windows-unsigned-exe-<run_id>` (ZIP with
the three executables) and `rivulet-windows-unsigned-msi-<run_id>` — the
signed files are downloaded back into `staging/` and shipped as usual.

#### What a maintainer must set up (SignPath Foundation)

1. Apply at [signpath.org](https://signpath.org) (open-source project,
   public repository, open build system). Approved projects start with a
   test certificate and receive a production certificate after a build
   review.
2. In the SignPath portal: create a **project** (slug →
   `SIGNPATH_PROJECT_SLUG`) with an **artifact configuration** matching the
   uploaded ZIP layout, and a **signing policy** (slug →
   `SIGNPATH_SIGNING_POLICY_SLUG`).
3. Create an **API token** for a CI user with *Submitter* permission (→
   `SIGNPATH_API_TOKEN`) and note the **organization ID** (→
   `SIGNPATH_ORGANIZATION_ID`).
4. Add the four secrets under **Settings → Secrets and variables →
   Actions**. The workflow activates the SignPath path only when all four
   are present.
5. Recommended: install the **SignPath GitHub App** and allow repository
   access so SignPath can verify workflow provenance (needed for the
   production certificate review).

Users can verify a signed release the usual way: Windows shows the publisher
in the file properties; `Get-AuthenticodeSignature <file>` in PowerShell
prints `Valid`.

The automation itself is smoke-tested on every push **without any secrets**
(`.github/workflows/signing-e2e.yml`): Windows via a committed self-signed
certificate + Pester, macOS via a freshly generated self-signed identity,
Linux via a throwaway GPG key. This proves the scripts work before real
certificates exist.

---

## What a maintainer must set up

All secrets live under **Settings → Secrets and variables → Actions** of the
repository (or the organization, with `Actions` access). `scripts/check-beta-gate.py`
reports exactly which secrets are still missing (criterion 4 of the Beta-Gate).

### Windows — Authenticode (`WINDOWS_CERT_*`)

1. **Obtain a certificate.** Buy a code-signing certificate from an issuer
   (DigiCert, Sectigo, GlobalSign, …). An **EV certificate** additionally
   removes the “unknown publisher” warning for most users; a standard
   certificate removes it after enough reputation is built. For a first
   smoke test, a self-signed certificate works locally (Windows will show
   “unknown publisher” — expected):
   ```powershell
   New-SelfSignedCertificate -Type CodeSigningCert -Subject "CN=Rivulet" `
     -CertStoreLocation Cert:\CurrentUser\My
   ```
2. **Export to `.pfx`** (the export password becomes `WINDOWS_CERT_PASSWORD`):
   ```powershell
   $thumb = (Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert).Thumbprint
   $pwd = Read-Host "PFX password" -AsSecureString
   Export-PfxCertificate -Cert "Cert:\CurrentUser\My\$thumb" `
     -FilePath cert.pfx -Password $pwd
   ```
3. **Base64-encode** into a single line:
   ```powershell
   [Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))
   ```
   (or `certutil -encode cert.pfx cert.txt` and paste the body without the
   `-----BEGIN/END CERTIFICATE-----` lines).
4. **Create the secrets** `WINDOWS_CERT_BASE64` + `WINDOWS_CERT_PASSWORD`.
   `sign.ps1` decodes the blob, signs the three executables **before**
   packaging (so the portable ZIP and MSI embed signed binaries) and signs
   the MSI afterwards. Timestamping uses DigiCert by default;
   `WINDOWS_TIMESTAMP_URL` overrides it (`off` disables it).

### Windows — SignPath Foundation (free for open source, `SIGNPATH_*`)

SignPath Foundation (signpath.org) gives qualifying open-source projects a
**free OV-level Authenticode certificate**. The certificate lives in
SignPath's HSM and never leaves their vault — the build workflow submits a
*signing request* and downloads the signed artifact. This is the preferred
Windows path when configured (it takes precedence over the PFX path).

**Hash-based vs. file-based — honest distinction.** Rivulet uses SignPath's
**file-based** signing request flow (the official
`signpath/github-action-submit-signing-request@v2.3` action): the unsigned
artifacts are uploaded, SignPath signs them, and the signed artifacts are
downloaded back. SignPath's true **hash-based** signing (private key stays
in the HSM while only digests cross the wire, via the Crypto Providers
KSP/Cryptoki) is part of the paid **Code Signing Gateway** product tier;
the free Foundation program does not expose it. In both cases the private
key never leaves SignPath's HSM — the security property is the same, only
the transport differs.

1. **Apply** at signpath.org for the Foundation program (open-source
   project, public repository, open build system). Approved projects get a
test certificate first and a production certificate after a build
   review. The application text lives in
   [signpath-application-draft.md](signpath-application-draft.md) — it was
   **submitted in September 2026**; the doc tracks the current application
   status.
2. **Create the SignPath project and signing policy** in the SignPath
   portal. The project needs an **artifact configuration** matching what
   the workflow uploads: the EXEs are uploaded as a GitHub artifact (a
   ZIP containing `rivulet-gui.exe`, `rivulet.exe`, `rivulet-updater.exe`)
   and the MSI as a separate artifact. The paste-in XML for this lives in
   [packaging/signpath/artifact-configuration.xml](../packaging/signpath/artifact-configuration.xml)
   and is kept in sync with the workflow by a CI pinning test. Portal
   steps: select the project → **Artifact Configurations** → **Add** →
   **Custom** → paste the XML → save. It defines both request shapes (a
   `<zip-file>` root signing the three EXEs, an `<msi-file>` root signing
   the installer as a whole) and the required `version` parameter the
   workflow passes on every submit. Because the workflow's submit steps
   do not pin an `artifact-configuration-slug`, SignPath selects this
   configuration automatically by matching the root element to the
   uploaded artifact. Then note the **project slug** and **signing policy
   slug**.
3. **Create an API token** for a CI user with *Submitter* permission on the
   signing policy; note it as the API token.
4. **Create the four secrets**: `SIGNPATH_API_TOKEN`, `SIGNPATH_ORGANIZATION_ID`,
   `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`. The workflow
   only activates the SignPath path when **all four** are present.
5. Optional: install the **SignPath GitHub App** and allow access to the
   repository so SignPath can verify the workflow provenance and the
   artifact's origin (recommended for the production certificate). No
   extra token permission is needed: `build-package.yml` deliberately
   does not set `permissions.actions: read` (GitHub rejects it in
   reusable workflows at startup, and on public repositories the default
   token can download artifacts without it).

When both the SignPath and PFX secret groups are configured, SignPath wins
(see the precedence note above).

### macOS — Developer ID + notarization (`MACOS_CERT_*` + `APPLE_*`)

1. **Obtain a certificate.** developer.apple.com → Certificates → create a
   *Developer ID Application* certificate, download and install it into the
   Keychain. This requires the **Apple Developer Program** ($99/year).
2. **Export to `.p12`**: Keychain Access → right-click the certificate →
   *Export…* → *Personal Information Exchange (.p12)*. The export password
   becomes `MACOS_CERT_PASSWORD`.
3. **Base64-encode** into a single line: `base64 -i cert.p12`.
4. **Create the secrets** `MACOS_CERT_BASE64` + `MACOS_CERT_PASSWORD`.
   `codesign-app.sh` imports the `.p12` into a temporary keychain and signs
   the app bundle with the hardened runtime.
5. **Notarization credentials** — create an *app-specific password*
   (appleid.apple.com → Sign-In & Security → App-Specific Passwords):
   - `APPLE_ID` — your Apple ID (sign-in email).
   - `APPLE_APP_PASSWORD` — the app-specific password (never your Apple ID
     password).
   - `APPLE_TEAM_ID` — your Team ID (developer.apple.com → Membership).
   `sign-notarize.sh` then notarizes and staples the DMG via `notarytool`.
6. *(Optional)* if your identity is not `Developer ID Application`, set
   `MACOS_SIGN_IDENTITY` accordingly.

### Linux — GPG (`LINUX_GPG_PRIVATE_KEY`)

Distributions and package managers expect a detached GPG signature next to
the AppImage. The workflow produces `rivulet-linux-x86_64.AppImage.asc`.

1. **Create a signing key** (once, on a trusted machine):
   ```bash
   gpg --full-generate-key   # RSA 4096 or Ed25519, no expiry recommended
   ```
2. **Export the private key** ASCII-armored:
   ```bash
   gpg --armor --export-secret-key <key-id>
   ```
   The output starts with `-----BEGIN PGP PRIVATE KEY BLOCK-----`.
3. **Create the secret** `LINUX_GPG_PRIVATE_KEY` — paste the armored block
   (multi-line secrets paste fine), **or** base64-encode it first
   (`base64 -w0 key.asc`) if you prefer a single-line value. If the key has a
   passphrase, also set `LINUX_GPG_PASSPHRASE`.
4. Users verify with:
   ```bash
   gpg --verify rivulet-linux-x86_64.AppImage.asc rivulet-linux-x86_64.AppImage
   ```

---

## Verifying

- `gh secret list` shows which names are configured.
- `scripts/check-beta-gate.py` (with a token that can read secrets) reports
  which secrets are missing — criterion 4 of the Beta-Gate. Windows is
  satisfied by **either** the PFX pair **or** the SignPath set.
- `python3 scripts/test-signpath-config.py` validates the SignPath wiring in
  `build-package.yml` (secrets, action pin, upload → submit → copy-back
  round trips, precedence); `--self-test` verifies the checker itself.
- With all secrets present, the next release build signs the packages; the
  signed artifacts are listed in the release’s `SHA256SUMS` manifest.

## Troubleshooting

- **`signtool.exe not found`** — the Windows SDK is not on PATH. Set
  `SIGNTOOL_PATH` (a secret or environment variable) to the full path, or
  install the Windows SDK on the runner.
- **macOS “no identity found”** — the imported certificate’s name does not
  match `MACOS_SIGN_IDENTITY`; set it to the certificate’s common name, or
  let the script fall back to the first codesigning identity.
- **macOS “MAC verification failed”** on `security import` — the `.p12` was
  exported with modern defaults; re-export with *legacy* format, or import
  the `.cer`/`.pem` pair instead.
- **GPG signature does not verify** — the key was re-created after the
  release was built; export and re-set `LINUX_GPG_PRIVATE_KEY` and rebuild.
- **Secrets are set but the release is still unsigned** — every platform
  requires its **full** secret set (see the table); a single missing secret
  disables that platform’s signing for that release.
- **SignPath signing request fails** — check the four secrets are all set,
  the API token has *Submitter* permission on the signing policy, and the
  uploaded artifact matches the
  [artifact configuration](../packaging/signpath/artifact-configuration.xml)
  (EXE ZIP with exactly the three executables, or the MSI). Rejections
  like *unknown parameter* mean the `version` parameter is not declared
  in the portal copy of the XML — re-paste the file. The action logs the
  signing-request URL; open it in the SignPath portal for the exact
  rejection reason.