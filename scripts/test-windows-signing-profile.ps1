[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InputPath,
    [Parameter(Mandatory = $true)]
    [string]$SignToolPath,
    [Parameter(Mandatory = $true)]
    [string]$ThirdPartyInputPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Write-TestCheckpoint {
    param([Parameter(Mandatory = $true)][string]$Name)
    Write-Host "[signing-test] $Name"
}

if (-not $IsWindows) {
    throw "The deterministic Windows signing test profile runs only on Windows."
}
foreach ($required in @($InputPath, $SignToolPath, $ThirdPartyInputPath)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "The Windows signing test profile is missing a required file: $required"
    }
}

$contract = Get-Content -LiteralPath (
    Join-Path $PSScriptRoot "windows-signing/artifact-signing-contract.v1.json"
) -Raw | ConvertFrom-Json
$thirdPartyContracts = @($contract.third_party_resigned_files)
if ($thirdPartyContracts.Count -ne 1 -or
    $thirdPartyContracts[0].name -cne [System.IO.Path]::GetFileName($ThirdPartyInputPath)) {
    throw "The test profile requires the one declared third-party re-signing input."
}
$thirdPartySourceFileHash = Get-FileHash -LiteralPath $ThirdPartyInputPath -Algorithm SHA256
$thirdPartySourceHash = $thirdPartySourceFileHash.Hash.ToLowerInvariant()
if ($thirdPartySourceHash -cne $thirdPartyContracts[0].source_sha256) {
    throw "The test profile received a different third-party source payload."
}
$thirdPartySourceSignature = Get-AuthenticodeSignature -LiteralPath $ThirdPartyInputPath
if ($thirdPartySourceSignature.Status -ne [System.Management.Automation.SignatureStatus]::NotSigned) {
    throw "The third-party test source must be the exact unsigned Foundry SDK payload."
}

$subject = "CN=BatCave Artifact Signing Test"
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) "batcave-signing-test-profile"
if (Test-Path -LiteralPath $testRoot) {
    throw "The fixed signing test-profile directory must be absent before the run: $testRoot"
}
New-Item -ItemType Directory -Path $testRoot | Out-Null
$signedCopy = Join-Path $testRoot "batcave-signing-test.exe"
$signedThirdPartyCopy = Join-Path $testRoot $thirdPartyContracts[0].name
$tamperedCopy = Join-Path $testRoot "batcave-signing-test-tampered.exe"
$certificate = $null

try {
    Write-TestCheckpoint "copying exact inputs"
    Copy-Item -LiteralPath $InputPath -Destination $signedCopy
    Copy-Item -LiteralPath $ThirdPartyInputPath -Destination $signedThirdPartyCopy
    Write-TestCheckpoint "creating test certificate"
    $certificate = New-SelfSignedCertificate `
        -Type Custom `
        -Subject $subject `
        -FriendlyName "BatCave deterministic signing test profile" `
        -KeyAlgorithm RSA `
        -KeyLength 2048 `
        -KeyUsage DigitalSignature `
        -CertStoreLocation "Cert:\CurrentUser\My" `
        -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3")

    Write-TestCheckpoint "signing BatCave input"
    & $SignToolPath sign /v /fd SHA256 /s My /sha1 $certificate.Thumbprint `
        /d "BatCave signing test profile" $signedCopy
    if ($LASTEXITCODE -ne 0) {
        throw "The local Windows signing test profile failed with status $LASTEXITCODE."
    }
    Write-TestCheckpoint "signing pinned third-party input"
    & $SignToolPath sign /v /fd SHA256 /s My /sha1 $certificate.Thumbprint `
        /d "BatCave third-party re-signing test profile" $signedThirdPartyCopy
    if ($LASTEXITCODE -ne 0) {
        throw "The third-party Windows re-signing test failed with status $LASTEXITCODE."
    }
    Write-TestCheckpoint "checking PowerShell signature state"
    $signature = Get-AuthenticodeSignature -LiteralPath $signedCopy
    $allowedIntactStatuses = @(
        [System.Management.Automation.SignatureStatus]::NotTrusted,
        [System.Management.Automation.SignatureStatus]::UnknownError
    )
    if ($signature.Status -notin $allowedIntactStatuses -or
        $null -eq $signature.SignerCertificate -or
        $signature.SignerCertificate.Subject -cne $subject -or
        $null -ne $signature.TimeStamperCertificate) {
        throw "The local test signature was not intact and intentionally untrusted."
    }
    $thirdPartySignature = Get-AuthenticodeSignature -LiteralPath $signedThirdPartyCopy
    if ($thirdPartySignature.Status -notin $allowedIntactStatuses -or
        $null -eq $thirdPartySignature.SignerCertificate -or
        $thirdPartySignature.SignerCertificate.Subject -cne $subject -or
        $null -ne $thirdPartySignature.TimeStamperCertificate) {
        throw "The third-party test signature was not intact and intentionally untrusted."
    }
    $signedThirdPartyFileHash = Get-FileHash -LiteralPath $signedThirdPartyCopy -Algorithm SHA256
    $signedThirdPartyHash = $signedThirdPartyFileHash.Hash.ToLowerInvariant()
    if ($signedThirdPartyHash -ceq $thirdPartySourceHash) {
        throw "The third-party signing test did not change the exact unsigned source bytes."
    }
    Write-TestCheckpoint "confirming SignTool does not trust the test certificate"
    & $SignToolPath verify /pa /all /v $signedCopy *> $null
    if ($LASTEXITCODE -eq 0) {
        throw "The isolated BatCave test signature unexpectedly passed trusted verification."
    }
    & $SignToolPath verify /pa /all /v $signedThirdPartyCopy *> $null
    if ($LASTEXITCODE -eq 0) {
        throw "The isolated third-party test signature unexpectedly passed trusted verification."
    }

    Write-TestCheckpoint "tampering signed fixture"
    Copy-Item -LiteralPath $signedCopy -Destination $tamperedCopy
    $stream = [System.IO.File]::Open(
        $tamperedCopy,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::ReadWrite
    )
    try {
        $stream.Position = [Math]::Floor($stream.Length / 2)
        $original = $stream.ReadByte()
        $stream.Position = [Math]::Floor($stream.Length / 2)
        $stream.WriteByte($original -bxor 1)
        $stream.Flush($true)
    } finally {
        $stream.Dispose()
    }
    $tamperedSignature = Get-AuthenticodeSignature -LiteralPath $tamperedCopy
    if ($tamperedSignature.Status -ne [System.Management.Automation.SignatureStatus]::HashMismatch) {
        throw "The byte-tampered signing test fixture did not report a hash mismatch."
    }
    Write-TestCheckpoint "verifying tampered fixture rejection"
    & $SignToolPath verify /pa /all /v $tamperedCopy *> $null
    if ($LASTEXITCODE -eq 0) {
        throw "The byte-tampered signing test fixture unexpectedly passed verification."
    }
    Write-Host "The isolated test profile re-signed only the pinned unsigned input and rejected tampered bytes."
} finally {
    Write-TestCheckpoint "cleaning certificate and fixtures"
    if ($null -ne $certificate) {
        Remove-Item -LiteralPath "Cert:\CurrentUser\My\$($certificate.Thumbprint)" -ErrorAction SilentlyContinue
        $certificate.Dispose()
    }
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
    Write-TestCheckpoint "cleanup complete"
}
