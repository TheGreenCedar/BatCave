[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$SkipBundle,
    [switch]$BenchmarkGate,
    [ValidateSet("x86", "x64", "ARM64")]
    [string]$BenchmarkPlatform = "x64",
    [int]$BenchmarkTicks = 120,
    [int]$BenchmarkSleepMs = 1000,
    [string]$BenchmarkBaselineJsonPath = "",
    [string]$BenchmarkBaselineArtifactPath = "",
    [string]$BenchmarkMinSpeedupMultiplier = "0.90",
    [string]$BenchmarkMaxP95Ms = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$appRoot = Join-Path $repoRoot "src/BatCave.App"
$cargoManifest = Join-Path $appRoot "src-tauri/Cargo.toml"
$tauriBuildScript = "tauri:build:windows"

function Run-Step {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $true)]
        [scriptblock]$ScriptBlock
    )

    Write-Host "==> $Name"
    & $ScriptBlock
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Push-Location $appRoot
try {
    npm run verify
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    cargo fmt --manifest-path "$cargoManifest" --check
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    cargo check --manifest-path "$cargoManifest"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    cargo test --manifest-path "$cargoManifest"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    Run-Step "Rust benchmark smoke" {
        Push-Location $repoRoot
        try {
            cargo run --manifest-path "$cargoManifest" -- --benchmark --ticks 2 --sleep-ms 0 --strict --max-p95-ms 10000
        }
        finally {
            Pop-Location
        }
    }

    if ($BenchmarkGate.IsPresent) {
        Run-Step "Rust benchmark regression gate" {
            Push-Location $repoRoot
            try {
                $gateScript = Join-Path $repoRoot "scripts/run-benchmark-gate.ps1"
                $gateArgs = @{
                    BenchmarkHost = "core"
                    Platform = $BenchmarkPlatform
                    Ticks = $BenchmarkTicks
                    SleepMs = $BenchmarkSleepMs
                    NoBuild = $true
                }

                if (-not [string]::IsNullOrWhiteSpace($BenchmarkBaselineJsonPath)) {
                    $gateArgs["BaselineJsonPath"] = $BenchmarkBaselineJsonPath
                }
                if (-not [string]::IsNullOrWhiteSpace($BenchmarkBaselineArtifactPath)) {
                    $gateArgs["BaselineArtifactPath"] = $BenchmarkBaselineArtifactPath
                }
                if (-not [string]::IsNullOrWhiteSpace($BenchmarkMinSpeedupMultiplier)) {
                    $gateArgs["MinSpeedupMultiplier"] = $BenchmarkMinSpeedupMultiplier
                }
                if (-not [string]::IsNullOrWhiteSpace($BenchmarkMaxP95Ms)) {
                    $gateArgs["MaxP95Ms"] = $BenchmarkMaxP95Ms
                }

                & $gateScript @gateArgs
            }
            finally {
                Pop-Location
            }
        }
    }

    if (-not $SkipBundle) {
        npm run $tauriBuildScript
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
    }
}
finally {
    Pop-Location
}

exit 0
