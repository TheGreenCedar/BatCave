[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InputPath,
    [Parameter(Mandatory = $true)]
    [string]$SignToolPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not $IsWindows) {
    throw "The deterministic Windows signing test profile runs only on Windows."
}
foreach ($required in @($InputPath, $SignToolPath)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "The Windows signing test profile is missing a required file: $required"
    }
}

$subject = "CN=BatCave Artifact Signing Test"
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) "batcave-signing-test-profile"
if (Test-Path -LiteralPath $testRoot) {
    throw "The fixed signing test-profile directory must be absent before the run: $testRoot"
}
New-Item -ItemType Directory -Path $testRoot | Out-Null
$signedCopy = Join-Path $testRoot "batcave-signing-test.exe"
$tamperedCopy = Join-Path $testRoot "batcave-signing-test-tampered.exe"
$certificate = $null
$rootStore = $null

try {
    Copy-Item -LiteralPath $InputPath -Destination $signedCopy
    $certificate = New-SelfSignedCertificate `
        -Type Custom `
        -Subject $subject `
        -FriendlyName "BatCave deterministic signing test profile" `
        -KeyAlgorithm RSA `
        -KeyLength 2048 `
        -KeyUsage DigitalSignature `
        -CertStoreLocation "Cert:\CurrentUser\My" `
        -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3")

    $rootStore = [System.Security.Cryptography.X509Certificates.X509Store]::new(
        [System.Security.Cryptography.X509Certificates.StoreName]::Root,
        [System.Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser
    )
    $rootStore.Open([System.Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
    $rootStore.Add($certificate)

    & $SignToolPath sign /v /fd SHA256 /s My /sha1 $certificate.Thumbprint `
        /d "BatCave signing test profile" $signedCopy
    if ($LASTEXITCODE -ne 0) {
        throw "The local Windows signing test profile failed with status $LASTEXITCODE."
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $signedCopy
    if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
        $signature.SignerCertificate.Subject -cne $subject -or
        $null -ne $signature.TimeStamperCertificate) {
        throw "The local test signature did not retain its isolated test-only contract."
    }
    & $SignToolPath verify /pa /all /v $signedCopy *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "The local test signature did not pass SignTool verification."
    }

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
    & $SignToolPath verify /pa /all /v $tamperedCopy *> $null
    if ($LASTEXITCODE -eq 0) {
        throw "The byte-tampered signing test fixture unexpectedly passed verification."
    }
    Write-Host "The isolated test signing profile accepted exact bytes and rejected tampered bytes."
} finally {
    if ($null -ne $rootStore) {
        if ($null -ne $certificate) { $rootStore.Remove($certificate) }
        $rootStore.Dispose()
    }
    if ($null -ne $certificate) {
        Remove-Item -LiteralPath "Cert:\CurrentUser\My\$($certificate.Thumbprint)" -ErrorAction SilentlyContinue
        $certificate.Dispose()
    }
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
