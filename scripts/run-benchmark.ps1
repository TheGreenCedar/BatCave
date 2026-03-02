param(
    [Alias("Host")]
    [ValidateSet("core", "winui")]
    [string]$BenchmarkHost = "core",
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
$coreProjectPath = Join-Path $repoRoot "BatCave.Bench/BatCave.Bench.csproj"
$winUiProjectPath = Join-Path $repoRoot "BatCave/BatCave.csproj"

if (-not $NoBuild) {
    dotnet build "$solutionPath"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

$strictArgs = @()
if ($Strict.IsPresent) {
    $strictArgs = @("--strict")
}

$coreArgs = @("--ticks", "$Ticks", "--sleep-ms", "$SleepMs") + $strictArgs
$winUiArgs = @("--benchmark", "--ticks", "$Ticks", "--sleep-ms", "$SleepMs") + $strictArgs

if ($BenchmarkHost -eq "core") {
    dotnet run --project "$coreProjectPath" -- @coreArgs
}
else {
    dotnet run --project "$winUiProjectPath" "-p:Platform=$Platform" -- @winUiArgs
}

exit $LASTEXITCODE
