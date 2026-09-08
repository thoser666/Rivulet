# Code Signing

Release packages are signed automatically **when the matching secrets are
configured**; without secrets, unsigned packages are built (the default, so
forks and development builds keep working). Signing never fails a release —
it is a best-effort enrichment of the alpha channel and a hard requirement
of the [Beta-Gate](../README.md#beta-gate) (criterion 4) for beta/RC/stable.

| Platform | Artifact | Script | Secrets |
|---|---|---|---|
| Windows | `rivulet-gui.exe`, `rivulet.exe`, `rivulet-updater.exe`, `.msi` | `packaging/windows/sign.ps1` (signtool) | `WINDOWS_CERT_BASE64`, `WINDOWS_CERT_PASSWORD` |
| macOS | `.app` (hardened runtime), `.dmg` (notarized + stapled) | `packaging/macos/codesign-app.sh`, `packaging/macos/sign-notarize.sh` | `MACOS_CERT_BASE64`, `MACOS_CERT_PASSWORD`, `APPLE_ID`, `APPLE_APP_PASSWORD`, `APPLE_TEAM_ID` |
| Linux | `.AppImage` (detached `.asc` signature) | `packaging/linux/sign-gpg.sh` (GPG) | `LINUX_GPG_PRIVATE_KEY` (optional `LINUX_GPG_PASSPHRASE`) |

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
  which secrets are missing — criterion 4 of the Beta-Gate.
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