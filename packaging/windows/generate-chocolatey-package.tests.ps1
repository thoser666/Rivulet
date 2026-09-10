# Pester tests for packaging/windows/generate-chocolatey-package.ps1.
#
# Run with Pester 5 (preinstalled on GitHub Actions Windows runners):
#   Invoke-Pester -CI packaging/windows/generate-chocolatey-package.tests.ps1
#
# The tests are deterministic and offline: they always pass -ZipSha256
# explicitly, so the renderer/validator run on any OS without gh or network.

BeforeAll {
    $scriptPath = Join-Path $PSScriptRoot "generate-chocolatey-package.ps1"
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) ("rivulet-choco-tests-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $testRoot | Out-Null
    # Keep the child pwsh -File output ANSI-free so assertions and the NUnit
    # report stay hermetic (GitHub Actions sets OutputRendering to Ansi).
    $env:NO_COLOR = "1"
    $env:TERM = "dumb"

    $fixedVersion = "0.65.0-alpha.55"
    $fixedTag = "v0.65.0-alpha.55"
    $fixedSha = "4ec9e53b9d2d5053d38faac207539cca70b6f4bf30851abe845680ecaa671b5b"

    function Invoke-Generator {
        param([string[]]$Arguments)
        $output = & pwsh -NoProfile -File $scriptPath @Arguments 2>&1
        [PSCustomObject]@{
            ExitCode = $LASTEXITCODE
            Output   = ($output -join "`n")
        }
    }

    function New-Package {
        param([string]$OutDir)
        return Invoke-Generator -Arguments @(
            "-Version", $fixedVersion,
            "-ReleaseTag", $fixedTag,
            "-ZipSha256", $fixedSha,
            "-OutDir", $OutDir
        )
    }

    function PackageDir {
        param([string]$OutDir)
        return (Join-Path $OutDir "rivulet")
    }

    AfterAll {
        Remove-Item -Recurse -Force $testRoot -ErrorAction SilentlyContinue
    }
}

Describe "generate-chocolatey-package.ps1" {
    It "generates a nuspec and install script from an explicit SHA" {
        $out = Join-Path $testRoot "gen"
        $result = New-Package -OutDir $out
        $result.ExitCode | Should -Be 0
        (Join-Path (PackageDir $out) "rivulet.nuspec") | Should -Exist
        (Join-Path (PackageDir $out) "tools/chocolateyInstall.ps1") | Should -Exist
    }

    It "normalizes the prerelease version for Chocolatey (dots are forbidden)" {
        $out = Join-Path $testRoot "version"
        New-Package -OutDir $out | Out-Null
        $nuspec = Get-Content (Join-Path (PackageDir $out) "rivulet.nuspec") -Raw
        $nuspec | Should -Match "<version>0\.65\.0-alpha55</version>"
        # Only the <version> element must be dot-free; the releaseNotes URL
        # legitimately keeps the canonical tag with the dot.
        ($nuspec -replace '<releaseNotes>[^<]*</releaseNotes>', '') | Should -Not -Match "alpha\.55"
    }

    It "pins the portable ZIP by SHA-256 and canonical release URL" {
        $out = Join-Path $testRoot "pin"
        New-Package -OutDir $out | Out-Null
        $install = Get-Content (Join-Path (PackageDir $out) "tools/chocolateyInstall.ps1") -Raw
        $install | Should -Match "checksum64\s*=\s*'$fixedSha'"
        $install | Should -Match "checksumType64\s*=\s*'sha256'"
        $install | Should -Match "url64bit\s*=\s*'https://github\.com/thoser666/Rivulet/releases/download/v0\.65\.0-alpha\.55/rivulet-windows-x86_64-portable\.zip'"
    }

    It "is deterministic across renders" {
        $outA = Join-Path $testRoot "det-a"
        $outB = Join-Path $testRoot "det-b"
        New-Package -OutDir $outA | Out-Null
        New-Package -OutDir $outB | Out-Null
        (Get-FileHash (Join-Path (PackageDir $outA) "rivulet.nuspec")).Hash |
            Should -Be (Get-FileHash (Join-Path (PackageDir $outB) "rivulet.nuspec")).Hash
        (Get-FileHash (Join-Path (PackageDir $outA) "tools/chocolateyInstall.ps1")).Hash |
            Should -Be (Get-FileHash (Join-Path (PackageDir $outB) "tools/chocolateyInstall.ps1")).Hash
    }

    It "rejects a malformed SHA-256" {
        $out = Join-Path $testRoot "badsha"
        $result = Invoke-Generator -Arguments @(
            "-Version", $fixedVersion,
            "-ReleaseTag", $fixedTag,
            "-ZipSha256", "not-a-hash",
            "-OutDir", $out
        )
        $result.ExitCode | Should -Not -Be 0
        $result.Output | Should -Match "64-char lowercase hex"
    }

    It "rejects an out-of-contract version" {
        $out = Join-Path $testRoot "badver"
        $result = Invoke-Generator -Arguments @(
            "-Version", "1.2.3.4",
            "-ReleaseTag", "v1.2.3.4",
            "-ZipSha256", $fixedSha,
            "-OutDir", $out
        )
        $result.ExitCode | Should -Not -Be 0
        $result.Output | Should -Match "Version must look like"
    }

    It "ValidateOnly detects drift in an edited install script" {
        $out = Join-Path $testRoot "drift"
        New-Package -OutDir $out | Out-Null
        $installPath = Join-Path (PackageDir $out) "tools/chocolateyInstall.ps1"
        (Get-Content $installPath -Raw) -replace 'sha256', 'md5' |
            Set-Content $installPath -NoNewline
        $result = Invoke-Generator -Arguments @(
            "-ValidateOnly",
            "-ManifestPath", (PackageDir $out),
            "-Version", $fixedVersion,
            "-ReleaseTag", $fixedTag,
            "-ZipSha256", $fixedSha
        )
        $result.ExitCode | Should -Not -Be 0
        $result.Output | Should -Match "package drift"
    }

    It "ValidateOnly passes an unmodified package directory" {
        $out = Join-Path $testRoot "valid"
        New-Package -OutDir $out | Out-Null
        $result = Invoke-Generator -Arguments @(
            "-ValidateOnly",
            "-ManifestPath", (PackageDir $out),
            "-Version", $fixedVersion,
            "-ReleaseTag", $fixedTag,
            "-ZipSha256", $fixedSha
        )
        $result.ExitCode | Should -Be 0
        $result.Output | Should -Match "OK package verified"
    }
}
