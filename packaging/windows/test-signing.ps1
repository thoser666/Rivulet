# End-to-end smoke test for the Windows code signing step using a self-signed
# certificate. Signs a copy of cmd.exe (a valid PE) through the production
# sign.ps1 script and verifies the resulting Authenticode signature.
#
# Usage: pwsh packaging/windows/test-signing.ps1
$ErrorActionPreference = "Stop"

$temp = Join-Path $env:RUNNER_TEMP "rivulet-signing-test"
New-Item -ItemType Directory -Force -Path $temp | Out-Null

$pfxPath = Join-Path $temp "test-cert.pfx"
$cerPath = Join-Path $temp "test-cert.cer"
$testExe = Join-Path $temp "test.exe"

# 1. Self-signed code signing certificate (current-user store, no admin).
$cert = New-SelfSignedCertificate `
  -Subject "CN=Rivulet CI Test" `
  -Type CodeSigningCert `
  -CertStoreLocation Cert:\CurrentUser\My `
  -KeyExportPolicy Exportable `
  -KeyAlgorithm RSA -KeyLength 2048 `
  -NotAfter (Get-Date).AddDays(1) `
  -HashAlgorithm SHA256

$password = ConvertTo-SecureString -String "rivulet-test" -Force -AsPlainText
Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $password | Out-Null

# Trust the certificate in the current-user root store so that
# `signtool verify /pa` can validate the chain without elevation.
Export-Certificate -Cert $cert -FilePath $cerPath | Out-Null
Import-Certificate -FilePath $cerPath -CertStoreLocation Cert:\CurrentUser\Root | Out-Null

# 2. A real PE file to sign (signtool refuses non-PE payloads).
Copy-Item "$env:SystemRoot\System32\cmd.exe" $testExe -Force

# 3. Sign through the production signing path.
$env:WINDOWS_CERT_BASE64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($pfxPath))
$env:WINDOWS_CERT_PASSWORD = "rivulet-test"
pwsh packaging/windows/sign.ps1 -Paths $testExe

# 4. Verify the Authenticode signature.
$signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
if ($signtool) {
  $signtoolPath = $signtool.Source
} else {
  $kitRoot = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) "Windows Kits\10\bin"
  $signtoolPath = Get-ChildItem -Path $kitRoot -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
  if (-not $signtoolPath) { throw "signtool.exe not found - install the Windows SDK" }
}

& $signtoolPath verify /pa /v $testExe
if ($LASTEXITCODE -ne 0) { throw "Signature verification failed" }

Write-Host "Windows signing smoke test passed."
