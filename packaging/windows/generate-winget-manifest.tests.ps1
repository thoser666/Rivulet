# Pester tests for packaging/windows/generate-winget-manifest.ps1.
#
# Run with Pester 5 (preinstalled on GitHub Actions Windows runners):
#   Invoke-Pester -CI packaging/windows/generate-winget-manifest.tests.ps1
#
# The tests are deterministic and offline: they always pass -InstallerSha256
# and -ProductCode explicitly, so the renderer/validator run on any OS
# without an MSI or WindowsInstaller COM.

BeforeAll {
    $scriptPath = Join-Path $PSScriptRoot "generate-winget-manifest.ps1"
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) ("rivulet-winget-tests-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $testRoot | Out-Null
    # Keep the child pwsh -File output ANSI-free so assertions and the NUnit
    # report stay hermetic (GitHub Actions sets OutputRendering to Ansi).
    $env:NO_COLOR = "1"
    $env:TERM = "dumb"

    $fixedVersion = "0.65.0-alpha.55"
    $fixedTag = "v0.65.0-alpha.55"
    $fixedSha = "4EC9E53B9D2D5053D38FAAC207539CCA70B6F4BF30851ABE845680ECAA671B5B"
    $fixedProduct = "A1111111-2222-3333-4444-555555555555"

    function Invoke-Generator {
        param(
            [string[]]$Arguments,
            [string]$WorkingDir = $testRoot
        )
        $output = & pwsh -NoProfile -File $scriptPath @Arguments 2>&1
        [PSCustomObject]@{
            ExitCode = $LASTEXITCODE
            Output   = ($output -join "`n")
        }
    }

    function New-Manifest {
        param([string]$OutDir)
        return Invoke-Generator -Arguments @(
            "-Version", $fixedVersion,
            "-ReleaseTag", $fixedTag,
            "-InstallerSha256", $fixedSha,
            "-ProductCode", $fixedProduct,
            "-OutDir", $OutDir
        )
    }

    function ManifestFilePath {
        param([string]$OutDir, [string]$Id = "Rivulet.Rivulet", [string]$Version = $fixedVersion)
        return (Join-Path (Join-Path (Join-Path $OutDir $Id) $Version) "$Id.yaml")
    }
}

AfterAll {
    Remove-Item -Recurse -Force $testRoot -ErrorAction SilentlyContinue
}

Describe "generate-winget-manifest.ps1" {
    Context "generation (deterministic inputs)" {
        It "renders the singleton manifest into the winget-pkgs layout" {
            $out = Join-Path $testRoot "gen-ok"
            $result = New-Manifest -OutDir $out
            $result.ExitCode | Should -Be 0

            $file = ManifestFilePath -OutDir $out
            Test-Path -LiteralPath $file | Should -Be $true

            $content = Get-Content -LiteralPath $file -Raw
            $content | Should -Match "PackageIdentifier: Rivulet.Rivulet"
            $content | Should -Match "PackageVersion: $fixedVersion"
            $content | Should -Match "ManifestType: singleton"
            $content | Should -Match "ManifestVersion: 1.6.0"
            $content | Should -Match "InstallerUrl: https://github\.com/thoser666/Rivulet/releases/download/$fixedTag/rivulet-windows-x86_64\.msi"
            $content | Should -Match "InstallerSha256: $($fixedSha.ToUpperInvariant())"
            $content | Should -Match "ProductCode: `"\{$fixedProduct\}`""
            $content | Should -Match "UpgradeCode: `"\{A5C1E5E8-7A3B-4C9D-B6E2-9F1D4C7A8B90\}`""
            $content | Should -Match "InstallerType: wix"
            $content | Should -Match "Scope: machine"
            $content | Should -Match "Architecture: x64"
            $content | Should -Match "Moniker: rivulet"
            $content | Should -Match "License: MIT"
        }

        It "writes a UTF-8 file without a BOM" {
            $out = Join-Path $testRoot "gen-nobom"
            New-Manifest -OutDir $out | Out-Null
            $file = ManifestFilePath -OutDir $out
            $bytes = [IO.File]::ReadAllBytes($file)
            # BOM would start EF BB BF. The file must start with "P" (0x50).
            $bytes[0] | Should -Be 0x50
        }

        It "accepts an explicit canonical InstallerUrl" {
            $out = Join-Path $testRoot "gen-url"
            $url = "https://github.com/thoser666/Rivulet/releases/download/v0.66.0.beta.1/rivulet-windows-x86_64.msi"
            $result = Invoke-Generator -Arguments @(
                "-Version", "0.66.0-beta.1",
                "-InstallerUrl", $url,
                "-InstallerSha256", $fixedSha,
                "-ProductCode", $fixedProduct,
                "-OutDir", $out
            )
            $result.ExitCode | Should -Be 0
            $content = Get-Content -LiteralPath (ManifestFilePath -OutDir $out -Version "0.66.0-beta.1") -Raw
            $content | Should -Match "InstallerUrl: $([regex]::Escape($url))"
        }
    }

    Context "input validation" {
        It "rejects a malformed version" {
            $result = Invoke-Generator -Arguments @(
                "-Version", "1.2",
                "-InstallerUrl", "https://github.com/thoser666/Rivulet/releases/download/v1.2/rivulet-windows-x86_64.msi",
                "-InstallerSha256", $fixedSha,
                "-ProductCode", $fixedProduct
            )
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match "Version must look like"
        }

        It "rejects a ReleaseTag that does not match v<version>" {
            $result = Invoke-Generator -Arguments @(
                "-Version", $fixedVersion,
                "-ReleaseTag", "v0.99.0",
                "-InstallerSha256", $fixedSha,
                "-ProductCode", $fixedProduct
            )
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match "must equal"
        }

        It "rejects a non-canonical InstallerUrl" {
            $result = Invoke-Generator -Arguments @(
                "-Version", $fixedVersion,
                "-InstallerUrl", "https://example.com/download/rivulet-windows-x86_64.msi",
                "-InstallerSha256", $fixedSha,
                "-ProductCode", $fixedProduct
            )
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match "InstallerUrl must be a GitHub release asset URL"
        }

        It "rejects a malformed ProductCode GUID" {
            $result = Invoke-Generator -Arguments @(
                "-Version", $fixedVersion,
                "-ReleaseTag", $fixedTag,
                "-InstallerSha256", $fixedSha,
                "-ProductCode", "not-a-guid"
            )
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match "not a valid GUID"
        }

        It "rejects a malformed InstallerSha256" {
            $result = Invoke-Generator -Arguments @(
                "-Version", $fixedVersion,
                "-ReleaseTag", $fixedTag,
                "-InstallerSha256", "zz",
                "-ProductCode", $fixedProduct
            )
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match "64 hex characters"
        }

        It "requires a SHA-256 source when no -MsiPath is given" {
            $result = Invoke-Generator -Arguments @(
                "-Version", $fixedVersion,
                "-ReleaseTag", $fixedTag,
                "-ProductCode", $fixedProduct
            )
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match "Provide -MsiPath"
        }

        It "fails clearly when -MsiPath does not exist" {
            $result = Invoke-Generator -Arguments @(
                "-Version", $fixedVersion,
                "-ReleaseTag", $fixedTag,
                "-MsiPath", (Join-Path $testRoot "missing.msi")
            )
            $result.ExitCode | Should -Not -Be 0
        }
    }

    Context "validation mode (-ValidateOnly)" {
        BeforeEach {
            $script:out = Join-Path $testRoot "validate"
            New-Manifest -OutDir $script:out | Out-Null
            $script:file = ManifestFilePath -OutDir $script:out
        }

        It "accepts an unchanged generated manifest" {
            $result = Invoke-Generator -Arguments @(
                "-ValidateOnly",
                "-ManifestPath", $script:file,
                "-Version", $fixedVersion,
                "-ReleaseTag", $fixedTag,
                "-InstallerSha256", $fixedSha,
                "-ProductCode", $fixedProduct
            )
            $result.ExitCode | Should -Be 0
            $result.Output | Should -Match "validation passed"
        }

        It "rejects a manifest with a tampered InstallerSha256" {
            $tampered = (Get-Content -LiteralPath $script:file -Raw) -replace "InstallerSha256: \S+", "InstallerSha256: 0000000000000000000000000000000000000000000000000000000000000000"
            [IO.File]::WriteAllText($script:file, $tampered, [Text.UTF8Encoding]::new($false))

            $result = Invoke-Generator -Arguments @(
                "-ValidateOnly",
                "-ManifestPath", $script:file,
                "-Version", $fixedVersion,
                "-ReleaseTag", $fixedTag,
                "-InstallerSha256", $fixedSha,
                "-ProductCode", $fixedProduct
            )
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match "mismatch"
        }

        It "rejects a manifest with a wrong ProductCode" {
            $result = Invoke-Generator -Arguments @(
                "-ValidateOnly",
                "-ManifestPath", $script:file,
                "-Version", $fixedVersion,
                "-ReleaseTag", $fixedTag,
                "-InstallerSha256", $fixedSha,
                "-ProductCode", "B2222222-3333-4444-5555-666666666666"
            )
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match "mismatch"
        }
    }

    Context "MsiPath mode" {
        It "computes the SHA-256 of a real file and uses the given ProductCode" {
            $dummy = Join-Path $testRoot "fake.msi"
            [IO.File]::WriteAllText($dummy, "not a real MSI - sha256 only", [Text.UTF8Encoding]::new($false))
            $expectedSha = (Get-FileHash -LiteralPath $dummy -Algorithm SHA256).Hash

            $out = Join-Path $testRoot "gen-msi"
            $shared = "C1D2E3F4-5566-7788-99AA-BBCCDDEEFF00"
            $result = Invoke-Generator -Arguments @(
                "-Version", $fixedVersion,
                "-ReleaseTag", $fixedTag,
                "-MsiPath", $dummy,
                "-ProductCode", $shared,
                "-OutDir", $out
            )
            $result.ExitCode | Should -Be 0
            $result.Output | Should -Match "SHA-256\s*:\s*$expectedSha"
            $content = Get-Content -LiteralPath (ManifestFilePath -OutDir $out) -Raw
            $content | Should -Match "InstallerSha256: $expectedSha"
            $content | Should -Match "ProductCode: `"\{$shared\}`""
        }
    }
}