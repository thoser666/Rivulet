# Generates and validates the Rivulet WinGet manifest for a GitHub release.
#
# Produces the winget-pkgs drop-in layout
#   <OutDir>/<PackageIdentifier>/<Version>/<PackageIdentifier>.yaml
# for a Rivulet Windows x64 MSI release asset. The manifest is a
# ManifestType: "singleton" file (installer + locale in one YAML) with the
# stable identity, canonical GitHub asset URL, SHA-256, and the MSI
# ProductCode/UpgradeCode so winget can detect and upgrade the installation.
#
# Usage:
#   pwsh packaging/windows/generate-winget-manifest.ps1 `
#       -Version 0.65.0-alpha.55 `
#       -ReleaseTag v0.65.0-alpha.55 `
#       -MsiPath staging/rivulet-windows-x86_64.msi `
#       -OutDir winget-manifests
#
#   # Deterministic/offline (any OS; ProductCode/chksum supplied by caller):
#   pwsh packaging/windows/generate-winget-manifest.ps1 `
#       -Version 0.65.0 -ReleaseTag v0.65.0 `
#       -InstallerSha256 <64 hex> -ProductCode <GUID> -OutDir winget-manifests
#
#   # Re-verify an existing manifest against the same release metadata:
#   pwsh packaging/windows/generate-winget-manifest.ps1 -ValidateOnly `
#       -ManifestPath .../<Id>/<Version>/<Id>.yaml `
#       -Version 0.65.0 -ReleaseTag v0.65.0 -InstallerSha256 <64 hex>
#
# With -MsiPath the SHA-256 is computed from the file and the ProductCode is
# read from the MSI database (WindowsInstaller COM, Windows only). For
# deterministic non-Windows runs pass -InstallerSha256 and/or -ProductCode
# explicitly; the renderer/validator are pure and CI-testable via Pester
# (packaging/windows/generate-winget-manifest.tests.ps1).

param(
  [Parameter(Mandatory=$true)][string]$Version,
  [string]$ReleaseTag,
  [string]$InstallerUrl,
  [string]$InstallerSha256,
  [string]$MsiPath,
  [string]$ProductCode,
  [string]$UpgradeCode = "A5C1E5E8-7A3B-4C9D-B6E2-9F1D4C7A8B90",
  [string]$PackageIdentifier = "Rivulet.Rivulet",
  [string]$Publisher = "Rivulet",
  [string]$PackageName = "Rivulet",
  [string]$Moniker = "rivulet",
  [string]$License = "MIT",
  [string]$LicenseUrl = "https://github.com/thoser666/Rivulet/blob/main/LICENSE",
  [string]$PackageUrl = "https://github.com/thoser666/Rivulet",
  [string]$AssetName = "rivulet-windows-x86_64.msi",
  [string]$OutDir = "winget-manifests",
  [switch]$ValidateOnly,
  [string]$ManifestPath
)

$ErrorActionPreference = "Stop"
$UpgradeCode = $UpgradeCode.ToUpperInvariant()

function Assert-ValidGuid {
  param([string]$Guid, [string]$What)
  $bare = $Guid.Trim('{', '}').ToUpperInvariant()
  if ($bare -notmatch '^[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}$') {
    throw "$What is not a valid GUID: '$Guid'"
  }
  return $bare
}

function Assert-ValidSha256 {
  param([string]$Sha)
  if ($Sha -notmatch '^[0-9A-Fa-f]{64}$') {
    throw "InstallerSha256 must be 64 hex characters, got '${Sha}'"
  }
  return $Sha.ToUpperInvariant()
}

function Assert-ValidVersion {
  if ($Version -notmatch '^\d+\.\d+\.\d+(-(alpha|beta|rc)\.\d+)?$') {
    throw "Version must look like x.y.z or x.y.z-(alpha|beta|rc).N, got '$Version'"
  }
}

function Build-CanonicalInstallerUrl {
  if ($ReleaseTag -and $ReleaseTag -ne "v$Version") {
    throw "ReleaseTag '$ReleaseTag' must equal 'v$Version' for the canonical asset URL"
  }
  if ($InstallerUrl) {
    $expectedPrefix = "$PackageUrl/releases/download/"
    if (-not $InstallerUrl.StartsWith($expectedPrefix, [StringComparison]::Ordinal)) {
      throw "InstallerUrl must be a GitHub release asset URL under $expectedPrefix, got '$InstallerUrl'"
    }
    if (-not $InstallerUrl.EndsWith("/$AssetName", [StringComparison]::Ordinal)) {
      throw "InstallerUrl must end with '/$AssetName', got '$InstallerUrl'"
    }
    return $InstallerUrl
  }
  return "$PackageUrl/releases/download/v$Version/$AssetName"
}

function Read-MsiProductCode {
  param([string]$Path)
  if (-not $Path) { throw "MsiPath required to read the ProductCode" }
  $resolved = (Resolve-Path -LiteralPath $Path).Path
  if (-not (Test-Path -LiteralPath $resolved) -or [IO.Path]::GetExtension($resolved) -ne ".msi") {
    throw "MsiPath must point to an existing .msi file, got '$resolved'"
  }
  $productCode = $null
  if ([Environment]::OSVersion.Platform -eq 'Win32NT') {
    $installer = New-Object -ComObject WindowsInstaller.Installer
    $db = $installer.GetType().InvokeMember("OpenDatabase", "InvokeMethod", $null, $installer, @($resolved, 0))
    try {
      $view = $db.GetType().InvokeMember("OpenView", "InvokeMethod", $null, $db,
        @("SELECT Value FROM Property WHERE Property = 'ProductCode'"))
      $view.GetType().InvokeMember("Execute", "InvokeMethod", $null, $view, $null)
      $record = $view.GetType().InvokeMember("Fetch", "InvokeMethod", $null, $view, $null)
      if ($record) {
        $productCode = $record.GetType().InvokeMember("StringData", "GetProperty", $null, $record, 1).ToString()
      }
    } finally {
      $view.GetType().InvokeMember("Close", "InvokeMethod", $null, $view, $null) | Out-Null
      $db.GetType().InvokeMember("Commit", "InvokeMethod", $null, $db, $null) | Out-Null
    }
  }
  if (-not $productCode) {
    throw "Could not read the ProductCode from '$resolved' (WindowsInstaller COM unavailable). Pass -ProductCode explicitly."
  }
  return $productCode
}

function Assert-ShasMatch {
  param([string]$Expected, [string]$Actual)
  if (-not $Expected) {
    throw "No expected InstallerSha256 provided"
  }
  if ($Actual -and $Expected.ToUpperInvariant() -ne $Actual.ToUpperInvariant()) {
    throw "InstallerSha256 mismatch: manifest has $($Actual.ToUpperInvariant()), expected $($Expected.ToUpperInvariant())"
  }
}

function Render-Manifest {
  param(
    [string]$VersionValue,
    [string]$Identifier,
    [string]$ManifestSha256,
    [string]$ManifestProductCode,
    [string]$ManifestUpgradeCode
  )
  return @"
PackageIdentifier: $Identifier
PackageVersion: $VersionValue
DefaultLocale: en-US
ManifestType: singleton
ManifestVersion: 1.6.0
Publisher: $Publisher
PackageName: $PackageName
PackageUrl: $PackageUrl
License: $License
LicenseUrl: $LicenseUrl
ShortDescription: Rivulet - Open-source screen recorder and live streaming studio
Description: Rivulet is an open-source screen recorder and live streaming studio with OBS-compatible automation (obs-websocket) and privacy-first telemetry.
Moniker: $Moniker
Tags:
  - streaming
  - broadcast
  - screen-recorder
  - recording
  - obs
  - twitch
  - capture
Installers:
  - Architecture: x64
    InstallerType: wix
    Scope: machine
    InstallerLocale: en-US
    InstallerUrl: $(Build-CanonicalInstallerUrl)
    InstallerSha256: $ManifestSha256
    UpgradeCode: "$ManifestUpgradeCode"
    ProductCode: "$ManifestProductCode"
"@
}

function Resolve-OutputPath {
  param([string]$Identifier, [string]$VersionValue)
  return (Join-Path (Join-Path (Join-Path $OutDir $Identifier) $VersionValue) "$Identifier.yaml")
}

function Validate-ManifestFile {
  param([string]$Path)
  if (-not $Path -or -not (Test-Path -LiteralPath $Path)) {
    throw "ManifestPath must point to an existing manifest file, got '$Path'"
  }
  $content = Get-Content -LiteralPath $Path -Raw
  foreach ($key in @(
    "PackageIdentifier: $PackageIdentifier",
    "PackageVersion: $Version",
    "ManifestType: singleton",
    "ManifestVersion:",
    "Publisher: $Publisher",
    "PackageName: $PackageName",
    "License:",
    "InstallerType: wix",
    "Scope: machine",
    "Architecture: x64"
  )) {
    if (-not $content.Contains($key)) {
      throw "Manifest $Path is missing the required '$key' block/entry"
    }
  }

  $expectedUrl = (Build-CanonicalInstallerUrl)
  if (-not $content.Contains("InstallerUrl: $expectedUrl")) {
    throw "Manifest InstallerUrl is not the canonical '$expectedUrl'"
  }
  $upgradeCanonical = Assert-ValidGuid $UpgradeCode "UpgradeCode"
  if (-not $content.Contains("UpgradeCode: `"{$upgradeCanonical}`"")) {
    throw "Manifest must keep the stable UpgradeCode '{$upgradeCanonical}'"
  }

  if ($InstallerSha256) {
    $m = [regex]::Match($content, '(?m)^\s*InstallerSha256:\s*([0-9A-Fa-f]{64})\s*$')
    if (-not $m.Success) {
      throw "Manifest InstallerSha256 is missing or malformed"
    }
    Assert-ShasMatch -Expected $InstallerSha256 -Actual $m.Groups[1].Value
  }

  if ($ProductCode) {
    $pc = [regex]::Match($content, "(?m)^\s*ProductCode:\s*`"?(\{?[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\}?)`"?\s*$")
    if (-not $pc.Success) {
      throw "Manifest ProductCode is missing or malformed"
    }
    $expectBare = (Assert-ValidGuid $ProductCode "ProductCode")
    $haveBare = $pc.Groups[1].Value.Trim('{', '}').ToUpperInvariant()
    if ($expectBare -ne $haveBare) {
      throw "Manifest ProductCode mismatch: has $haveBare, expected $expectBare"
    }
  }

  return $Path
}

function Main {
  Assert-ValidVersion

  # UpgradeCode must always be a valid, canonical GUID.
  $upgradeCanonical = Assert-ValidGuid $UpgradeCode "UpgradeCode"

  if ($ValidateOnly) {
    $validated = Validate-ManifestFile -Path $ManifestPath
    Write-Host "winget manifest validation passed: $validated"
    return
  }

  $sha = $null
  $productCanonical = $null
  if ($MsiPath) {
    $resolved = (Resolve-Path -LiteralPath $MsiPath).Path
    $sha = (Get-FileHash -LiteralPath $resolved -Algorithm SHA256).Hash.ToUpperInvariant()
    if (-not $ProductCode) {
      $ProductCode = Read-MsiProductCode -Path $resolved
    }
  } else {
    if (-not $InstallerSha256) {
      throw "Provide -MsiPath (computes SHA-256 + reads ProductCode) or -InstallerSha256"
    }
    $sha = Assert-ValidSha256 $InstallerSha256
  }
  if (-not $sha) { $sha = (Assert-ValidSha256 $InstallerSha256) }
  $productCanonical = Assert-ValidGuid $ProductCode "ProductCode"

  $manifest = Render-Manifest `
    -VersionValue $Version `
    -Identifier $PackageIdentifier `
    -ManifestSha256 $sha `
    -ManifestProductCode "{$productCanonical}" `
    -ManifestUpgradeCode "{$upgradeCanonical}"

  $outFile = Resolve-OutputPath $PackageIdentifier $Version
  $outDir = [IO.Path]::GetDirectoryName($outFile)
  New-Item -ItemType Directory -Force -Path $outDir | Out-Null
  [IO.File]::WriteAllText($outFile, $manifest, [Text.UTF8Encoding]::new($false))

  Write-Host "winget manifest generated (dry-run payload): $outFile"
  Write-Host "  InstallerUrl : $(Build-CanonicalInstallerUrl)"
  Write-Host "  SHA-256      : $sha"
  Write-Host "  ProductCode  : {$productCanonical}"
  Write-Host "  UpgradeCode  : $upgradeCanonical"
  Write-Host "Submission stays external: open a PR against microsoft/winget-pkgs with this folder."
}

Main