# Pester tests for packaging/windows/generate-scoop-manifest.ps1.
#
# Run with Pester 5 (preinstalled on GitHub Actions Windows runners):
#   Invoke-Pester -CI packaging/windows/generate-scoop-manifest.tests.ps1
#
# The tests are deterministic and offline: they always pass -ZipSha256
# explicitly, so the renderer/validator run without network access.

BeforeAll {
    $scriptPath = Join-Path $PSScriptRoot "generate-scoop-manifest.ps1"
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) ("rivulet-scoop-tests-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $testRoot | Out-Null
    # Keep the child pwsh -File output ANSI-free so assertions and the NUnit
    # report stay hermetic (GitHub Actions sets OutputRendering to Ansi).
    $env:NO_COLOR = "1"
    $env:TERM = "dumb"

    $fixedVersion = "0.65.0-alpha.55"
    $fixedTag = "v0.65.0-alpha.55"
    $fixedSha = "4ec9e53b9d2d5053d38faac207539cca70b6f4bf30851abe845680ecaa671b5b"

    function Invoke-Generator {
        param(
            [string[]]$Arguments,
            [string]$WorkingDir = $testRoot
        )
        $output = & pwsh -NoProfile -File $scriptPath @Arguments 2>&1
        [PSCustomObject]@{
            ExitCode = $LASTEXITCODE
            Output   = ($output | Out-String)
        }
    }
}

AfterAll {
    Remove-Item -Recurse -Force $testRoot -ErrorAction SilentlyContinue
}

Describe "generate-scoop-manifest.ps1" {

    Context "rendering" {

        It "renders a manifest for a fixed version/sha offline" {
            $out = Join-Path $testRoot "render-ok"
            $r = Invoke-Generator @(
                "-Version", $fixedVersion,
                "-ReleaseTag", $fixedTag,
                "-ZipSha256", $fixedSha,
                "-OutDir", $out
            )
            $r.ExitCode | Should -Be 0
            $manifestPath = Join-Path $out "rivulet.json"
            Test-Path $manifestPath | Should -BeTrue
            $m = Get-Content $manifestPath -Raw | ConvertFrom-Json
            $m.version | Should -Be $fixedVersion
            $m.architecture."64bit".url | Should -Be "https://github.com/thoser666/Rivulet/releases/download/$fixedTag/rivulet-windows-x86_64-portable.zip"
            $m.architecture."64bit".hash | Should -Be $fixedSha
            $m.license | Should -Match "MIT"
            $m.homepage | Should -Be "https://github.com/thoser666/Rivulet"
        }

        It "uppercase input is normalized to lowercase" {
            $out = Join-Path $testRoot "render-upper"
            $r = Invoke-Generator @(
                "-Version", $fixedVersion, "-ReleaseTag", $fixedTag,
                "-ZipSha256", $fixedSha.ToUpperInvariant(),
                "-OutDir", $out
            )
            $r.ExitCode | Should -Be 0
            $m = Get-Content (Join-Path $out "rivulet.json") -Raw | ConvertFrom-Json
            $m.architecture."64bit".hash | Should -Be $fixedSha
        }

        It "short hashes are rejected" {
            $out = Join-Path $testRoot "render-short"
            $r = Invoke-Generator @(
                "-Version", $fixedVersion, "-ReleaseTag", $fixedTag,
                "-ZipSha256", "abcd1234",
                "-OutDir", $out
            )
            $r.ExitCode | Should -Not -Be 0
        }
    }

    Context "validation" {

        It "re-verifies an existing manifest byte-exact" {
            $out = Join-Path $testRoot "validate-ok"
            Invoke-Generator @(
                "-Version", $fixedVersion, "-ReleaseTag", $fixedTag,
                "-ZipSha256", $fixedSha, "-OutDir", $out
            ) | Out-Null
            $r = Invoke-Generator @(
                "-ValidateOnly",
                "-ManifestPath", (Join-Path $out "rivulet.json"),
                "-Version", $fixedVersion, "-ReleaseTag", $fixedTag,
                "-ZipSha256", $fixedSha
            )
            $r.ExitCode | Should -Be 0
            $r.Output | Should -Match "OK manifest verified"
        }

        It "detects drift: wrong sha in an existing manifest" {
            $out = Join-Path $testRoot "validate-drift"
            Invoke-Generator @(
                "-Version", $fixedVersion, "-ReleaseTag", $fixedTag,
                "-ZipSha256", $fixedSha, "-OutDir", $out
            ) | Out-Null
            $manifestPath = Join-Path $out "rivulet.json"
            # Tamper: flip one hash nibble.
            $tampered = (Get-Content $manifestPath -Raw) -replace [regex]::Escape($fixedSha.Substring(0, 8)), "00000000"
            Set-Content -LiteralPath $manifestPath -Value $tampered -NoNewline
            $r = Invoke-Generator @(
                "-ValidateOnly",
                "-ManifestPath", $manifestPath,
                "-Version", $fixedVersion, "-ReleaseTag", $fixedTag,
                "-ZipSha256", $fixedSha
            )
            $r.ExitCode | Should -Not -Be 0
            $r.Output | Should -Match "manifest drift"
        }
    }
}
