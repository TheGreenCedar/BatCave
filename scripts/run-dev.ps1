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
$tauriDevScript = "tauri:dev:windows"

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
        npm run $tauriDevScript -- @AppArgs
    }
    else {
        npm run $tauriDevScript
    }

    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
