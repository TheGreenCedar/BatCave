[CmdletBinding(PositionalBinding = $false)]
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
$projectPath = Join-Path $repoRoot "src/BatCave.App/BatCave.App.csproj"
. "$PSScriptRoot/winui-run-helpers.ps1"

if (-not $NoBuild) {
    dotnet build "$solutionPath" "-p:Platform=$Platform"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

$runArgs = Get-WinUiRunArguments -ProjectPath $projectPath -RuntimePlatform $Platform -CommandArgs $AppArgs
dotnet @runArgs

exit $LASTEXITCODE
