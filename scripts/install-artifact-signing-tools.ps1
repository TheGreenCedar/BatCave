[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Destination
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not $IsWindows) {
    throw "Artifact Signing tools can only be prepared on Windows."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$contractPath = Join-Path $PSScriptRoot "windows-signing/artifact-signing-contract.v1.json"
$contract = Get-Content -LiteralPath $contractPath -Raw | ConvertFrom-Json

$destinationPath = [System.IO.Path]::GetFullPath($Destination)
if (Test-Path -LiteralPath $destinationPath) {
    if ((Get-ChildItem -LiteralPath $destinationPath -Force | Measure-Object).Count -ne 0) {
        throw "Artifact Signing tools destination must be empty: $destinationPath"
    }
} else {
    New-Item -ItemType Directory -Path $destinationPath | Out-Null
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
foreach ($package in $contract.client_packages) {
    $packageRoot = Join-Path $destinationPath $package.id
    New-Item -ItemType Directory -Path $packageRoot | Out-Null
    $archive = Join-Path $destinationPath "$($package.id).$($package.version).nupkg"
    Invoke-WebRequest -Uri $package.url -OutFile $archive
    $digest = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($digest -cne $package.sha256) {
        throw "$($package.id) digest mismatch: expected $($package.sha256), received $digest"
    }
    [System.IO.Compression.ZipFile]::ExtractToDirectory($archive, $packageRoot)
}

$dlib = Join-Path $destinationPath $contract.paths.artifact_signing_dlib
$signTool = Join-Path $destinationPath $contract.paths.signtool
foreach ($required in @($dlib, $signTool)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Pinned Artifact Signing tool is missing after extraction: $required"
    }
}

"BATCAVE_ARTIFACT_SIGNING_TOOLS=$destinationPath" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
"BATCAVE_ARTIFACT_SIGNING_DLIB=$dlib" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
"BATCAVE_SIGNTOOL_PATH=$signTool" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
Write-Host "Prepared digest-pinned Artifact Signing client and SignTool packages."
