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

# MSI akzeptiert nur numerische Versionen "x.x.x.x". Aus einer Pre-Release-
# Version (z.B. "0.2.0-alpha.1") wird "0.2.0.1" (letzte Komponente = Alpha-Nr.),
# aus einer stabilen Version "0.2.0" wird "0.2.0.0".
$msiVersion = $Version
if ($msiVersion -match "^(?<base>\d+\.\d+\.\d+)(?:-(?:alpha|beta|rc)\.(?<pre>\d+))?$") {
  $msiVersion = "$($Matches['base']).$($Matches['pre'] ?? '0')"
} elseif ($msiVersion -match "^(?<base>\d+\.\d+\.\d+)$") {
  $msiVersion = "$($Matches['base']).0"
} else {
  Write-Host "WARNUNG: Nicht parsebare Version '$Version', nutze 0.0.0.0" -ForegroundColor Yellow
  $msiVersion = "0.0.0.0"
}
Write-Host "MSI-Version: $msiVersion (aus $Version)"

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
  -arch x64 `
  -out $harvestXml `
  -exclude ($harvestExclude -join ";")
if ($LASTEXITCODE -ne 0) { throw "heat.exe fehlgeschlagen" }

# WiX-Binary setzen: Version und Bundle-Pfad als separate Preprocessor-Defines.
$mainObj = Join-Path $Staging "rivulet.wixobj"
$harvestObj = Join-Path $Staging "rivulet.harvest.wixobj"

Write-Host "candle: $wxs"
& candle.exe $wxs "-dProductVersion=$msiVersion" "-dBundleDir=$bundle" -ext WixUIExtension -out $mainObj
if ($LASTEXITCODE -ne 0) { throw "candle.exe fehlgeschlagen (Produkt)" }

Write-Host "candle: $harvestXml"
& candle.exe $harvestXml "-dProductVersion=$msiVersion" "-dBundleDir=$bundle" -ext WixUIExtension -out $harvestObj
if ($LASTEXITCODE -ne 0) { throw "candle.exe fehlgeschlagen (Harvest)" }

$lightOut = Join-Path $Staging "rivulet.msi"
Write-Host "light: $mainObj, $harvestObj -> $lightOut"
& light.exe $mainObj $harvestObj -ext WixUIExtension -o $lightOut
if ($LASTEXITCODE -ne 0) { throw "light.exe fehlgeschlagen" }

Copy-Item $lightOut $OutFile -Force
Write-Host "MSI erstellt: $OutFile"
