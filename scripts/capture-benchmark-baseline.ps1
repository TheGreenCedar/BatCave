[CmdletBinding(PositionalBinding = $false)]
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
    [string]$OutputDirectory = ""
)

$ErrorActionPreference = "Stop"

function Parse-BenchmarkJson {
    param([string]$OutputText)

    $start = $OutputText.IndexOf("{")
    $end = $OutputText.LastIndexOf("}")
    if ($start -lt 0 -or $end -lt $start) {
        throw "Unable to locate benchmark JSON payload in output."
    }
    return $OutputText.Substring($start, ($end - $start + 1)) | ConvertFrom-Json
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

if ([string]::IsNullOrWhiteSpace($MachineClass)) {
    $MachineClass = $env:COMPUTERNAME
}
if ([string]::IsNullOrWhiteSpace($MachineClass)) {
    $MachineClass = "local"
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
$runScript = Join-Path $PSScriptRoot "run-benchmark.ps1"
$runArgs = @(
    "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $runScript,
    "-BenchmarkHost", $BenchmarkHost,
    "-Platform", $Platform,
    "-MachineClass", $MachineClass,
    "-WorkloadProfile", $WorkloadProfile,
    "-WarmupTicks", "$WarmupTicks",
    "-Ticks", "$MeasuredTicks",
    "-SleepMs", "$SleepMs",
    "-Repeats", "$RepeatCount"
)

Write-Host "Capturing benchmark protocol v4 baseline ($BenchmarkHost)..."
$previousErrorActionPreference = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    $raw = & powershell @runArgs 2>&1 | ForEach-Object { "$_" } | Out-String
    $exitCode = $LASTEXITCODE
}
finally {
    $ErrorActionPreference = $previousErrorActionPreference
}
if ($exitCode -ne 0) {
    throw "Benchmark baseline failed with exit code $exitCode.`n$raw"
}
$summary = Parse-BenchmarkJson -OutputText $raw

$binaryPath = Join-Path $repoRoot "src/BatCave.App/src-tauri/target/release/batcave-monitor-cli.exe"
$binarySha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
$baseSha = (& git -C $repoRoot rev-parse HEAD | Out-String).Trim()
if ((& git -C $repoRoot status --porcelain | Out-String).Trim()) {
    $baseSha = "$baseSha-dirty"
}

$artifact = [ordered]@{
    format_version = 4
    captured_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    base_sha = $baseSha
    binary_sha256 = $binarySha256
    host = $summary.host
    measurement_origin = $summary.measurement_origin
    evidence_scope = $summary.evidence_scope
    whole_app_measured = $summary.whole_app_measured
    live_command = $summary.live_command
    command_transport = $summary.command_transport
    serialization_scope = $summary.serialization_scope
    latency_gate_metric = $summary.latency_gate_metric
    platform = $summary.platform
    architecture = $summary.architecture
    machine_class = $summary.machine_class
    workload_profile = $summary.workload_profile
    warmup_ticks = $summary.warmup_ticks
    measured_ticks = $summary.measured_ticks
    inter_command_delay_ms = $summary.inter_command_delay_ms
    repeat_count = $summary.repeat_count
    baseline_selection = "median-by-live-command-p95"
    median_collection_p95_ms = $summary.median_collection_p95_ms
    median_publication_p95_ms = $summary.median_publication_p95_ms
    median_serialization_p95_ms = $summary.median_serialization_p95_ms
    median_live_command_p95_ms = $summary.median_live_command_p95_ms
    baseline_summary = $summary
    baseline_summary_path = $baselineSummaryPath
}

Write-JsonUtf8NoBom -Value $artifact -Path $artifactPath
Write-JsonUtf8NoBom -Value $summary -Path $baselineSummaryPath

Write-Host "Baseline artifact written:"
Write-Host "  $artifactPath"
Write-Host "Baseline summary written:"
Write-Host "  $baselineSummaryPath"
