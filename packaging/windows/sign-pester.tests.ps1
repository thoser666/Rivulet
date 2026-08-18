# Pester tests for packaging/windows/sign.ps1.
#
# Run with Pester 5 on a Windows host with the Windows SDK installed:
#   Invoke-Pester packaging/windows/sign-pester.tests.ps1

BeforeAll {
    $signScript = Join-Path $PSScriptRoot "sign.ps1"
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) ("rivulet-sign-tests-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $testRoot | Out-Null

    Write-Host "[sign.tests] preparing self-signed certificate"

    # signtool only signs real PE/MSI payloads, so use a copy of cmd.exe.
    $testExe = Join-Path $testRoot "test.exe"
    Copy-Item "$env:SystemRoot\System32\cmd.exe" $testExe -Force

    # Build a self-signed code-signing certificate via the .NET API instead
    # of New-SelfSignedCertificate: the PKI cmdlets hang/crash under
    # PowerShell 7.5 on the GitHub Windows runner
    # (PowerShell/PowerShell#25189), which left this job stuck forever.
    $rsa = [System.Security.Cryptography.RSA]::Create(2048)
    $req = [System.Security.Cryptography.X509Certificates.CertificateRequest]::new(
        "CN=Rivulet Pester Test",
        $rsa,
        [System.Security.Cryptography.HashAlgorithmName]::SHA256,
        [System.Security.Cryptography.RSASignaturePadding]::Pkcs1)
    $req.CertificateExtensions.Add(
        [System.Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($false, $false, 0, $false))
    $codeSigningOids = [System.Security.Cryptography.OidCollection]::new()
    $codeSigningOids.Add([System.Security.Cryptography.Oid]::new("1.3.6.1.5.5.7.3.3")) | Out-Null
    $req.CertificateExtensions.Add(
        [System.Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($codeSigningOids, $false))
    $cert = $req.CreateSelfSigned([DateTimeOffset]::Now.AddDays(-1), [DateTimeOffset]::Now.AddDays(1))

    $pfxPath = Join-Path $testRoot "cert.pfx"
    [IO.File]::WriteAllBytes(
        $pfxPath,
        $cert.Export([System.Security.Cryptography.X509Certificates.X509ContentType]::Pfx, "rivulet-test"))
    $certBase64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($pfxPath))

    # Trust the certificate in the current-user root store so that
    # `signtool verify /pa` can validate the chain without elevation.
    # Use certutil instead of Import-Certificate: installing into the root
    # store triggers a "Security Warning" UI prompt, which hangs indefinitely
    # on CI runners; certutil -f performs the same install without UI.
    $cerPath = Join-Path $testRoot "cert.cer"
    [IO.File]::WriteAllBytes(
        $cerPath,
        $cert.Export([System.Security.Cryptography.X509Certificates.X509ContentType]::Cert))
    certutil -user -addstore -f Root $cerPath | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "certutil -addstore Root failed (exit $LASTEXITCODE)" }

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
    # Root store removal is also UI-free via certutil. The signing cert
    # itself is ephemeral (.NET in-memory) and never entered a store.
    certutil -user -delstore Root "Rivulet Pester Test" | Out-Null
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
            $result = Invoke-SignScript -Paths @($testExe)
            $result.ExitCode | Should -Be 0

            & $signtoolPath verify /pa /v $testExe | Out-Null
            $LASTEXITCODE | Should -Be 0
        }
    }
}
