[CmdletBinding(PositionalBinding = $false)]
param(
    [Alias("Host")]
    [ValidateSet("core")]
    [string]$BenchmarkHost = "core",
    [ValidateSet("x86", "x64", "ARM64")]
    [string]$Platform = "x64",
    [int]$Ticks = 120,
    [int]$SleepMs = 1000,
    [string]$BaselineJsonPath = "",
    [string]$BaselineArtifactPath = "",
    [string]$MinSpeedupMultiplier = "0.90",
    [string]$MaxP95Ms = "",
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

function Write-JsonUtf8NoBom {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value,

        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $encoding = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 30), $encoding)
}

$hasBaselineJson = -not [string]::IsNullOrWhiteSpace($BaselineJsonPath)
$hasBaselineArtifact = -not [string]::IsNullOrWhiteSpace($BaselineArtifactPath)
$hasBaseline = $hasBaselineJson -or $hasBaselineArtifact
$hasP95Budget = -not [string]::IsNullOrWhiteSpace($MaxP95Ms)

if ($hasBaselineJson -and $hasBaselineArtifact) {
    throw "Specify either -BaselineJsonPath or -BaselineArtifactPath, not both."
}

if (-not $hasBaseline -and -not $hasP95Budget) {
    throw "Benchmark gate requires -BaselineJsonPath, -BaselineArtifactPath, or -MaxP95Ms."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot "artifacts\benchmarks"
}

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null

$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$reportPath = Join-Path $OutputDirectory "gate-$BenchmarkHost-$timestamp.json"
$scriptPath = Join-Path $PSScriptRoot "run-benchmark.ps1"

$benchmarkArgs = @(
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    $scriptPath,
    "-BenchmarkHost",
    $BenchmarkHost,
    "-Platform",
    $Platform,
    "-Ticks",
    "$Ticks",
    "-SleepMs",
    "$SleepMs",
    "-Strict"
)

if ($hasBaselineJson) {
    $benchmarkArgs += @("-BaselineJsonPath", $BaselineJsonPath)
}
if ($hasBaselineArtifact) {
    $benchmarkArgs += @("-BaselineArtifactPath", $BaselineArtifactPath)
}
if ($hasBaseline -and -not [string]::IsNullOrWhiteSpace($MinSpeedupMultiplier)) {
    $benchmarkArgs += @("-MinSpeedupMultiplier", $MinSpeedupMultiplier)
}
if ($hasP95Budget) {
    $benchmarkArgs += @("-MaxP95Ms", $MaxP95Ms)
}
if ($NoBuild.IsPresent) {
    $benchmarkArgs += @("-NoBuild")
}

$previousErrorActionPreference = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    $raw = & powershell @benchmarkArgs 2>&1 | ForEach-Object { "$_" } | Out-String
    $exitCode = $LASTEXITCODE
}
finally {
    $ErrorActionPreference = $previousErrorActionPreference
}

$summary = $null
if (-not [string]::IsNullOrWhiteSpace($raw)) {
    try {
        $summary = Parse-BenchmarkJson -OutputText $raw
    }
    catch {
        if ($exitCode -eq 0) {
            throw
        }
    }
}

$report = [ordered]@{
    format_version = 1
    captured_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    host = $BenchmarkHost
    platform = $Platform
    ticks = $Ticks
    sleep_ms = $SleepMs
    strict = $true
    baseline_json_path = $BaselineJsonPath
    baseline_artifact_path = $BaselineArtifactPath
    min_speedup_multiplier = if ($hasBaseline) { $MinSpeedupMultiplier } else { "" }
    max_p95_ms = $MaxP95Ms
    exit_code = $exitCode
    strict_passed = if ($summary -ne $null) { [bool]$summary.strict_passed } else { $false }
    benchmark_summary = $summary
}

Write-JsonUtf8NoBom -Value $report -Path $reportPath

$trimmed = $raw.TrimEnd()
if ($trimmed.Length -gt 0) {
    Write-Host $trimmed
}

Write-Host "Benchmark gate report written:"
Write-Host "  $reportPath"

if ($exitCode -ne 0) {
    exit $exitCode
}

exit 0
