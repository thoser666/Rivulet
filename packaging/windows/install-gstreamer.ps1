# install-gstreamer.ps1 - Install the GStreamer MSVC x86_64 SDK for CI.
#
# Handles BOTH installer generations so the version pin can move freely:
#   * <= 1.26.x: two classic MSIs (runtime + devel), installed silently
#     with `msiexec /qn INSTALLLEVEL=100`.
#   * >= 1.28.x: ONE unified Inno Setup .exe installer (cerbero); the
#     MSI-era per-component packages no longer exist on
#     gstreamer.freedesktop.org for these versions. Installed silently
#     with /VERYSILENT /TYPE=devel into a FIXED directory.
#
# Download strategy (identical for both formats):
#   1. local cache directory (actions/cache restore)
#   2. the mirrored Rivulet release tagged `gstreamer-msi-<version>`
#      (tag name is historic; it hosts the .exe assets too)
#   3. freedesktop.org direct download (Varnish CDN, may 503)
# Every download is verified against the official freedesktop
# `.sha256sum` before it is trusted (the 1.28 installers are NOT
# Authenticode-signed, so the digest is the only integrity anchor).
#
# After installation the script exports the same environment contract the
# workflows have always consumed: GSTREAMER_1_0_ROOT_MSVC_X86_64,
# PKG_CONFIG_PATH, GST_PLUGIN_PATH, GST_PLUGIN_SYSTEM_PATH,
# GST_PLUGIN_SCANNER, GST_REGISTRY and PATH.
#
# Usage (CI):
#   pwsh packaging/windows/install-gstreamer.ps1 -Version 1.28.6 -CacheDir C:/gstreamer-cache

param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [string]$CacheDir = "C:/gstreamer-cache",

    # Fixed install root. The Inno installer gets /DIR=<this> so the
    # layout is byte-for-byte deterministic regardless of installer
    # defaults; the MSI path installs to the machine default which IS
    # this path for the x86_64 MSVC runtime.
    [string]$InstallRoot = "C:\gstreamer\1.0\msvc_x86_64",

    # When set, skip the actual installation (download + verify only).
    [switch]$DownloadOnly
)

$ErrorActionPreference = "Stop"

$freedesktopBase = "https://gstreamer.freedesktop.org/data/pkg/windows/$Version/msvc"
$mirrorBase = "https://github.com/thoser666/Rivulet/releases/download/gstreamer-msi-$Version"

# The Inno generation replaced the per-component MSIs with one unified
# installer; detect the format from what upstream actually ships.
$exeInstaller = "gstreamer-1.0-msvc-x86_64-$Version.exe"
$runtimeMsi = "gstreamer-1.0-msvc-x86_64-$Version.msi"
$develMsi = "gstreamer-1.0-devel-msvc-x86_64-$Version.msi"

$exeAvailable = $false
try {
    $head = Invoke-WebRequest -Uri "$freedesktopBase/$exeInstaller" -Method Head -TimeoutSec 30
    $exeAvailable = ($head.StatusCode -eq 200)
} catch {
    $exeAvailable = $false
}

if ($exeAvailable) {
    $artifacts = @($exeInstaller)
    Write-Host "GStreamer $Version ships the unified Inno Setup installer (no per-component MSIs)."
} else {
    $artifacts = @($runtimeMsi, $develMsi)
    Write-Host "GStreamer $Version ships the classic runtime/devel MSI pair."
}

function Get-OfficialSha256([string]$FileName) {
    # The .sha256sum sidecar lists "digest  <name>"; parse robustly.
    # -UseBasicParsing: the sidecar is a plain text file, and the default
    # IE-based parser can return a Byte[] instead of a string.
    $content = (Invoke-WebRequest -Uri "$freedesktopBase/$FileName.sha256sum" -TimeoutSec 60 -UseBasicParsing).Content
    if ($content -is [byte[]]) { $content = [System.Text.Encoding]::UTF8.GetString($content) }
    $first = ("$content").Trim() -split "`n" | Select-Object -First 1
    return ($first -split '\s+')[0].ToLowerInvariant()
}

function Save-VerifiedArtifact([string]$FileName, [string]$DestPath) {
    $expected = Get-OfficialSha256 $FileName

    # 1. local cache
    $cached = Join-Path $CacheDir $FileName
    if (Test-Path $cached) {
        $actual = (Get-FileHash $cached -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -eq $expected) {
            Write-Host "$FileName restored from cache (digest verified)."
            Copy-Item $cached $DestPath -Force
            return
        }
        Write-Host "Cached $FileName has a stale digest - re-downloading."
        Remove-Item $cached -Force
    }

    # 2. mirrored release
    try {
        Write-Host "Downloading $FileName from the mirrored release..."
        Invoke-WebRequest -Uri "$mirrorBase/$FileName" -OutFile $DestPath -TimeoutSec 900
        $actual = (Get-FileHash $DestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -eq $expected) {
            Write-Host "$FileName downloaded from the mirror (digest verified)."
            if (-not (Test-Path $CacheDir)) { New-Item -ItemType Directory -Path $CacheDir -Force | Out-Null }
            Copy-Item $DestPath $cached -Force
            return
        }
        Write-Host "Mirror copy of $FileName has digest $actual, expected $expected - falling back."
        Remove-Item $DestPath -Force
    } catch {
        Write-Host "Mirror download failed: $_"
        if (Test-Path $DestPath) { Remove-Item $DestPath -Force }
    }

    # 3. freedesktop.org (retry; the Varnish CDN 503s under load)
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        try {
            Write-Host "Downloading $FileName from freedesktop.org (attempt $attempt/3)..."
            Invoke-WebRequest -Uri "$freedesktopBase/$FileName" -OutFile $DestPath -TimeoutSec 900
            $actual = (Get-FileHash $DestPath -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actual -ne $expected) {
                throw "digest mismatch: got $actual, expected $expected"
            }
            Write-Host "$FileName downloaded from freedesktop.org (digest verified)."
            if (-not (Test-Path $CacheDir)) { New-Item -ItemType Directory -Path $CacheDir -Force | Out-Null }
            Copy-Item $DestPath $cached -Force
            return
        } catch {
            Write-Host "Download attempt ${attempt} failed: $_"
            if (Test-Path $DestPath) { Remove-Item $DestPath -Force }
            if ($attempt -eq 3) { throw "All download sources failed for $FileName" }
            Start-Sleep -Seconds (10 * $attempt)
        }
    }
}

if (-not (Test-Path $CacheDir)) { New-Item -ItemType Directory -Path $CacheDir -Force | Out-Null }
$workDir = Join-Path $env:TEMP "rivulet-gst-$Version"
if (-not (Test-Path $workDir)) { New-Item -ItemType Directory -Path $workDir -Force | Out-Null }

$paths = @{}
foreach ($name in $artifacts) {
    $dest = Join-Path $workDir $name
    Save-VerifiedArtifact $name $dest
    $paths[$name] = $dest
}

if ($DownloadOnly) {
    Write-Host "Download-only mode: skipping installation."
    exit 0
}

if ($exeAvailable) {
    # Inno Setup silent install (see the download page's parameter list
    # and cerbero's data/inno generator):
    #   /VERYSILENT      no dialogs at all
    #   /SUPPRESSMSGBOXES
    #   /NORESTART       no unattended reboot
    #   /TYPE=devel      Runtime + development headers (replaces the
    #                    second devel MSI)
    #   /DIR=<root>      fixed install directory (deterministic layout;
    #                    1.28 defaults to %LOCALAPPDATA%-scoped paths
    #                    which CI does not want)
    # The installer is not Authenticode-signed (verified); integrity
    # rests entirely on the SHA256 check above.
    # The installer declares PrivilegesRequired=admin (with commandline
    # override allowed). CI runners are elevated, so the admin install
    # (machine ENV + registry, uninstaller) goes through; on non-elevated
    # dev machines it fails, and we retry once in /PORTABLE=1 mode, which
    # skips machine ENV/registry entirely - the fixed /DIR keeps the
    # layout identical either way, and root resolution below handles both.
    Write-Host "Installing the unified installer into $InstallRoot ..."
    $proc = Start-Process -FilePath $paths[$exeInstaller] `
        -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/TYPE=devel", "/DIR=`"$InstallRoot`"" `
        -Wait -PassThru
    if ($proc.ExitCode -ne 0) {
        Write-Host "Admin install exited with $($proc.ExitCode) - retrying in portable mode (no machine ENV/registry)."
        $proc = Start-Process -FilePath $paths[$exeInstaller] `
            -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/TYPE=devel", "/PORTABLE=1", "/DIR=`"$InstallRoot`"" `
            -Wait -PassThru
    }
    if ($proc.ExitCode -ne 0) {
        Write-Host "::error::Inno installer exited with $($proc.ExitCode)."
        exit 1
    }
} else {
    Write-Host "Installing runtime and devel MSIs..."
    Start-Process msiexec -ArgumentList "/i", "`"$($paths[$runtimeMsi])`"", "/qn", "/norestart", "INSTALLLEVEL=100" -Wait
    Start-Process msiexec -ArgumentList "/i", "`"$($paths[$develMsi])`"", "/qn", "/norestart", "INSTALLLEVEL=100" -Wait
}

# Resolve the install root the same way the workflows always have: the
# machine environment variable registered by the installer, then the
# fixed default. The exe path with /DIR= lands exactly on $InstallRoot,
# so both formats converge on the same layout contract.
$registered = [System.Environment]::GetEnvironmentVariable("GSTREAMER_1_0_ROOT_MSVC_X86_64", "Machine")
if ($registered -and (Test-Path (Join-Path $registered "bin\gst-launch-1.0.exe"))) {
    $gstRoot = $registered
} elseif (Test-Path (Join-Path $InstallRoot "bin\gst-launch-1.0.exe")) {
    $gstRoot = $InstallRoot
} else {
    Write-Host "::error::GStreamer installed but $InstallRoot\bin\gst-launch-1.0.exe not found."
    exit 1
}

$gstBin = Join-Path $gstRoot "bin"
$pkgPath = Join-Path $gstRoot "lib\pkgconfig"
$gstPlugins = Join-Path $gstRoot "lib\gstreamer-1.0"
$gstScanner = Join-Path $gstRoot "libexec\gstreamer-1.0\gst-plugin-scanner.exe"

$report = & (Join-Path $gstBin "gst-launch-1.0.exe") --version 2>&1 | Select-Object -First 1
Write-Host "Installed: $report"

echo "PKG_CONFIG_PATH=$pkgPath" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
echo "GSTREAMER_1_0_ROOT_MSVC_X86_64=$gstRoot" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
echo "GST_PLUGIN_SYSTEM_PATH=$gstPlugins" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
echo "GST_PLUGIN_PATH=$gstPlugins" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
echo "GST_PLUGIN_SCANNER=$gstScanner" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
echo "GST_REGISTRY=$env:TEMP\gst-registry.bin" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
echo $gstBin | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
