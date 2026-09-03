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
# The launcher is the stable entry point. It starts the version installed in
# this directory and records pre-Rust diagnostics when the GUI fails.
if (Test-Path (Join-Path $Staging "rivulet-launcher.exe")) {
  Copy-Item (Join-Path $Staging "rivulet-launcher.exe") (Join-Path $bundle "rivulet-launcher.exe") -Force
}
Copy-Item (Join-Path $Staging "rivulet.exe") (Join-Path $bundle "rivulet.exe") -Force
# The update watchdog runs detached during updates and waits for the GUI to
# exit before installing. It must live next to the GUI so the updater can find
# it via current_exe().parent().
if (Test-Path (Join-Path $Staging "rivulet-updater.exe")) {
  Copy-Item (Join-Path $Staging "rivulet-updater.exe") (Join-Path $bundle "rivulet-updater.exe") -Force
}

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

# 7. Discord setup assets: the app icon (member list), the Rich Presence
# artwork (profile card) and the setup docs ship inside the installer so
# users can complete the Discord Rich Presence configuration offline. The
# layout mirrors the repo (docs/assets/...) so relative image links in the
# markdown keep working. The MSI harvests the whole bundle directory, so
# everything placed here lands in the installer automatically.
$repoRoot = Join-Path $PSScriptRoot "..\.."
$discordDir = Join-Path $bundle "discord"
New-Item -ItemType Directory -Force -Path (Join-Path $discordDir "assets") | Out-Null
foreach ($file in @(
  @{ Src = "docs/assets/rivulet-app-icon-512.png";      Dst = "rivulet-app-icon-512.png" },
  @{ Src = "docs/assets/rivulet-rich-presence-1024.png"; Dst = "rivulet-rich-presence-1024.png" },
  @{ Src = "docs/assets/rivulet-rich-presence-512.png";  Dst = "assets\rivulet-rich-presence-512.png" },
  @{ Src = "docs/activity-status.md";                    Dst = "activity-status.md" }
)) {
  $src = Join-Path $repoRoot $file.Src
  $dst = Join-Path $discordDir $file.Dst
  if (Test-Path $src) {
    Copy-Item $src $dst -Force
  } else {
    Write-Host "WARNING: Discord asset missing, skipped: $src" -ForegroundColor Yellow
  }
}

# 6. Zip the bundle (portable variant).
if (Test-Path $OutFile) { Remove-Item $OutFile -Force }
Compress-Archive -Path (Join-Path $bundle "*") -DestinationPath $OutFile -CompressionLevel Optimal
Write-Host "Portable bundle created: $OutFile"
