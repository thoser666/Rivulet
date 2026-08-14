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

$wxs = Join-Path $PSScriptRoot "rivulet.wxs"
$bundle = Join-Path $Staging "bundle"
if (-not (Test-Path $bundle)) { throw "Bundle-Verzeichnis fehlt: $bundle (zuerst build-portable.ps1 ausführen)" }

$harvestXml = Join-Path $Staging "rivulet.harvest.wxs"

# WiX Toolset installieren (falls nicht vorhanden).
if (-not (Get-Command "candle.exe" -ErrorAction SilentlyContinue)) {
  Write-Host "WiX Toolset wird installiert (choco)..."
  choco install wixtoolset -y --no-progress
}

if (-not (Get-Command "heat.exe" -ErrorAction SilentlyContinue)) {
  throw "heat.exe fehlt - WiX-Installation fehlgeschlagen"
}

# Alle Dateien im Bundle harvesten (exkl. bereits erzeugter Artefakte).
Write-Host "Harvesting Bundle: $bundle"
$harvestExclude = @("*.wixobj", "*.wixpdb", "rivulet.harvest.wxs", "*.msi", "*.msi.clean", "*.zip")
& heat.exe dir $bundle -cg ProductComponents -dr INSTALLFOLDER `
  -srd -sfrag -sreg -gg -var var.BundleDir `
  -out $harvestXml `
  -exclude ($harvestExclude -join ";")
if ($LASTEXITCODE -ne 0) { throw "heat.exe fehlgeschlagen" }

# WiX-Binary setzen: Version und Bundle-Pfad als separate Preprocessor-Defines.
$mainObj = Join-Path $Staging "rivulet.wixobj"
$harvestObj = Join-Path $Staging "rivulet.harvest.wixobj"

Write-Host "candle: $wxs"
& candle.exe $wxs "-dProductVersion=$Version" "-dBundleDir=$bundle" -ext WixUIExtension -out $mainObj
if ($LASTEXITCODE -ne 0) { throw "candle.exe fehlgeschlagen (Produkt)" }

Write-Host "candle: $harvestXml"
& candle.exe $harvestXml "-dProductVersion=$Version" "-dBundleDir=$bundle" -ext WixUIExtension -out $harvestObj
if ($LASTEXITCODE -ne 0) { throw "candle.exe fehlgeschlagen (Harvest)" }

$lightOut = Join-Path $Staging "rivulet.msi"
Write-Host "light: $mainObj, $harvestObj -> $lightOut"
& light.exe $mainObj $harvestObj -ext WixUIExtension -o $lightOut
if ($LASTEXITCODE -ne 0) { throw "light.exe fehlgeschlagen" }

Copy-Item $lightOut $OutFile -Force
Write-Host "MSI erstellt: $OutFile"
