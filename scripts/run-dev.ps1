[CmdletBinding(PositionalBinding = $false)]
param(
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
    if ($WebOnly) {
        Write-Warning "Browser fixture mode is layout-only. Do not use it for product screenshots or verification; capture the native Tauri window with Computer Use."
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
