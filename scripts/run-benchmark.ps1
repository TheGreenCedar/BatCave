param(
    [Alias("Host")]
    [ValidateSet("core", "winui")]
    [string]$BenchmarkHost = "core",
    [ValidateSet("x86", "x64", "ARM64")]
    [string]$Platform = "x64",
    [int]$Ticks = 120,
    [int]$SleepMs = 1000,
    [string]$BaselineJsonPath = "",
    [string]$BaselineArtifactPath = "",
    [string]$MinSpeedupMultiplier = "",
    [string]$MaxP95Ms = "",
    [switch]$Strict,
    [switch]$NoBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$solutionPath = Join-Path $repoRoot "BatCave.slnx"
$coreProjectPath = Join-Path $repoRoot "src/BatCave.Bench/BatCave.Bench.csproj"
$winUiProjectPath = Join-Path $repoRoot "src/BatCave.App/BatCave.App.csproj"
. "$PSScriptRoot/winui-run-helpers.ps1"

if (-not [string]::IsNullOrWhiteSpace($BaselineJsonPath) -and -not [string]::IsNullOrWhiteSpace($BaselineArtifactPath)) {
    throw "Specify either -BaselineJsonPath or -BaselineArtifactPath, not both."
}

function Resolve-BaselineSummaryPath {
    param(
        [string]$SummaryPath,
        [string]$ArtifactPath,
        [string]$BenchmarkHostName,
        [string]$HostPlatform,
        [string]$RepositoryRoot,
        [int]$RequestedTicks,
        [int]$RequestedSleepMs
    )

    if ([string]::IsNullOrWhiteSpace($ArtifactPath)) {
        return @{
            BaselinePath = $SummaryPath
            TempPath = ""
        }
    }

    if (-not (Test-Path -LiteralPath $ArtifactPath)) {
        throw "Baseline artifact not found: $ArtifactPath"
    }

    $artifact = Get-Content -LiteralPath $ArtifactPath -Raw | ConvertFrom-Json
    if ($artifact -eq $null) {
        throw "Baseline artifact is empty: $ArtifactPath"
    }

    if (-not [string]::IsNullOrWhiteSpace($artifact.host) -and $artifact.host -ne $BenchmarkHostName) {
        throw "Baseline artifact host mismatch. Expected '$BenchmarkHostName', found '$($artifact.host)'."
    }

    if (-not [string]::IsNullOrWhiteSpace($artifact.platform) -and $artifact.platform -ne $HostPlatform) {
        throw "Baseline artifact platform mismatch. Expected '$HostPlatform', found '$($artifact.platform)'."
    }

    if ($artifact.measured_ticks -and [int]$artifact.measured_ticks -ne $RequestedTicks) {
        throw "Baseline artifact measured_ticks mismatch. Expected '$RequestedTicks', found '$($artifact.measured_ticks)'."
    }

    if ($artifact.sleep_ms -and [int]$artifact.sleep_ms -ne $RequestedSleepMs) {
        throw "Baseline artifact sleep_ms mismatch. Expected '$RequestedSleepMs', found '$($artifact.sleep_ms)'."
    }

    if (-not [string]::IsNullOrWhiteSpace($artifact.baseline_summary_path)) {
        $resolvedSummaryPath = [string]$artifact.baseline_summary_path
        if (-not [System.IO.Path]::IsPathRooted($resolvedSummaryPath)) {
            $resolvedSummaryPath = Join-Path $RepositoryRoot $resolvedSummaryPath
        }

        if (-not (Test-Path -LiteralPath $resolvedSummaryPath)) {
            throw "Baseline artifact references missing baseline_summary_path: $resolvedSummaryPath"
        }

        return @{
            BaselinePath = $resolvedSummaryPath
            TempPath = ""
        }
    }

    if ($artifact.baseline_summary -eq $null) {
        throw "Baseline artifact missing 'baseline_summary' and 'baseline_summary_path'."
    }

    $tmpPath = Join-Path ([System.IO.Path]::GetTempPath()) ("batcave-baseline-summary-" + [Guid]::NewGuid().ToString("N") + ".json")
    $artifact.baseline_summary | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $tmpPath -Encoding UTF8

    return @{
        BaselinePath = $tmpPath
        TempPath = $tmpPath
    }
}

if (-not $NoBuild) {
    dotnet build "$solutionPath" "-p:Platform=$Platform"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

$resolvedBaseline = Resolve-BaselineSummaryPath `
    -SummaryPath $BaselineJsonPath `
    -ArtifactPath $BaselineArtifactPath `
    -BenchmarkHostName $BenchmarkHost `
    -HostPlatform $Platform `
    -RepositoryRoot $repoRoot `
    -RequestedTicks $Ticks `
    -RequestedSleepMs $SleepMs

$effectiveBaselineJsonPath = [string]$resolvedBaseline.BaselinePath
$tempBaselinePath = [string]$resolvedBaseline.TempPath

$strictArgs = @()
if ($Strict.IsPresent) {
    $strictArgs = @("--strict")
}

$compareArgs = @()
if (-not [string]::IsNullOrWhiteSpace($effectiveBaselineJsonPath)) {
    $compareArgs += @("--baseline-json", "$effectiveBaselineJsonPath")
}

if (-not [string]::IsNullOrWhiteSpace($MinSpeedupMultiplier)) {
    $compareArgs += @("--min-speedup-multiplier", "$MinSpeedupMultiplier")
}
elseif ($Strict.IsPresent -and -not [string]::IsNullOrWhiteSpace($effectiveBaselineJsonPath)) {
    $compareArgs += @("--min-speedup-multiplier", "10")
}

if (-not [string]::IsNullOrWhiteSpace($MaxP95Ms)) {
    $compareArgs += @("--max-p95-ms", "$MaxP95Ms")
}

$coreArgs = @("--ticks", "$Ticks", "--sleep-ms", "$SleepMs") + $strictArgs + $compareArgs
$winUiArgs = @("--benchmark", "--benchmark-host", "winui", "--ticks", "$Ticks", "--sleep-ms", "$SleepMs") + $strictArgs + $compareArgs

if ($BenchmarkHost -eq "core") {
    try {
        dotnet run --project "$coreProjectPath" -- @coreArgs
    }
    finally {
        if (-not [string]::IsNullOrWhiteSpace($tempBaselinePath)) {
            Remove-Item -LiteralPath $tempBaselinePath -ErrorAction SilentlyContinue
        }
    }
}
else {
    try {
        $runArgs = Get-WinUiRunArguments -ProjectPath $winUiProjectPath -RuntimePlatform $Platform -CommandArgs $winUiArgs
        dotnet @runArgs
    }
    finally {
        if (-not [string]::IsNullOrWhiteSpace($tempBaselinePath)) {
            Remove-Item -LiteralPath $tempBaselinePath -ErrorAction SilentlyContinue
        }
    }
}

exit $LASTEXITCODE
