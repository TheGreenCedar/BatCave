param(
    [ValidateSet("core")]
    [string]$BenchmarkHost = "core",
    [ValidateSet("x86", "x64", "ARM64")]
    [string]$Platform = "x64",
    [string]$WorkloadProfile = "fixed-default",
    [string]$MachineClass = "",
    [int]$WarmupTicks = 30,
    [int]$MeasuredTicks = 120,
    [int]$SleepMs = 1000,
    [ValidateRange(1, 20)]
    [int]$RepeatCount = 5,
    [string]$OutputDirectory = "",
    [switch]$NoBuild
)

$ErrorActionPreference = "Stop"

function Parse-BenchmarkJson {
    param([string]$OutputText)

    $start = $OutputText.IndexOf("{")
    $end = $OutputText.LastIndexOf("}")
    if ($start -lt 0 -or $end -lt $start) {
        throw "Unable to locate benchmark JSON payload in output."
    }

    $json = $OutputText.Substring($start, ($end - $start + 1))
    return $json | ConvertFrom-Json
}

function Invoke-BenchmarkRun {
    param(
        [int]$Ticks,
        [switch]$RunNoBuild
    )

    $scriptPath = Join-Path $PSScriptRoot "run-benchmark.ps1"
    $benchmarkArgs = @{
        BenchmarkHost = $BenchmarkHost
        Platform = $Platform
        Ticks = $Ticks
        SleepMs = $SleepMs
    }

    if ($RunNoBuild.IsPresent) {
        $benchmarkArgs["NoBuild"] = $true
    }

    $raw = & $scriptPath @benchmarkArgs | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "Benchmark run failed with exit code $LASTEXITCODE.`n$raw"
    }

    return (Parse-BenchmarkJson -OutputText $raw)
}

if ([string]::IsNullOrWhiteSpace($MachineClass)) {
    $MachineClass = $env:COMPUTERNAME
}

$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot "artifacts\benchmarks"
}

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null

$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$artifactPrefix = "baseline-$BenchmarkHost-$timestamp"
$artifactPath = Join-Path $OutputDirectory "$artifactPrefix.json"
$baselineSummaryPath = Join-Path $OutputDirectory "$artifactPrefix.summary.json"

Write-Host "Capturing baseline benchmark ($BenchmarkHost) with fixed profile '$WorkloadProfile'..."
Write-Host "Warmup: $WarmupTicks ticks, measured: $MeasuredTicks ticks, sleep: $SleepMs ms, repeats: $RepeatCount."

$runNoBuild = $NoBuild.IsPresent
if (-not $NoBuild.IsPresent) {
    Write-Host "Building solution before baseline capture..."
    dotnet build (Join-Path $repoRoot "BatCave.slnx")
    if ($LASTEXITCODE -ne 0) {
        throw "dotnet build failed with exit code $LASTEXITCODE."
    }

    $runNoBuild = $true
}

Write-Host "Running warmup window..."
$null = Invoke-BenchmarkRun -Ticks $WarmupTicks -RunNoBuild:$runNoBuild

$runs = @()
for ($index = 0; $index -lt $RepeatCount; $index++) {
    Write-Host ("Running measured repeat {0}/{1}..." -f ($index + 1), $RepeatCount)
    $run = Invoke-BenchmarkRun -Ticks $MeasuredTicks -RunNoBuild:$runNoBuild
    $runs += $run
}

$medianIndex = [Math]::Floor(($runs.Count - 1) / 2)
$baselineSummary = $runs | Sort-Object tick_p95_ms | Select-Object -Index $medianIndex

$artifact = [ordered]@{
    format_version = 1
    captured_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    machine_class = $MachineClass
    host = $BenchmarkHost
    platform = $Platform
    workload_profile = $WorkloadProfile
    warmup_ticks = $WarmupTicks
    measured_ticks = $MeasuredTicks
    sleep_ms = $SleepMs
    repeat_count = $RepeatCount
    baseline_selection = "median-by-tick-p95"
    baseline_summary = $baselineSummary
    baseline_summary_path = $baselineSummaryPath
    runs = $runs
}

$artifact | ConvertTo-Json -Depth 30 | Set-Content -Path $artifactPath -Encoding UTF8
$baselineSummary | ConvertTo-Json -Depth 30 | Set-Content -Path $baselineSummaryPath -Encoding UTF8

Write-Host "Baseline artifact written:"
Write-Host "  $artifactPath"
Write-Host "Baseline summary for --baseline-json written:"
Write-Host "  $baselineSummaryPath"
