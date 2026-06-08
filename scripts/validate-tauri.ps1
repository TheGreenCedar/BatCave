[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$SkipBundle
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
