[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$SkipBundle
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$appRoot = Join-Path $repoRoot "src/BatCave.App"
$cargoManifest = Join-Path $appRoot "src-tauri/Cargo.toml"

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

    if (-not $SkipBundle) {
        npm run tauri:build
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
    }
}
finally {
    Pop-Location
}

exit 0
