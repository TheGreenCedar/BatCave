[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Path
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ($env:BATCAVE_WINDOWS_SIGNING_PROFILE -cne "production") {
    throw "Tauri's custom Windows signer is release-only and requires the production signing profile."
}

$required = @(
    "BATCAVE_ARTIFACT_SIGNING_DLIB",
    "BATCAVE_ARTIFACT_SIGNING_METADATA",
    "BATCAVE_SIGNTOOL_PATH"
)
foreach ($name in $required) {
    $value = [Environment]::GetEnvironmentVariable($name)
    if ([string]::IsNullOrWhiteSpace($value) -or -not (Test-Path -LiteralPath $value -PathType Leaf)) {
        throw "Required release signing input $name is missing."
    }
}

$target = [System.IO.Path]::GetFullPath($Path)
if (-not (Test-Path -LiteralPath $target -PathType Leaf)) {
    throw "Tauri requested signing for a missing file: $target"
}

$existing = Get-AuthenticodeSignature -LiteralPath $target
if ($existing.Status -eq [System.Management.Automation.SignatureStatus]::Valid -and
    $existing.SignerCertificate.Subject -cmatch '^CN=Albert Najjar(?:,|$)' -and
    $null -ne $existing.TimeStamperCertificate) {
    Write-Host "Retained existing verified BatCave signature on $([System.IO.Path]::GetFileName($target))."
    return
}
if ($existing.Status -ne [System.Management.Automation.SignatureStatus]::NotSigned) {
    throw "Refusing to replace an existing unexpected signature on $target ($($existing.Status))."
}

& $env:BATCAVE_SIGNTOOL_PATH sign /v /fd SHA256 `
    /tr "http://timestamp.acs.microsoft.com/" /td SHA256 `
    /dlib $env:BATCAVE_ARTIFACT_SIGNING_DLIB `
    /dmdf $env:BATCAVE_ARTIFACT_SIGNING_METADATA `
    /d "BatCave Monitor" $target
if ($LASTEXITCODE -ne 0) {
    throw "Artifact Signing failed for $target with exit code $LASTEXITCODE."
}

$signed = Get-AuthenticodeSignature -LiteralPath $target
if ($signed.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
    $signed.SignerCertificate.Subject -cnotmatch '^CN=Albert Najjar(?:,|$)' -or
    $null -eq $signed.TimeStamperCertificate) {
    throw "Artifact Signing did not produce the required publisher and timestamp on $target."
}
