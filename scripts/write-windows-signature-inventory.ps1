[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("inner", "final")]
    [string]$Phase,
    [Parameter(Mandatory = $true)]
    [string[]]$Path,
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,
    [string]$GeneratedInstallerScript
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not $IsWindows) {
    throw "Windows signature inventory can only be produced on Windows."
}
if ($env:BATCAVE_WINDOWS_SIGNING_PROFILE -cne "production") {
    throw "Release signature inventory requires the production signing profile."
}
if ([string]::IsNullOrWhiteSpace($env:BATCAVE_SIGNTOOL_PATH) -or
    -not (Test-Path -LiteralPath $env:BATCAVE_SIGNTOOL_PATH -PathType Leaf)) {
    throw "BATCAVE_SIGNTOOL_PATH must identify the digest-pinned SignTool executable."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$contractPath = Join-Path $PSScriptRoot "windows-signing/artifact-signing-contract.v1.json"
$contract = Get-Content -LiteralPath $contractPath -Raw | ConvertFrom-Json

function Get-CertificateSha256([System.Security.Cryptography.X509Certificates.X509Certificate2]$Certificate) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([Convert]::ToHexString($sha.ComputeHash($Certificate.RawData))).ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function Resolve-NsisValue([string]$Value, [hashtable]$Defines) {
    $resolved = $Value
    for ($iteration = 0; $iteration -lt 8; $iteration++) {
        $match = [regex]::Match($resolved, '\$\{([A-Za-z0-9_]+)\}')
        if (-not $match.Success) { return $resolved }
        $key = $match.Groups[1].Value
        if (-not $Defines.ContainsKey($key)) {
            throw "Generated NSIS script contains an unresolved PE source macro: $key"
        }
        $resolved = $resolved.Replace($match.Value, [string]$Defines[$key])
    }
    throw "Generated NSIS script macro expansion exceeded the fixed bound."
}

function Get-GeneratedPeInputs([string]$ScriptPath) {
    $script = [System.IO.Path]::GetFullPath($ScriptPath)
    if (-not (Test-Path -LiteralPath $script -PathType Leaf)) {
        throw "Generated NSIS script is missing: $script"
    }
    $defines = @{}
    $lines = Get-Content -LiteralPath $script
    foreach ($line in $lines) {
        $match = [regex]::Match($line, '^\s*!define\s+([A-Za-z0-9_]+)\s+"([^"]*)"\s*$')
        if ($match.Success) { $defines[$match.Groups[1].Value] = $match.Groups[2].Value }
    }

    $inputs = [System.Collections.Generic.List[string]]::new()
    foreach ($line in $lines) {
        if ($line -notmatch '^\s*File(?:\s|$)') { continue }
        $matches = [regex]::Matches($line, '"([^"]+)"')
        if ($matches.Count -eq 0) { continue }
        $source = Resolve-NsisValue $matches[$matches.Count - 1].Groups[1].Value $defines
        if ([System.IO.Path]::GetExtension($source) -notin @(".exe", ".dll")) { continue }
        if (-not [System.IO.Path]::IsPathRooted($source)) {
            $source = Join-Path (Split-Path -Parent $script) $source
        }
        $inputs.Add([System.IO.Path]::GetFullPath($source))
    }
    return $inputs
}

function Get-Disposition([string]$Name) {
    if ($contract.batcave_owned_files -ccontains $Name) {
        return $(if ($Name -ceq "uninstall.exe") { "generated_signed" } else { "batcave_signed" })
    }
    if ($Name -cmatch '^BatCave\.Monitor_.+_x64-setup\.exe$') {
        return "generated_signed"
    }
    if (@($contract.third_party_resigned_files.name) -ccontains $Name) {
        return "third_party_resigned"
    }
    if ($contract.upstream_files -ccontains $Name) {
        return "upstream_preserved"
    }
    throw "Packaged PE is outside the signed-file allowlist: $Name"
}

function Test-ExpectedSubject([string]$Disposition, [string]$Subject) {
    if ($Disposition -in @("batcave_signed", "generated_signed", "third_party_resigned")) {
        return $Subject -cmatch '^CN=Albert Najjar(?:,|$)'
    }
    foreach ($pattern in $contract.upstream_publishers) {
        if ($Subject -cmatch $pattern) { return $true }
    }
    return $false
}

$allPaths = [System.Collections.Generic.List[string]]::new()
foreach ($candidate in $Path) { $allPaths.Add([System.IO.Path]::GetFullPath($candidate)) }
if (-not [string]::IsNullOrWhiteSpace($GeneratedInstallerScript)) {
    foreach ($candidate in (Get-GeneratedPeInputs $GeneratedInstallerScript)) {
        if (-not $allPaths.Contains($candidate)) { $allPaths.Add($candidate) }
    }
}

$records = [System.Collections.Generic.List[object]]::new()
$seenNames = @{}
foreach ($candidate in ($allPaths | Sort-Object)) {
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw "Packaged PE is missing: $candidate"
    }
    $name = [System.IO.Path]::GetFileName($candidate)
    if ($seenNames.ContainsKey($name)) {
        $prior = $seenNames[$name]
        if ((Get-FileHash -LiteralPath $prior -Algorithm SHA256).Hash -cne
            (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash) {
            throw "Two packaged PE files share the name $name with different bytes."
        }
        continue
    }
    $seenNames[$name] = $candidate
    $disposition = Get-Disposition $name
    $signature = Get-AuthenticodeSignature -LiteralPath $candidate
    if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
        throw "Packaged PE $name has invalid Authenticode status $($signature.Status)."
    }
    if ($null -eq $signature.SignerCertificate -or $null -eq $signature.TimeStamperCertificate) {
        throw "Packaged PE $name must have a signer and an RFC3161 timestamp."
    }
    if (-not (Test-ExpectedSubject $disposition $signature.SignerCertificate.Subject)) {
        throw "Packaged PE $name has unexpected publisher $($signature.SignerCertificate.Subject)."
    }
    $originalSha256 = $null
    if ($disposition -ceq "third_party_resigned") {
        $sourceContracts = @(
            $contract.third_party_resigned_files | Where-Object { $_.name -ceq $name }
        )
        if ($sourceContracts.Count -ne 1 -or
            $sourceContracts[0].source_sha256 -cnotmatch '^[0-9a-f]{64}$') {
            throw "Packaged re-signed PE $name does not have one exact source contract."
        }
        $originalSha256 = "sha256:$($sourceContracts[0].source_sha256)"
        $signedDigest = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($signedDigest -ceq $sourceContracts[0].source_sha256) {
            throw "Packaged re-signed PE $name still matches its unsigned source bytes."
        }
    }

    $verificationOutput = & $env:BATCAVE_SIGNTOOL_PATH verify /pa /all /v $candidate 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "SignTool trust verification failed for $name with exit code $LASTEXITCODE."
    }
    $timestampMatch = $verificationOutput |
        Select-String -Pattern '^The signature is timestamped:\s*(.+)$' |
        Select-Object -First 1
    if ($null -eq $timestampMatch) {
        throw "SignTool did not report a verified timestamp for $name."
    }
    $timestamp = [DateTimeOffset]::Parse(
        $timestampMatch.Matches[0].Groups[1].Value,
        [Globalization.CultureInfo]::GetCultureInfo("en-US")
    ).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")

    $records.Add([ordered]@{
        name = $name
        sha256 = "sha256:$((Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant())"
        disposition = $disposition
        original_sha256 = $originalSha256
        publisher_subject = $signature.SignerCertificate.Subject
        certificate_sha256 = "sha256:$(Get-CertificateSha256 $signature.SignerCertificate)"
        rfc3161_timestamp_utc = $timestamp
        timestamp_certificate_sha256 = "sha256:$(Get-CertificateSha256 $signature.TimeStamperCertificate)"
        authenticode_status = "valid"
        signtool_policy = "pa_all"
    })
}

$required = @(
    "Microsoft.AI.Foundry.Local.Core.dll",
    "batcave-collector-service.exe",
    "batcave-monitor-cli.exe",
    "batcave-monitor.exe",
    "onnxruntime-genai.dll",
    "onnxruntime.dll"
)
if ($Phase -ceq "final") {
    $required += @("MicrosoftEdgeWebView2RuntimeInstaller.exe", "uninstall.exe")
    if (-not ($records.name | Where-Object { $_ -cmatch '^BatCave\.Monitor_.+_x64-setup\.exe$' })) {
        throw "Final signature inventory is missing the versioned NSIS installer."
    }
}
foreach ($name in $required) {
    if ($records.name -cnotcontains $name) {
        throw "$Phase signature inventory is missing required PE $name."
    }
}

$sourceSha = $env:BATCAVE_SOURCE_COMMIT_SHA
if ($sourceSha -cnotmatch '^[0-9a-f]{40}$') {
    throw "BATCAVE_SOURCE_COMMIT_SHA must be the exact release commit."
}
$inventory = [ordered]@{
    schema_version = 1
    profile = "production"
    phase = $Phase
    source_sha = $sourceSha
    publisher = [ordered]@{
        display_name = $contract.publisher.display_name
        required_subject = $contract.publisher.subject
    }
    timestamp = [ordered]@{
        protocol = $contract.timestamp.protocol
        url = $contract.timestamp.url
        digest = $contract.timestamp.digest
    }
    files = @($records | Sort-Object name)
}

$output = [System.IO.Path]::GetFullPath($OutputPath)
$parent = Split-Path -Parent $output
New-Item -ItemType Directory -Path $parent -Force | Out-Null
$inventory | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $output -Encoding utf8
Write-Host "Verified and recorded $($records.Count) $Phase Windows PE signatures."
