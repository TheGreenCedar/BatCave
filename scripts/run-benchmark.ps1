param(
    [ValidateSet("x86", "x64", "ARM64")]
    [string]$Platform = "x64",
    [int]$Ticks = 120,
    [int]$SleepMs = 1000,
    [switch]$Strict,
    [switch]$NoBuild
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

$cliArgs = @("--benchmark", "--ticks", "$Ticks", "--sleep-ms", "$SleepMs")
if ($Strict.IsPresent) {
    $cliArgs += "--strict"
}

dotnet run --project $projectPath -p:Platform=$Platform -- @cliArgs
exit $LASTEXITCODE
