param(
    [ValidateSet("x86", "x64", "ARM64")]
    [string]$Platform = "x64",
    [switch]$NoBuild,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$AppArgs
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$solutionPath = Join-Path $repoRoot "BatCave.slnx"
$projectPath = Join-Path $repoRoot "BatCave/BatCave.csproj"

if (-not $NoBuild) {
    dotnet build $solutionPath
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

if ($AppArgs.Count -gt 0) {
    dotnet run --project $projectPath -p:Platform=$Platform -- @AppArgs
} else {
    dotnet run --project $projectPath -p:Platform=$Platform
}

exit $LASTEXITCODE
