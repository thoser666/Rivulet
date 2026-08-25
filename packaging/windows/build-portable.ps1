# Creates a portable (self-contained) Rivulet bundle on Windows: copies the
# binary plus the GStreamer runtime (bin DLLs, plugins, plugin scanner) into a
# bundle directory so the exe runs on machines without GStreamer installed.
# The bundle directory is reused by the MSI build (build-msi.ps1).
#
# Usage: pwsh packaging/windows/build-portable.ps1 -Staging <dir> -OutFile <zip-path>
param(
  [Parameter(Mandatory=$true)][string]$Staging,
  [Parameter(Mandatory=$true)][string]$OutFile
)

$ErrorActionPreference = "Stop"

$gstRoot = [System.Environment]::GetEnvironmentVariable("GSTREAMER_1_0_ROOT_MSVC_X86_64", "Machine")
if (-not $gstRoot) { $gstRoot = "C:\gstreamer\1.0\msvc_x86_64\" }
$gstBin = Join-Path $gstRoot "bin"
$gstPlugins = Join-Path $gstRoot "lib\gstreamer-1.0"
$gstScannerDir = Join-Path $gstRoot "libexec\gstreamer-1.0"

if (-not (Test-Path $gstBin)) { throw "GStreamer bin not found: $gstBin" }
if (-not (Test-Path $gstPlugins)) { throw "GStreamer plugins not found: $gstPlugins" }

$bundle = Join-Path $Staging "bundle"
New-Item -ItemType Directory -Force -Path $bundle | Out-Null

# 1. Application binaries. The launcher is the user-facing entry point:
# it records failures from the GUI process that happen before Rust logging.
Copy-Item (Join-Path $Staging "rivulet-gui.exe") (Join-Path $bundle "rivulet-gui.exe") -Force
Copy-Item (Join-Path $Staging "rivulet.exe") (Join-Path $bundle "rivulet.exe") -Force

# 2. GStreamer runtime: all bin DLLs.
Copy-Item (Join-Path $gstBin "*.*") $bundle -Force -ErrorAction SilentlyContinue

# 3. GStreamer plugins in a subdirectory.
$pluginsDir = Join-Path $bundle "gstreamer-1.0"
New-Item -ItemType Directory -Force -Path $pluginsDir | Out-Null
Copy-Item (Join-Path $gstPlugins "*.*") $pluginsDir -Force -ErrorAction SilentlyContinue

# 4. Plugin scanner (needed by GStreamer for registration).
if (Test-Path $gstScannerDir) {
  Copy-Item (Join-Path $gstScannerDir "gst-plugin-scanner.exe") $bundle -Force -ErrorAction SilentlyContinue
}

# 5. Set the env template: load rtmp2sink & plugins from the bundle.
@"
@echo off
set GST_PLUGIN_PATH=%~dp0gstreamer-1.0
set GST_PLUGIN_SYSTEM_PATH=%~dp0gstreamer-1.0
set GST_PLUGIN_SCANNER=%~dp0gst-plugin-scanner.exe
set GST_REGISTRY=%TEMP%\rivulet-gst-registry.bin
start "" "%~dp0rivulet.exe" %*
"@ | Set-Content -Path (Join-Path $bundle "Rivulet.bat") -Encoding ASCII

# 6. Zip the bundle (portable variant).
if (Test-Path $OutFile) { Remove-Item $OutFile -Force }
Compress-Archive -Path (Join-Path $bundle "*") -DestinationPath $OutFile -CompressionLevel Optimal
Write-Host "Portable bundle created: $OutFile"
