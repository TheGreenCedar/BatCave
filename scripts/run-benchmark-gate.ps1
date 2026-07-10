[CmdletBinding(PositionalBinding = $false)]
param(
    [Alias("Host")]
    [ValidateSet("core")]
    [string]$BenchmarkHost = "core",
    [ValidateSet("x86", "x64", "ARM64")]
    [string]$Platform = "x64",
    [string]$MachineClass = "",
    [string]$WorkloadProfile = "fixed-default",
    [int]$WarmupTicks = 30,
    [int]$Ticks = 120,
    [int]$SleepMs = 1000,
    [ValidateRange(1, 20)]
    [int]$Repeats = 5,
    [string]$BaselineJsonPath = "",
    [string]$BaselineArtifactPath = "",
    [string]$MinSpeedupMultiplier = "0.90",
    [string]$MaxP95Ms = "",
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
$reportPath = Join-Path $OutputDirectory "gate-$BenchmarkHost-$timestamp.json"
$scriptPath = Join-Path $PSScriptRoot "run-benchmark.ps1"
$benchmarkArgs = @(
    "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $scriptPath,
    "-BenchmarkHost", $BenchmarkHost,
    "-Platform", $Platform,
    "-MachineClass", $MachineClass,
    "-WorkloadProfile", $WorkloadProfile,
    "-WarmupTicks", "$WarmupTicks",
    "-Ticks", "$Ticks",
    "-SleepMs", "$SleepMs",
    "-Repeats", "$Repeats",
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

$binaryPath = Join-Path $repoRoot "src/BatCave.App/src-tauri/target/release/batcave-monitor-cli.exe"
$binarySha256 = if (Test-Path -LiteralPath $binaryPath) {
    (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
} else {
    ""
}
$candidateSha = (& git -C $repoRoot rev-parse HEAD | Out-String).Trim()
if ((& git -C $repoRoot status --porcelain | Out-String).Trim()) {
    $candidateSha = "$candidateSha-dirty"
}
$report = [ordered]@{
    format_version = 2
    captured_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    candidate_sha = $candidateSha
    binary_sha256 = $binarySha256
    host = $BenchmarkHost
    platform = "windows"
    architecture = $Platform.ToLowerInvariant()
    machine_class = $MachineClass
    workload_profile = $WorkloadProfile
    warmup_ticks = $WarmupTicks
    measured_ticks = $Ticks
    sleep_ms = $SleepMs
    repeat_count = $Repeats
    strict = $true
    baseline_json_path = $BaselineJsonPath
    baseline_artifact_path = $BaselineArtifactPath
    min_speedup_multiplier = if ($hasBaseline) { $MinSpeedupMultiplier } else { "" }
    max_p95_ms = $MaxP95Ms
    exit_code = $exitCode
    strict_passed = if ($null -ne $summary) { [bool]$summary.strict_passed } else { $false }
    speed_ratio = if ($null -ne $summary) { $summary.speed_ratio } else { $null }
    benchmark_summary = $summary
}
Write-JsonUtf8NoBom -Value $report -Path $reportPath

$trimmed = $raw.TrimEnd()
if ($trimmed.Length -gt 0) {
    Write-Host $trimmed
}
Write-Host "Benchmark gate report written:"
Write-Host "  $reportPath"
exit $exitCode
