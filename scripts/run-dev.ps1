[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$NoBuild,
    [switch]$WebOnly,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$AppArgs
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$appRoot = Join-Path $repoRoot "src/BatCave.App"

Push-Location $appRoot
try {
    if (-not $NoBuild) {
        npm run build
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
    }

    if ($WebOnly) {
        npm run dev
    }
    elseif ($AppArgs.Count -gt 0) {
        npm run tauri:dev -- @AppArgs
    }
    else {
        npm run tauri:dev
    }

    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
