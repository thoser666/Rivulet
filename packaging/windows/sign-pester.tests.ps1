# Pester tests for packaging/windows/sign.ps1.
#
# Run with Pester 5 on a Windows host with the Windows SDK installed:
#   Invoke-Pester packaging/windows/sign-pester.tests.ps1

BeforeAll {
    $signScript = Join-Path $PSScriptRoot "sign.ps1"
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) ("rivulet-sign-tests-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $testRoot | Out-Null

    # Use the committed throwaway self-signed code-signing certificate
    # (test-cert.pfx, password "rivulet-test"). Generating a certificate or
    # trusting one in the root store hangs on GitHub runners, so the cert is
    # checked in and the signature is verified without chain trust below.
    $pfxPath = Join-Path $PSScriptRoot "test-cert.pfx"
    $certBase64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($pfxPath))

    Write-Host "[sign.tests] locating signtool.exe"

    # signtool discovery, identical to sign.ps1 (but without a recursive
    # scan, which is very slow on CI runners with many SDK versions).
    $signtoolPath = $env:SIGNTOOL_PATH
    if (-not $signtoolPath) {
        $signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
        if ($signtool) {
            $signtoolPath = $signtool.Source
        } else {
            $kitRoot = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) "Windows Kits\10\bin"
            $signtoolPath = Get-ChildItem -Path $kitRoot -Directory -ErrorAction SilentlyContinue |
                Sort-Object Name -Descending |
                ForEach-Object { Join-Path $_.FullName "x64\signtool.exe" } |
                Where-Object { Test-Path $_ } |
                Select-Object -First 1
        }
    }
    if (-not $signtoolPath) { throw "signtool.exe not found - install the Windows SDK or set SIGNTOOL_PATH" }
    Write-Host "[sign.tests] using signtool: $signtoolPath"

    # signtool only signs real PE/MSI payloads, so use a copy of cmd.exe.
    # cmd.exe ships with Microsoft's own Authenticode signature; strip it
    # first so the signature our test applies becomes the outer (and only)
    # one. Get-AuthenticodeSignature only reports the outermost signature.
    $testExe = Join-Path $testRoot "test.exe"
    Copy-Item "$env:SystemRoot\System32\cmd.exe" $testExe -Force
    & $signtoolPath remove /s $testExe | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "signtool remove failed for $testExe" }

    # Pester 5 does not expose top-level functions to `It` blocks, so the
    # helper that drives sign.ps1 must live in BeforeAll.
    function Invoke-SignScript {
        param([string[]]$Paths)
        $output = & pwsh -NoProfile -File $signScript -Paths $Paths 2>&1
        [PSCustomObject]@{
            ExitCode = $LASTEXITCODE
            Output   = ($output -join "`n")
        }
    }
}

AfterAll {
    Remove-Item -Recurse -Force $testRoot -ErrorAction SilentlyContinue
}

Describe "sign.ps1" {
    Context "when credentials are missing" {
        It "fails clearly when WINDOWS_CERT_BASE64 is unset" {
            $env:WINDOWS_CERT_BASE64 = $null
            $env:WINDOWS_CERT_PASSWORD = $null
            $result = Invoke-SignScript -Paths @($testExe)
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match "WINDOWS_CERT_BASE64 not set"
        }

        It "fails clearly when WINDOWS_CERT_PASSWORD is unset" {
            $env:WINDOWS_CERT_BASE64 = $certBase64
            $env:WINDOWS_CERT_PASSWORD = $null
            $result = Invoke-SignScript -Paths @($testExe)
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match "WINDOWS_CERT_PASSWORD not set"
        }
    }

    Context "when the target file does not exist" {
        It "fails" {
            $env:WINDOWS_CERT_BASE64 = $certBase64
            $env:WINDOWS_CERT_PASSWORD = "rivulet-test"
            $result = Invoke-SignScript -Paths @((Join-Path $testRoot "missing.exe"))
            $result.ExitCode | Should -Not -Be 0
        }
    }

    Context "with a valid certificate and a real PE file" {
        It "applies a verifiable Authenticode signature" {
            $env:WINDOWS_CERT_BASE64 = $certBase64
            $env:WINDOWS_CERT_PASSWORD = "rivulet-test"
            $env:WINDOWS_TIMESTAMP_URL = "off"
            $result = Invoke-SignScript -Paths @($testExe)
            $result.ExitCode | Should -Be 0

            $sig = Get-AuthenticodeSignature $testExe
            $sig.Status | Should -Not -Be "NotSigned"
            $sig.Status | Should -Not -Be "HashMismatch"
            $sig.SignerCertificate.Subject | Should -Match "Rivulet Pester Test"
        }
    }
}
