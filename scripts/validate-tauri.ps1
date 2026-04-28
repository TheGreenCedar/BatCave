[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$SkipBundle
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$appRoot = Join-Path $repoRoot "src/BatCave.App"

Push-Location $appRoot
try {
    npm run verify
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    cargo fmt --manifest-path ".\src-tauri\Cargo.toml" --check
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    cargo check --manifest-path ".\src-tauri\Cargo.toml"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    cargo test --manifest-path ".\src-tauri\Cargo.toml"
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
