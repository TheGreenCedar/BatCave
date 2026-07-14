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
    [string]$MinSpeedupMultiplier = "",
    [string]$MaxP95Ms = "",
    [switch]$Strict,
    [switch]$DevBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$cargoManifest = Join-Path $repoRoot "src/BatCave.App/src-tauri/Cargo.toml"
$buildProfile = if ($DevBuild.IsPresent) { "debug" } else { "release" }
$buildDir = Join-Path $repoRoot "src/BatCave.App/src-tauri/target/$buildProfile"
$runningOnWindows = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)
$runtimePlatform = if ($runningOnWindows) { "windows" } else { "linux" }
$architecture = switch ($Platform.ToLowerInvariant()) {
    "x64" { "x86_64" }
    "arm64" { "aarch64" }
    default { $Platform.ToLowerInvariant() }
}
$benchmarkExeName = if ($runningOnWindows) { "batcave-monitor-cli.exe" } else { "batcave-monitor-cli" }
$benchmarkExe = Join-Path $buildDir $benchmarkExeName
$tempBaselinePath = ""

if ([string]::IsNullOrWhiteSpace($MachineClass)) {
    $MachineClass = if ($runningOnWindows) { $env:COMPUTERNAME } else { $env:HOSTNAME }
}
if ([string]::IsNullOrWhiteSpace($MachineClass)) {
    $MachineClass = "local"
}
if (-not [string]::IsNullOrWhiteSpace($BaselineJsonPath) -and -not [string]::IsNullOrWhiteSpace($BaselineArtifactPath)) {
    throw "Specify either -BaselineJsonPath or -BaselineArtifactPath, not both."
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

function Assert-ArtifactValue {
    param(
        [object]$Artifact,
        [string]$Name,
        [object]$Expected
    )

    $actual = $Artifact.$Name
    if ($null -eq $actual -or "$actual" -ne "$Expected") {
        throw "Baseline artifact $Name mismatch. Expected '$Expected', found '$actual'."
    }
}

if (-not [string]::IsNullOrWhiteSpace($BaselineArtifactPath)) {
    if (-not (Test-Path -LiteralPath $BaselineArtifactPath)) {
        throw "Baseline artifact not found: $BaselineArtifactPath"
    }

    $artifact = Get-Content -LiteralPath $BaselineArtifactPath -Raw | ConvertFrom-Json
    Assert-ArtifactValue $artifact "format_version" 4
    Assert-ArtifactValue $artifact "host" $BenchmarkHost
    Assert-ArtifactValue $artifact "measurement_origin" "owned_sampling_engine_refresh_and_protocol_serialization"
    Assert-ArtifactValue $artifact "evidence_scope" "core_runtime_host_only"
    Assert-ArtifactValue $artifact "whole_app_measured" $false
    Assert-ArtifactValue $artifact "live_command" "refresh_now"
    Assert-ArtifactValue $artifact "command_transport" "in_process_bounded_channel"
    Assert-ArtifactValue $artifact "serialization_scope" "runtime_protocol_v3_encode_and_json"
    Assert-ArtifactValue $artifact "latency_gate_metric" "median_live_command_p95_ms"
    Assert-ArtifactValue $artifact "baseline_selection" "median-by-live-command-p95"
    Assert-ArtifactValue $artifact "platform" $runtimePlatform
    Assert-ArtifactValue $artifact "architecture" $architecture
    Assert-ArtifactValue $artifact "machine_class" $MachineClass
    Assert-ArtifactValue $artifact "workload_profile" $WorkloadProfile
    Assert-ArtifactValue $artifact "warmup_ticks" $WarmupTicks
    Assert-ArtifactValue $artifact "measured_ticks" $Ticks
    Assert-ArtifactValue $artifact "inter_command_delay_ms" $SleepMs
    Assert-ArtifactValue $artifact "repeat_count" $Repeats

    $baselineSummary = $artifact.baseline_summary
    if ($null -eq $baselineSummary -and -not [string]::IsNullOrWhiteSpace($artifact.baseline_summary_path)) {
        $summaryPath = [string]$artifact.baseline_summary_path
        if (-not [System.IO.Path]::IsPathRooted($summaryPath)) {
            $summaryPath = Join-Path $repoRoot $summaryPath
        }
        if (Test-Path -LiteralPath $summaryPath) {
            $baselineSummary = Get-Content -LiteralPath $summaryPath -Raw | ConvertFrom-Json
        }
    }
    if ($null -eq $baselineSummary) {
        throw "Baseline artifact missing baseline_summary and a readable baseline_summary_path."
    }

    $tempBaselinePath = Join-Path ([System.IO.Path]::GetTempPath()) ("batcave-baseline-summary-" + [Guid]::NewGuid().ToString("N") + ".json")
    Write-JsonUtf8NoBom -Value $baselineSummary -Path $tempBaselinePath
    $BaselineJsonPath = $tempBaselinePath
}

$cargoArgs = @("build", "--manifest-path", $cargoManifest, "--bin", "batcave-monitor-cli")
if (-not $DevBuild.IsPresent) {
    $cargoArgs += "--release"
}
& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
if (-not (Test-Path -LiteralPath $benchmarkExe)) {
    throw "Benchmark executable not found after $buildProfile build: $benchmarkExe"
}

$benchmarkArgs = @(
    "--benchmark",
    "--platform", $runtimePlatform,
    "--architecture", $architecture,
    "--machine-class", $MachineClass,
    "--workload-profile", $WorkloadProfile,
    "--warmup-ticks", "$WarmupTicks",
    "--ticks", "$Ticks",
    "--sleep-ms", "$SleepMs",
    "--repeats", "$Repeats"
)
if ($Strict.IsPresent) {
    $benchmarkArgs += "--strict"
}
if (-not [string]::IsNullOrWhiteSpace($BaselineJsonPath)) {
    $benchmarkArgs += @("--baseline-json", $BaselineJsonPath)
}
if (-not [string]::IsNullOrWhiteSpace($MinSpeedupMultiplier)) {
    $benchmarkArgs += @("--min-speedup-multiplier", $MinSpeedupMultiplier)
}
elseif ($Strict.IsPresent -and -not [string]::IsNullOrWhiteSpace($BaselineJsonPath)) {
    $benchmarkArgs += @("--min-speedup-multiplier", "0.90")
}
if (-not [string]::IsNullOrWhiteSpace($MaxP95Ms)) {
    $benchmarkArgs += @("--max-p95-ms", $MaxP95Ms)
}

try {
    & $benchmarkExe @benchmarkArgs
    $exitCode = $LASTEXITCODE
}
finally {
    if (-not [string]::IsNullOrWhiteSpace($tempBaselinePath)) {
        Remove-Item -LiteralPath $tempBaselinePath -ErrorAction SilentlyContinue
    }
}

exit $exitCode
