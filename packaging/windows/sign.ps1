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
# runners but signtool is not necessarily on PATH.
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
Write-Host "Using signtool: $signtoolPath"

$certPath = Join-Path $env:TEMP "rivulet-cert.pfx"
[IO.File]::WriteAllBytes($certPath, [Convert]::FromBase64String($certBase64))

try {
  foreach ($file in $Paths) {
    $resolved = (Resolve-Path $file).Path
    Write-Host "Signing: $resolved"
    & $signtoolPath sign /f $certPath /p $certPassword /fd SHA256 /tr "http://timestamp.digicert.com" /td SHA256 $resolved
    if ($LASTEXITCODE -ne 0) { throw "Signing failed: $resolved" }
  }
} finally {
  Remove-Item $certPath -Force -ErrorAction SilentlyContinue
}

Write-Host "Signing complete."
