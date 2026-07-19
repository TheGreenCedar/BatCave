[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$EvidenceDirectory
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not $IsWindows) {
    throw "The signed Windows release must be built on Windows."
}
if ($env:BATCAVE_WINDOWS_SIGNING_PROFILE -cne "production") {
    throw "The signed release build requires BATCAVE_WINDOWS_SIGNING_PROFILE=production."
}
foreach ($name in @(
    "BATCAVE_ARTIFACT_SIGNING_DLIB",
    "BATCAVE_ARTIFACT_SIGNING_METADATA",
    "BATCAVE_SIGNTOOL_PATH",
    "TAURI_SIGNING_PRIVATE_KEY",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"
)) {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
        throw "Required protected release input $name is missing."
    }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$appRoot = Join-Path $repoRoot "src/BatCave.App"
$tauriRoot = Join-Path $appRoot "src-tauri"
$targetRoot = Join-Path $tauriRoot "target/release"
$evidenceRoot = [System.IO.Path]::GetFullPath($EvidenceDirectory)
New-Item -ItemType Directory -Path $evidenceRoot -Force | Out-Null

function Invoke-Checked([string]$Executable, [string[]]$Arguments, [string]$WorkingDirectory) {
    Push-Location $WorkingDirectory
    try {
        & $Executable @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "$Executable exited with status $LASTEXITCODE."
        }
    } finally {
        Pop-Location
    }
}

$releaseConfig = "src-tauri/tauri.windows.release.conf.json"
Invoke-Checked "npm" @(
    "run", "tauri", "--", "build", "--no-bundle", "--config", $releaseConfig, "--ci"
) $appRoot
Invoke-Checked "cargo" @(
    "build", "--locked", "--release", "--manifest-path", (Join-Path $tauriRoot "Cargo.toml"),
    "--bin", "batcave-collector-service", "--bin", "batcave-monitor-cli"
) $repoRoot

$innerOwned = @(
    (Join-Path $targetRoot "batcave-collector-service.exe"),
    (Join-Path $targetRoot "batcave-monitor-cli.exe"),
    (Join-Path $targetRoot "batcave-monitor.exe")
)
$signer = Join-Path $tauriRoot "windows/sign-artifact.ps1"
foreach ($file in $innerOwned) {
    & $signer -Path $file
    if ($LASTEXITCODE -ne 0) { throw "Inner PE signing failed for $file." }
}

$nativeRoot = Join-Path $tauriRoot ".generated/foundry-native"
$upstream = @(
    (Join-Path $nativeRoot "Microsoft.AI.Foundry.Local.Core.dll"),
    (Join-Path $nativeRoot "onnxruntime-genai.dll"),
    (Join-Path $nativeRoot "onnxruntime.dll")
)
$inventoryWriter = Join-Path $repoRoot "scripts/write-windows-signature-inventory.ps1"
& $inventoryWriter -Phase inner -Path @($innerOwned + $upstream) `
    -OutputPath (Join-Path $evidenceRoot "windows-signatures-inner.json")

$uninstaller = Join-Path $evidenceRoot "uninstall.exe"
$env:BATCAVE_UNINSTALLER_EXPORT_PATH = $uninstaller
Invoke-Checked "npm" @(
    "run", "tauri", "--", "bundle", "--config", $releaseConfig, "--ci"
) $appRoot

$installers = @(Get-ChildItem -LiteralPath (Join-Path $targetRoot "bundle/nsis") -Filter "*.exe")
if ($installers.Count -ne 1) {
    throw "Expected exactly one generated NSIS installer; found $($installers.Count)."
}
$installer = $installers[0].FullName
$installerDigestBeforeUpdater = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash
Invoke-Checked "npm" @("run", "tauri", "--", "signer", "sign", $installer) $appRoot
$signature = "$installer.sig"
if (-not (Test-Path -LiteralPath $signature -PathType Leaf)) {
    throw "Tauri updater signing did not create $signature."
}
$installerDigestAfterUpdater = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash
if ($installerDigestBeforeUpdater -cne $installerDigestAfterUpdater) {
    throw "Tauri updater signing changed the finalized Authenticode installer bytes."
}

$verifiedCopy = Join-Path $evidenceRoot "updater-verified-copy.exe"
Invoke-Checked "cargo" @(
    "run", "--quiet", "--locked", "--release", "--manifest-path",
    (Join-Path $tauriRoot "Cargo.toml"), "--bin", "batcave-verify-updater-signature", "--",
    $installer, $signature, (Join-Path $tauriRoot "tauri.conf.json"), $verifiedCopy
) $repoRoot
if ((Get-FileHash -LiteralPath $verifiedCopy -Algorithm SHA256).Hash -cne $installerDigestAfterUpdater) {
    throw "Verified updater copy does not match the finalized installer."
}
Remove-Item -LiteralPath $verifiedCopy

$generatedInstaller = Join-Path $targetRoot "nsis/x64/installer.nsi"
& $inventoryWriter -Phase final -Path @($innerOwned + $upstream + @($uninstaller, $installer)) `
    -GeneratedInstallerScript $generatedInstaller `
    -OutputPath (Join-Path $evidenceRoot "windows-signatures-final.json")

$tampered = Join-Path $evidenceRoot "tampered-installer.exe"
Copy-Item -LiteralPath $installer -Destination $tampered
$stream = [System.IO.File]::Open($tampered, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite)
try {
    $offset = [Math]::Floor($stream.Length / 2)
    $stream.Position = $offset
    $original = $stream.ReadByte()
    $stream.Position = $offset
    $stream.WriteByte($original -bxor 1)
    $stream.Flush($true)
} finally {
    $stream.Dispose()
}
& $env:BATCAVE_SIGNTOOL_PATH verify /pa /all /v $tampered *> $null
if ($LASTEXITCODE -eq 0) {
    throw "Byte-tampered installer unexpectedly passed Authenticode verification."
}
Remove-Item -LiteralPath $tampered

$finalInventory = Get-Content -LiteralPath (Join-Path $evidenceRoot "windows-signatures-final.json") -Raw |
    ConvertFrom-Json
$receipt = [ordered]@{
    schema_version = 1
    source_sha = $env:BATCAVE_SOURCE_COMMIT_SHA
    installer = [ordered]@{
        name = [System.IO.Path]::GetFileName($installer)
        sha256 = "sha256:$($installerDigestAfterUpdater.ToLowerInvariant())"
        updater_signature_name = [System.IO.Path]::GetFileName($signature)
        updater_signature_sha256 = "sha256:$((Get-FileHash -LiteralPath $signature -Algorithm SHA256).Hash.ToLowerInvariant())"
    }
    signing_inventory_sha256 = "sha256:$((Get-FileHash -LiteralPath (Join-Path $evidenceRoot "windows-signatures-final.json") -Algorithm SHA256).Hash.ToLowerInvariant())"
    signed_file_count = $finalInventory.files.Count
    tamper_probe = "rejected"
}
$receipt | ConvertTo-Json -Depth 4 |
    Set-Content -LiteralPath (Join-Path $evidenceRoot "windows-release-signing-receipt.json") -Encoding utf8
Write-Host "Built the signed Windows NSIS installer, then generated and verified its updater signature."
