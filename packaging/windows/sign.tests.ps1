# Pester tests for packaging/windows/sign.ps1.
#
# Run with Pester 5 on a Windows host with the Windows SDK installed:
#   Invoke-Pester packaging/windows/sign.tests.ps1

BeforeAll {
    $signScript = Join-Path $PSScriptRoot "sign.ps1"
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) ("rivulet-sign-tests-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $testRoot | Out-Null

    # signtool only signs real PE/MSI payloads, so use a copy of cmd.exe.
    $testExe = Join-Path $testRoot "test.exe"
    Copy-Item "$env:SystemRoot\System32\cmd.exe" $testExe -Force

    # Self-signed code signing certificate (current-user store, no admin).
    $cert = New-SelfSignedCertificate `
        -Subject "CN=Rivulet Pester Test" `
        -Type CodeSigningCert `
        -CertStoreLocation Cert:\CurrentUser\My `
        -KeyExportPolicy Exportable `
        -NotAfter (Get-Date).AddDays(1)
    $pfxPath = Join-Path $testRoot "cert.pfx"
    $password = ConvertTo-SecureString -String "rivulet-test" -Force -AsPlainText
    Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $password | Out-Null
    $certBase64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($pfxPath))

    # Trust the certificate in the current-user root store so that
    # `signtool verify /pa` can validate the chain without elevation.
    $cerPath = Join-Path $testRoot "cert.cer"
    Export-Certificate -Cert $cert -FilePath $cerPath | Out-Null
    Import-Certificate -FilePath $cerPath -CertStoreLocation Cert:\CurrentUser\Root | Out-Null

    # signtool discovery, identical to sign.ps1.
    $signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($signtool) {
        $signtoolPath = $signtool.Source
    } else {
        $kitRoot = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) "Windows Kits\10\bin"
        $signtoolPath = Get-ChildItem -Path $kitRoot -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\' } |
            Sort-Object FullName -Descending |
            Select-Object -First 1 -ExpandProperty FullName
    }
}

AfterAll {
    Remove-Item -Recurse -Force $testRoot -ErrorAction SilentlyContinue
    Get-ChildItem Cert:\CurrentUser\My, Cert:\CurrentUser\Root |
        Where-Object { $_.Subject -eq "CN=Rivulet Pester Test" } |
        Remove-Item -ErrorAction SilentlyContinue
}

function Invoke-SignScript {
    param([string[]]$Paths)
    $output = & pwsh -NoProfile -File $signScript -Paths $Paths 2>&1
    [PSCustomObject]@{
        ExitCode = $LASTEXITCODE
        Output   = ($output -join "`n")
    }
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
