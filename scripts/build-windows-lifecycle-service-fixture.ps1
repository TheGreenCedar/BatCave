[CmdletBinding(PositionalBinding = $false)]
param(
    [Parameter(Mandatory = $true)]
    [string]$SourceRoot
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$planPath = Join-Path $repoRoot "src/BatCave.App/src-tauri/src/windows_lifecycle_proof_plan.v1.json"
$plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json
$fixturePlan = $plan.incompatible_service_fixture
$fixtureSourceCommit = [string]$fixturePlan.build_source_commit_sha
$fixtureProductVersion = [string]$fixturePlan.product_version
$artifactRoot = Join-Path $repoRoot "artifacts/windows-lifecycle-proof"
$targetRoot = Join-Path $artifactRoot "incompatible-service-target"
$fixturePath = Join-Path $repoRoot ([string]$fixturePlan.relative_path)

$resolvedSourceRoot = (Resolve-Path -LiteralPath $SourceRoot).Path
$sourceManifest = Join-Path $resolvedSourceRoot "src/BatCave.App/src-tauri/Cargo.toml"
if (-not (Test-Path -LiteralPath $sourceManifest -PathType Leaf)) {
    throw "The fixture source root does not contain the BatCave Cargo manifest."
}

$sourceCommit = (git -C $resolvedSourceRoot rev-parse HEAD).Trim().ToLowerInvariant()
if ($LASTEXITCODE -ne 0 -or $sourceCommit -cne $fixtureSourceCommit) {
    throw "The fixture source root is not at the fixed source commit."
}
$sourceStatus = git -C $resolvedSourceRoot status --porcelain=v1 --untracked-files=normal
if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrWhiteSpace(($sourceStatus -join "`n"))) {
    throw "The fixture source worktree must be clean."
}
if (Test-Path -LiteralPath $fixturePath) {
    $retainedFixture = Get-Item -LiteralPath $fixturePath
    $retainedFixtureSha256 = (Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($retainedFixture.Length -ne [long]$fixturePlan.size -or
        $retainedFixtureSha256 -cne [string]$fixturePlan.sha256 -or
        $retainedFixture.VersionInfo.ProductVersion -cne $fixtureProductVersion) {
        throw "The retained fixture does not match the pinned plan."
    }
}

New-Item -ItemType Directory -Path $artifactRoot -Force | Out-Null
$previousTauriConfig = [Environment]::GetEnvironmentVariable("TAURI_CONFIG", "Process")
$previousSourceCommit = [Environment]::GetEnvironmentVariable("BATCAVE_SOURCE_COMMIT_SHA", "Process")
$previousTargetDir = [Environment]::GetEnvironmentVariable("CARGO_TARGET_DIR", "Process")
try {
    $env:TAURI_CONFIG = "{`"version`":`"$fixtureProductVersion`"}"
    Remove-Item -Path Env:BATCAVE_SOURCE_COMMIT_SHA -ErrorAction SilentlyContinue
    $env:CARGO_TARGET_DIR = $targetRoot
    cargo build --locked --release --manifest-path $sourceManifest --bin batcave-collector-service
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}
finally {
    if ($null -eq $previousTauriConfig) {
        Remove-Item -Path Env:TAURI_CONFIG -ErrorAction SilentlyContinue
    }
    else {
        $env:TAURI_CONFIG = $previousTauriConfig
    }
    if ($null -eq $previousSourceCommit) {
        Remove-Item -Path Env:BATCAVE_SOURCE_COMMIT_SHA -ErrorAction SilentlyContinue
    }
    else {
        $env:BATCAVE_SOURCE_COMMIT_SHA = $previousSourceCommit
    }
    if ($null -eq $previousTargetDir) {
        Remove-Item -Path Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    }
    else {
        $env:CARGO_TARGET_DIR = $previousTargetDir
    }
}

$builtFixture = Join-Path $targetRoot "release/batcave-collector-service.exe"
if (-not (Test-Path -LiteralPath $builtFixture -PathType Leaf)) {
    throw "The incompatible service fixture was not produced."
}
$builtFixtureItem = Get-Item -LiteralPath $builtFixture
$builtProductVersion = $builtFixtureItem.VersionInfo.ProductVersion
if ($builtProductVersion -cne $fixtureProductVersion) {
    throw "The built incompatible service fixture ProductVersion is not the fixed value."
}
$builtFixtureSha256 = (Get-FileHash -LiteralPath $builtFixture -Algorithm SHA256).Hash.ToLowerInvariant()
if ($builtFixtureItem.Length -ne [long]$fixturePlan.size -or $builtFixtureSha256 -cne [string]$fixturePlan.sha256) {
    throw "The rebuilt fixture does not match the pinned plan identity."
}
if (Test-Path -LiteralPath $fixturePath) {
    Write-Host "The retained fixture and cache-assisted build match the pinned plan."
}
else {
    Copy-Item -LiteralPath $builtFixture -Destination $fixturePath
}
$fixture = Get-Item -LiteralPath $fixturePath
$productVersion = $fixture.VersionInfo.ProductVersion
if ($productVersion -cne $fixtureProductVersion) {
    throw "The incompatible service fixture ProductVersion is not the fixed value."
}
$fixtureSha256 = (Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash.ToLowerInvariant()
$cargoLockSha256 = (Get-FileHash -LiteralPath (Join-Path $resolvedSourceRoot "src/BatCave.App/src-tauri/Cargo.lock") -Algorithm SHA256).Hash.ToLowerInvariant()

[ordered]@{
    build_source_commit_sha = $fixtureSourceCommit
    relative_path = [string]$fixturePlan.relative_path
    size = $fixture.Length
    sha256 = $fixtureSha256
    product_version = $productVersion
    cargo_lock_sha256 = $cargoLockSha256
    rustc = (rustc --version)
    cargo = (cargo --version)
} | ConvertTo-Json -Compress
