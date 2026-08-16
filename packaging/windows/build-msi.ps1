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

# MSI only accepts numeric versions "x.x.x.x". From a pre-release version
# (e.g. "0.2.0-alpha.1") "0.2.0.1" is derived (last component = alpha number);
# from a stable version "0.2.0" it becomes "0.2.0.0".
$msiVersion = $Version
if ($msiVersion -match "^(?<base>\d+\.\d+\.\d+)(?:-(?:alpha|beta|rc)\.(?<pre>\d+))?$") {
  $msiVersion = "$($Matches['base']).$($Matches['pre'] ?? '0')"
} elseif ($msiVersion -match "^(?<base>\d+\.\d+\.\d+)$") {
  $msiVersion = "$($Matches['base']).0"
} else {
  Write-Host "WARNING: Unparseable version '$Version', using 0.0.0.0" -ForegroundColor Yellow
  $msiVersion = "0.0.0.0"
}
Write-Host "MSI version: $msiVersion (from $Version)"

$wxs = Join-Path $PSScriptRoot "rivulet.wxs"
$bundle = Join-Path $Staging "bundle"
if (-not (Test-Path $bundle)) { throw "Bundle directory missing: $bundle (run build-portable.ps1 first)" }

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
& candle.exe $wxs "-dProductVersion=$msiVersion" "-dBundleDir=$bundle" -ext WixUIExtension -out $mainObj
if ($LASTEXITCODE -ne 0) { throw "candle.exe failed (product)" }

Write-Host "candle: $harvestXml"
& candle.exe $harvestXml "-dProductVersion=$msiVersion" "-dBundleDir=$bundle" -ext WixUIExtension -out $harvestObj
if ($LASTEXITCODE -ne 0) { throw "candle.exe failed (harvest)" }

$lightOut = Join-Path $Staging "rivulet.msi"
Write-Host "light: $mainObj, $harvestObj -> $lightOut"
& light.exe $mainObj $harvestObj -ext WixUIExtension -o $lightOut
if ($LASTEXITCODE -ne 0) { throw "light.exe failed" }

Copy-Item $lightOut $OutFile -Force
Write-Host "MSI created: $OutFile"
