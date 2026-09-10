# Generates and validates the Rivulet Scoop manifest for a GitHub release.
#
# Scoop installs from a bucket that references the portable ZIP asset with a
# SHA-256 — no signing required (unlike winget, which wants signed installers
# for a smooth first-run experience). The manifest is rendered from a release
# tag and pinned by the SHA from the release's SHA256SUMS asset; it never
# trusts the manifest it validates by re-rendering and comparing.
#
# Usage:
#   # From a real release (computes nothing locally; takes the SHA-256 from
#   # the release's SHA256SUMS asset, verifying the asset is listed there):
#   pwsh packaging/windows/generate-scoop-manifest.ps1 `
#       -Version 0.65.0-alpha.163 -ReleaseTag v0.65.0-alpha.163 -OutDir out
#
#   # Deterministic/offline (any OS; SHA-256 supplied by caller):
#   pwsh packaging/windows/generate-scoop-manifest.ps1 `
#       -Version 0.65.0-alpha.55 -ReleaseTag v0.65.0-alpha.55 `
#       -ZipSha256 <64 hex> -OutDir out
#
#   # Re-verify an existing manifest (re-render + byte compare):
#   pwsh packaging/windows/generate-scoop-manifest.ps1 -ValidateOnly `
#       -ManifestPath out/rivulet.json `
#       -Version 0.65.0-alpha.55 -ReleaseTag v0.65.0-alpha.55 -ZipSha256 <64 hex>
#
# The bucket itself lives in a separate repository (e.g. thoser666/scoop-bucket)
# with the standard Scoop bucket layout; the CI dry-run job renders the
# manifest for a real release and proves the URL/SHA pair is listed in the
# release's own SHA256SUMS.

param(
  [Parameter(Mandatory = $true)][string]$Version,
  [string]$ReleaseTag,
  [string]$ZipSha256,
  [string]$Repo = "thoser666/Rivulet",
  [string]$BucketName = "rivulet",
  [string]$Publisher = "Rivulet",
  [string]$License = "MIT",
  [string]$LicenseUrl = "https://github.com/thoser666/Rivulet/blob/develop/LICENSE",
  [string]$OutDir = "scoop-manifests",
  [switch]$ValidateOnly,
  [string]$ManifestPath
)

$ErrorActionPreference = "Stop"
$tag = if ($ReleaseTag) { $ReleaseTag } else { "v$Version" }

function Assert-ValidSha256 {
  param([string]$Sha, [string]$What)
  if ($Sha -notmatch '^[0-9a-f]{64}$') {
    throw "$What must be a 64-char lowercase hex SHA-256, got: '$Sha'"
  }
}

function Get-ZipSha256FromRelease {
  param([string]$Repo, [string]$Tag, [string]$AssetName)
  if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "gh CLI not found; pass -ZipSha256 explicitly for offline rendering"
  }
  $tmp = Join-Path ([IO.Path]::GetTempPath()) ("rivulet-scoop-shasums-" + [Guid]::NewGuid().ToString("N"))
  New-Item -ItemType Directory -Force -Path $tmp | Out-Null
  try {
    & gh release download $Tag --repo $Repo --pattern SHA256SUMS --dir $tmp | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "failed to download SHA256SUMS for $Tag" }
    $sumsFile = Join-Path $tmp "SHA256SUMS"
    if (-not (Test-Path $sumsFile)) { throw "release $Tag has no SHA256SUMS asset" }
    $line = Select-String -LiteralPath $sumsFile -Pattern ('^([0-9a-f]{64})\s+\*?' + [regex]::Escape($AssetName) + '\s*$') |
      Select-Object -First 1
    if (-not $line) {
      throw "SHA256SUMS for $Tag does not list '$AssetName' — refusing to render an unverified manifest"
    }
    return $line.Matches[0].Groups[1].Value
  }
  finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
  }
}

# Scoop manifest fields. Notes:
# - checkver/github: Scoop can auto-check for newer versions against the repo.
# - extract_dir: the portable ZIP has a flat layout (no top-level folder).
# - bin: rivulet-gui.exe is always present; a shim is created on PATH.
# - shortcuts: Start-menu entry for the GUI (requires Scoop ≥ 0.4 / 'extras'
#   handler; harmless no-op on older clients).
$assetName = "rivulet-windows-x86_64-portable.zip"
$zipUrl = "https://github.com/$Repo/releases/download/$tag/$assetName"

if (-not $ZipSha256) {
  $ZipSha256 = Get-ZipSha256FromRelease -Repo $Repo -Tag $tag -AssetName $assetName
}
$ZipSha256 = $ZipSha256.ToLowerInvariant()
Assert-ValidSha256 -Sha $ZipSha256 -What "-ZipSha256 / SHA256SUMS hash"

# Convert the .NET version string into Scoop's nested version array:
# "0.65.0" -> @("0", "65", "0"); a prerelease suffix (e.g. "-alpha.163") is
# dropped because Scoop picks the highest numeric version array.
$numeric = ($Version -replace '-.*$', '' -split '\.') | ForEach-Object { $_ }
$versionParts = @($numeric + @("0", "0", "0"))[0..2]

$manifest = [ordered]@{
  '$schema' = "https://raw.githubusercontent.com/ScoopInstaller/Scoop/master/schema.json"
  version   = $Version
  architecture = [ordered]@{
    "64bit" = [ordered]@{
      url     = $zipUrl
      hash    = $ZipSha256
    }
  }
  architecture_note = "Windows x86_64 only; the release pipeline currently builds no arm64 Windows binaries."
  bin       = @("rivulet-gui.exe")
  shortcuts = @(
    @("rivulet-gui.exe", "Rivulet")
  )
  extract_dir = $null
  checkver = [ordered]@{
    github = "https://github.com/$Repo"
  }
  autofix = $true
  description = "Rivulet - Open-source screen recorder and live streaming studio"
  homepage    = "https://github.com/$Repo"
  license     = "$License`nLicenseUrl: $LicenseUrl" -replace "`n", " "
  notes       = @(
    "Rivulet is installed as a portable bundle with a bundled GStreamer runtime.",
    "The 'rivulet' shim starts the Rivulet GUI; recordings go to the folder configured in the app."
  )
}
# extract_dir only makes sense with a value; drop the null key.
$manifest.Remove("extract_dir")

$json = $manifest | ConvertTo-Json -Depth 6

if ($ValidateOnly) {
  if (-not $ManifestPath) { throw "-ValidateOnly requires -ManifestPath" }
  $existing = Get-Content -LiteralPath $ManifestPath -Raw
  $expected = $json -replace "`r`n", "`n"
  $actual = $existing -replace "`r`n", "`n"
  if ($expected -ne $actual) {
    throw "manifest drift: $ManifestPath does not match the expected render for version $Version (tag $tag, sha $ZipSha256)"
  }
  # Structural sanity: the fields Scoop actually consumes must survive.
  $parsed = $existing | ConvertFrom-Json
  foreach ($field in @("version", "architecture", "bin", "license", "checkver")) {
    if (-not $parsed.PSObject.Properties[$field]) {
      throw "manifest drift: required Scoop field '$field' missing from $ManifestPath"
    }
  }
  Write-Output "OK manifest verified: $ManifestPath"
  exit 0
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$outPath = Join-Path $OutDir "$BucketName.json"
[System.IO.File]::WriteAllText($outPath, $json)
Write-Output "Wrote $outPath"
Write-Output "Bucket layout: copy $BucketName.json into the bucket repo's 'bucket/' directory."
