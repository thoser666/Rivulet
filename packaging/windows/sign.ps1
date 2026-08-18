# Signs one or more files with the Rivulet code signing certificate via
# signtool (Windows SDK).
#
# Usage: pwsh packaging/windows/sign.ps1 -Paths @("path1.exe","path2.msi")
#
# Requires the following environment variables:
#   WINDOWS_CERT_BASE64   Base64-encoded .pfx code signing certificate
#   WINDOWS_CERT_PASSWORD Password of the .pfx archive
param(
  [Parameter(Mandatory=$true)][string[]]$Paths
)

$ErrorActionPreference = "Stop"

$certBase64 = $env:WINDOWS_CERT_BASE64
$certPassword = $env:WINDOWS_CERT_PASSWORD
if (-not $certBase64) { throw "WINDOWS_CERT_BASE64 not set" }
if (-not $certPassword) { throw "WINDOWS_CERT_PASSWORD not set" }

# Locate signtool.exe. The Windows SDK is preinstalled on the GitHub Windows
# runners but signtool is not necessarily on PATH. A recursive scan over all
# SDK versions is very slow on CI, so probe the standard layout directly;
# SIGNTOOL_PATH overrides everything.
$signtoolPath = $env:SIGNTOOL_PATH
if (-not $signtoolPath) {
  $signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
  if ($signtool) {
    $signtoolPath = $signtool.Source
  } else {
    $kitRoot = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) "Windows Kits\10\bin"
    $signtoolPath = Get-ChildItem -Path $kitRoot -Directory -ErrorAction SilentlyContinue |
      Sort-Object Name -Descending |
      ForEach-Object { Join-Path $_.FullName "x64\signtool.exe" } |
      Where-Object { Test-Path $_ } |
      Select-Object -First 1
  }
}
if (-not $signtoolPath) { throw "signtool.exe not found - install the Windows SDK or set SIGNTOOL_PATH" }
Write-Host "Using signtool: $signtoolPath"

$certPath = Join-Path $env:TEMP "rivulet-cert.pfx"
[IO.File]::WriteAllBytes($certPath, [Convert]::FromBase64String($certBase64))

# WINDOWS_TIMESTAMP_URL overrides the RFC3161 timestamp server. Set it to
# "off" to skip timestamping (the smoke test does this to stay offline).
$timestampUrl = $env:WINDOWS_TIMESTAMP_URL
if ($null -eq $timestampUrl) { $timestampUrl = "http://timestamp.digicert.com" }

try {
  foreach ($file in $Paths) {
    $resolved = (Resolve-Path $file).Path
    Write-Host "Signing: $resolved"
    if ($timestampUrl -eq "off") {
      & $signtoolPath sign /f $certPath /p $certPassword /fd SHA256 $resolved
    } else {
      & $signtoolPath sign /f $certPath /p $certPassword /fd SHA256 /tr $timestampUrl /td SHA256 $resolved
    }
    if ($LASTEXITCODE -ne 0) { throw "Signing failed: $resolved" }
  }
} finally {
  Remove-Item $certPath -Force -ErrorAction SilentlyContinue
}

Write-Host "Signing complete."
