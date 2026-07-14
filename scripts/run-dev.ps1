[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$WebOnly,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$AppArgs
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$appRoot = Join-Path $repoRoot "src/BatCave.App"

Push-Location $appRoot
try {
    if ($WebOnly) {
        Write-Warning "Browser fixture mode is layout-only. Do not use it for product screenshots or verification; capture the native Tauri window with Computer Use."
        npm run dev
    }
    elseif ($AppArgs.Count -gt 0) {
        npm run tauri -- dev @AppArgs
    }
    else {
        npm run tauri -- dev
    }

    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
