[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,
    [Parameter(Mandatory = $true)]
    [string]$Endpoint,
    [Parameter(Mandatory = $true)]
    [string]$AccountName,
    [Parameter(Mandatory = $true)]
    [string]$CertificateProfileName,
    [Parameter(Mandatory = $true)]
    [string]$CorrelationId
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ($Endpoint -notmatch '^https://[a-z0-9]+\.codesigning\.azure\.net/?$') {
    throw "Artifact Signing endpoint must be an exact regional HTTPS endpoint."
}
foreach ($entry in @($AccountName, $CertificateProfileName, $CorrelationId)) {
    if ([string]::IsNullOrWhiteSpace($entry) -or $entry.Length -gt 128) {
        throw "Artifact Signing metadata contains an invalid bounded value."
    }
}

$metadata = [ordered]@{
    Endpoint = $Endpoint.TrimEnd('/')
    CodeSigningAccountName = $AccountName
    CertificateProfileName = $CertificateProfileName
    CorrelationId = $CorrelationId
    ExcludeCredentials = @(
        "EnvironmentCredential"
        "WorkloadIdentityCredential"
        "ManagedIdentityCredential"
        "SharedTokenCacheCredential"
        "VisualStudioCredential"
        "VisualStudioCodeCredential"
        "AzurePowerShellCredential"
        "AzureDeveloperCliCredential"
        "InteractiveBrowserCredential"
    )
}

$parent = Split-Path -Parent ([System.IO.Path]::GetFullPath($OutputPath))
New-Item -ItemType Directory -Path $parent -Force | Out-Null
$metadata | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $OutputPath -Encoding utf8
"BATCAVE_ARTIFACT_SIGNING_METADATA=$([System.IO.Path]::GetFullPath($OutputPath))" |
    Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
Write-Host "Prepared Artifact Signing metadata for AzureCliCredential-only authentication."
