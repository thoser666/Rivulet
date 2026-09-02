# Builds a Rivulet MSI installer from the portable bundle directory using WiX.
# The bundle is created by build-portable.ps1 (exe + GStreamer runtime) so the
# MSI ships a self-contained installation.
#
# Usage: pwsh packaging/windows/build-msi.ps1 -Version <version> -Staging <dir> -OutFile <path>
param(
  [Parameter(Mandatory=$true)][string]$Version,
  [Parameter(Mandatory=$true)][string]$Staging,
  [Parameter(Mandatory=$true)][string]$OutFile
)

$ErrorActionPreference = "Stop"

# MSI only accepts numeric versions "x.y.z.w", and Windows Installer compares
# ONLY the first THREE fields (major.minor.build) for upgrades - the fourth
# (revision) field is silently ignored. Putting a varying alpha number into the
# fourth field made every alpha of the same base version look identical to
# Windows Installer, so <MajorUpgrade AllowSameVersionUpgrades="no"> refused to
# treat a newer alpha as an upgrade: the old version stayed, the new files were
# never installed (only the registry/ARP entry was touched).
#
# To keep every publish at a higher MSI version than the previous one, the
# pre-release number is mapped INTO the three compared fields:
#   - "0.65.0-alpha.55" -> "0.65.55"   (patch = pre-release number)
#     If that patch would exceed 255 the overflow carries into the minor
#     field, e.g. "-alpha.300" -> minor +1, patch 44 => "0.65.44~" handled by
#     the same arithmetic below.
#   - a stable "0.65.0" -> "0.65.255"  (patch 255) so it sorts above every
#     alpha of the same base line, while the next base line starts again from
#     a lower patch but a higher minor/major.
$msiVersionRegexes = @(
  "^(?<major>\d+)\.(?<minor>\d+)\.(?<patch>\d+)-(?:alpha|beta|rc)\.(?<pre>\d+)$",
  "^(?<major>\d+)\.(?<minor>\d+)\.(?<patch>\d+)$"
)
$msiMajor = 0; $msiMinor = 0; $msiPatch = 0
$parsed = $false
foreach ($re in $msiVersionRegexes) {
  if ($Version -match $re) {
    $msiMajor = [int]$Matches['major']
    $msiMinor = [int]$Matches['minor']
    $hasPre = ($Matches.ContainsKey('pre'))
    if ($hasPre) {
      # Pre-release: patch = pre-release number (carry overflow into minor).
      $msiPatch = [int]$Matches['pre']
      if ($msiPatch -gt 255) {
        $msiMinor += [int][Math]::Floor($msiPatch / 256)
        $msiPatch = $msiPatch % 256
      }
    } else {
      # Stable: sort above every alpha of the same base line.
      $msiPatch = 255
    }
    $parsed = $true
    break
  }
}
if (-not $parsed) {
  Write-Host "WARNING: Unparseable version '$Version', using 0.0.0.0" -ForegroundColor Yellow
} else {
  # Clamp fields to the MSI range just in case the carry ever exceeds it.
  $msiMajor = [Math]::Min($msiMajor, 255)
  $msiMinor = [Math]::Min($msiMinor, 255)
  $msiPatch = [Math]::Min($msiPatch, 255)
}
$msiVersion = "$msiMajor.$msiMinor.$msiPatch"
Write-Host "MSI version: $msiVersion (from $Version)"

$wxs = Join-Path $PSScriptRoot "rivulet.wxs"
$bundle = Join-Path $Staging "bundle"
if (-not (Test-Path $bundle)) { throw "Bundle directory missing: $bundle (run build-portable.ps1 first)" }

# Icon for the Start Menu / Desktop shortcuts.
$iconPath = Join-Path $PSScriptRoot "..\..\rivulet-gui\assets\rivulet_logo.ico"
if (-not (Test-Path $iconPath)) { throw "Icon file missing: $iconPath" }

# License text shown in the WixUI_InstallDir license agreement dialog.
$licensePath = Join-Path $PSScriptRoot "license.rtf"
if (-not (Test-Path $licensePath)) { throw "License file missing: $licensePath" }

$harvestXml = Join-Path $Staging "rivulet.harvest.wxs"

# Install the WiX toolset (if not present).
if (-not (Get-Command "candle.exe" -ErrorAction SilentlyContinue)) {
  Write-Host "Installing WiX Toolset (choco)..."
  choco install wixtoolset -y --no-progress
}

if (-not (Get-Command "heat.exe" -ErrorAction SilentlyContinue)) {
  throw "heat.exe missing - WiX installation failed"
}

# Harvest all files in the bundle (excluding already generated artifacts).
Write-Host "Harvesting bundle: $bundle"
$harvestExclude = @("*.wixobj", "*.wixpdb", "rivulet.harvest.wxs", "*.msi", "*.msi.clean", "*.zip")
& heat.exe dir $bundle -cg ProductComponents -dr INSTALLFOLDER `
  -srd -sfrag -sreg -gg -var var.BundleDir `
  -arch x64 `
  -out $harvestXml `
  -exclude ($harvestExclude -join ";")
if ($LASTEXITCODE -ne 0) { throw "heat.exe failed" }

# heat v3 does not reliably mark components as 64-bit even though the
# product installs to ProgramFiles64Folder (otherwise ICE80 fails at light).
# Therefore retroactively set Win64="yes" on all harvested components.
$harvestContent = Get-Content $harvestXml -Raw
$harvestContent = $harvestContent -replace '<Component ', '<Component Win64="yes" '
Set-Content -Path $harvestXml -Value $harvestContent -NoNewline -Encoding UTF8
Write-Host "Set Win64='yes' on all harvested components."

# Set the WiX binaries: version and bundle path as separate preprocessor defines.
$mainObj = Join-Path $Staging "rivulet.wixobj"
$harvestObj = Join-Path $Staging "rivulet.harvest.wixobj"

Write-Host "candle: $wxs"
& candle.exe $wxs "-dProductVersion=$msiVersion" "-dBundleDir=$bundle" "-dIconPath=$iconPath" "-dLicensePath=$licensePath" -ext WixUIExtension -out $mainObj
if ($LASTEXITCODE -ne 0) { throw "candle.exe failed (product)" }

Write-Host "candle: $harvestXml"
& candle.exe $harvestXml "-dProductVersion=$msiVersion" "-dBundleDir=$bundle" "-dIconPath=$iconPath" "-dLicensePath=$licensePath" -ext WixUIExtension -out $harvestObj
if ($LASTEXITCODE -ne 0) { throw "candle.exe failed (harvest)" }

$lightOut = Join-Path $Staging "rivulet.msi"
Write-Host "light: $mainObj, $harvestObj -> $lightOut"
& light.exe $mainObj $harvestObj -ext WixUIExtension -o $lightOut
if ($LASTEXITCODE -ne 0) { throw "light.exe failed" }

Copy-Item $lightOut $OutFile -Force
Write-Host "MSI created: $OutFile"
