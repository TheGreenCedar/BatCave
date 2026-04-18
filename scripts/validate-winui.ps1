param(
    [string]$Platform = "x64",
    [string]$RunPlatform = "",
    [ValidateSet("core", "winui")]
    [string]$BenchmarkHost = "core",
    [int]$Ticks = 120,
    [int]$SleepMs = 1000,
    [string]$BaselineJsonPath = "",
    [string]$BaselineArtifactPath = "",
    [string]$MinSpeedupMultiplier = "10",
    [string]$MaxP95Ms = "",
    [switch]$RunPerformanceGate,
    [switch]$SkipLaunchSmoke,
    [int]$LaunchTimeoutSeconds = 25
)

$ErrorActionPreference = "Stop"
$isScriptHarness = -not [string]::IsNullOrWhiteSpace($env:FAKE_DOTNET_LOG)
$includeHarnessRuntimeDiagnostics = $env:FAKE_DOTNET_INCLUDE_RUNTIME_DIAGNOSTICS -eq "1"
. "$PSScriptRoot/winui-run-helpers.ps1"

function Assert-LastExitCode {
    param(
        [string]$CommandName
    )

    if ($LASTEXITCODE -ne 0) {
        throw "$CommandName failed with exit code $LASTEXITCODE."
    }
}

function ConvertFrom-DiagnosticJson {
    param(
        [string]$Payload,
        [string]$SurfaceName
    )

    if ([string]::IsNullOrWhiteSpace($Payload)) {
        throw "$SurfaceName produced no output."
    }

    $trimmedPayload = $Payload.Trim()
    $candidates = [System.Collections.Generic.List[string]]::new()
    $candidates.Add($trimmedPayload)

    $firstBrace = $trimmedPayload.IndexOf('{')
    $lastBrace = $trimmedPayload.LastIndexOf('}')
    if ($firstBrace -ge 0 -and $lastBrace -gt $firstBrace) {
        $embeddedJson = $trimmedPayload.Substring($firstBrace, $lastBrace - $firstBrace + 1).Trim()
        if (-not [string]::Equals($embeddedJson, $trimmedPayload, [System.StringComparison]::Ordinal)) {
            $candidates.Add($embeddedJson)
        }
    }

    foreach ($candidate in $candidates) {
        try {
            return $candidate | ConvertFrom-Json
        }
        catch {
        }
    }

    throw "$SurfaceName did not contain a valid JSON payload."
}

function Get-DefaultRunPlatform {
    if (-not [string]::IsNullOrWhiteSpace($env:PROCESSOR_ARCHITECTURE)) {
        switch ($env:PROCESSOR_ARCHITECTURE.ToUpperInvariant()) {
            "ARM64" { return "ARM64" }
            "AMD64" { return "x64" }
            "X86" { return "x86" }
        }
    }

    return "x64"
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

function Clear-StaleBuildProcesses {
    $processNames = @("XamlCompiler", "MakeAppx", "makeappx")

    Get-Process -Name $processNames -ErrorAction SilentlyContinue |
        Stop-Process -Force -ErrorAction SilentlyContinue
}

function Invoke-BuildWithRetry {
    param(
        [string]$BuildPlatform
    )

    Clear-StaleBuildProcesses

    for ($attempt = 1; $attempt -le 2; $attempt++) {
        dotnet build BatCave/BatCave.csproj "-p:Platform=$BuildPlatform"
        if ($LASTEXITCODE -eq 0) {
            return
        }

        if ($attempt -lt 2) {
            Write-Warning "Build attempt $attempt failed. Clearing lingering build processes and retrying once."
            Clear-StaleBuildProcesses
            Start-Sleep -Seconds 1
        }
    }

    Assert-LastExitCode "dotnet build BatCave/BatCave.csproj"
}

function Invoke-TestsWithRetry {
    Clear-StaleBuildProcesses

    for ($attempt = 1; $attempt -le 2; $attempt++) {
        dotnet test BatCave.slnx -m:1
        if ($LASTEXITCODE -eq 0) {
            return
        }

        if ($attempt -lt 2) {
            Write-Warning "Test attempt $attempt failed. Clearing lingering test/build processes and retrying once."
            Clear-StaleBuildProcesses
            Start-Sleep -Seconds 1
        }
    }

    Assert-LastExitCode "dotnet test BatCave.slnx -m:1"
}

function Invoke-LaunchPolicyGateDiagnostics {
    param(
        [string]$ProjectPath,
        [string]$RuntimePlatform
    )

    Write-Host "Verifying launch policy diagnostics surface..."
    $runArgs = Get-WinUiRunArguments -ProjectPath $ProjectPath -RuntimePlatform $RuntimePlatform -CommandArgs @("--print-gate-status")
    $output = dotnet @runArgs
    Assert-LastExitCode "dotnet run -- --print-gate-status"

    $payload = $output | Out-String
    $json = ConvertFrom-DiagnosticJson -Payload $payload -SurfaceName "Launch policy diagnostics surface"
    if ($null -eq $json -or $null -eq $json.passed) {
        throw "Launch policy diagnostics payload is missing expected fields."
    }

    Write-Host "Launch policy diagnostics verified."
}

function Invoke-RuntimeHealthDiagnostics {
    param(
        [string]$ProjectPath,
        [string]$RuntimePlatform
    )

    Write-Host "Verifying runtime health diagnostics surface..."
    $runArgs = Get-WinUiRunArguments -ProjectPath $ProjectPath -RuntimePlatform $RuntimePlatform -CommandArgs @("--print-runtime-health")
    $output = dotnet @runArgs
    Assert-LastExitCode "dotnet run -- --print-runtime-health"

    $payload = $output | Out-String
    $json = ConvertFrom-DiagnosticJson -Payload $payload -SurfaceName "Runtime health diagnostics surface"
    if ($null -eq $json -or $null -eq $json.runtime_loop_enabled) {
        throw "Runtime health diagnostics payload is missing expected fields."
    }

    Write-Host "Runtime health diagnostics verified."
}

function Invoke-WinUiLaunchSmoke {
    param(
        [string]$ProjectPath,
        [string]$RuntimePlatform,
        [int]$TimeoutSeconds
    )

    Write-Host "Running WinUI launch smoke verification..."
    $runner = $null
    $appProcess = $null

    $existingIds = @(Get-Process -Name "BatCave" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)

    try {
        $runner = Start-Process `
            -FilePath "dotnet" `
            -ArgumentList (Get-WinUiRunArguments -ProjectPath $ProjectPath -RuntimePlatform $RuntimePlatform) `
            -PassThru

        $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
        while ((Get-Date) -lt $deadline) {
            $candidates = Get-Process -Name "BatCave" -ErrorAction SilentlyContinue | Where-Object {
                $_.MainWindowHandle -ne 0 -and $existingIds -notcontains $_.Id
            }

            if ($candidates) {
                $appProcess = $candidates | Select-Object -First 1
                break
            }

            if ($runner.HasExited) {
                throw "dotnet run exited before BatCave opened a top-level window."
            }

            Start-Sleep -Milliseconds 250
        }

        if ($null -eq $appProcess) {
            throw "BatCave launch smoke failed: no top-level window detected within ${TimeoutSeconds}s."
        }

        if ([string]::IsNullOrWhiteSpace($appProcess.MainWindowTitle)) {
            throw "BatCave launch smoke failed: main window title was empty."
        }

        Write-Host "Launch smoke verified (PID=$($appProcess.Id), Title='$($appProcess.MainWindowTitle)')."
    }
    finally {
        if ($appProcess -and -not $appProcess.HasExited) {
            Stop-Process -Id $appProcess.Id -Force -ErrorAction SilentlyContinue
        }

        if ($runner -and -not $runner.HasExited) {
            Stop-Process -Id $runner.Id -Force -ErrorAction SilentlyContinue
        }
    }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$projectPath = Join-Path $repoRoot "BatCave/BatCave.csproj"
$resolvedRunPlatform = if ([string]::IsNullOrWhiteSpace($RunPlatform)) { Get-DefaultRunPlatform } else { $RunPlatform }
Push-Location $repoRoot
try {
    Write-Host "Validating WinUI compile path (Platform=$Platform, RunPlatform=$resolvedRunPlatform)..."
    Invoke-BuildWithRetry -BuildPlatform $Platform

    Write-Host "Running solution tests (serialized)..."
    Invoke-TestsWithRetry

    if ($RunPerformanceGate) {
        if ([string]::IsNullOrWhiteSpace($BaselineJsonPath) -and [string]::IsNullOrWhiteSpace($BaselineArtifactPath)) {
            throw "RunPerformanceGate requires -BaselineJsonPath or -BaselineArtifactPath."
        }

        if (-not [string]::IsNullOrWhiteSpace($BaselineJsonPath) -and -not [string]::IsNullOrWhiteSpace($BaselineArtifactPath)) {
            throw "Specify either -BaselineJsonPath or -BaselineArtifactPath, not both."
        }

        Write-Host "Running strict performance gate benchmark..."
        $resolvedBaseline = Resolve-BaselineSummaryPath `
            -SummaryPath $BaselineJsonPath `
            -ArtifactPath $BaselineArtifactPath `
            -BenchmarkHostName $BenchmarkHost `
            -HostPlatform $resolvedRunPlatform `
            -RepositoryRoot $repoRoot `
            -RequestedTicks $Ticks `
            -RequestedSleepMs $SleepMs

        $benchmarkArgs = @{
            BenchmarkHost = $BenchmarkHost
            Platform = $resolvedRunPlatform
            Ticks = $Ticks
            SleepMs = $SleepMs
            MinSpeedupMultiplier = $MinSpeedupMultiplier
            NoBuild = $true
            Strict = $true
        }

        if (-not [string]::IsNullOrWhiteSpace($resolvedBaseline.BaselinePath)) {
            $benchmarkArgs["BaselineJsonPath"] = [string]$resolvedBaseline.BaselinePath
        }

        if (-not [string]::IsNullOrWhiteSpace($MaxP95Ms)) {
            $benchmarkArgs["MaxP95Ms"] = $MaxP95Ms
        }

        try {
            & "$PSScriptRoot/run-benchmark.ps1" @benchmarkArgs
            Assert-LastExitCode "scripts/run-benchmark.ps1 strict gate"
        }
        finally {
            if (-not [string]::IsNullOrWhiteSpace($resolvedBaseline.TempPath)) {
                Remove-Item -LiteralPath ([string]$resolvedBaseline.TempPath) -ErrorAction SilentlyContinue
            }
        }
    }

    Invoke-LaunchPolicyGateDiagnostics -ProjectPath $projectPath -RuntimePlatform $resolvedRunPlatform

    if ($isScriptHarness -and -not $includeHarnessRuntimeDiagnostics) {
        Write-Host "Skipping runtime health diagnostics (script harness mode)."
    }
    else {
        Invoke-RuntimeHealthDiagnostics -ProjectPath $projectPath -RuntimePlatform $resolvedRunPlatform
    }

    if ($isScriptHarness) {
        Write-Host "Skipping launch smoke verification (script harness mode)."
    }
    elseif (-not $SkipLaunchSmoke) {
        Invoke-WinUiLaunchSmoke -ProjectPath $projectPath -RuntimePlatform $resolvedRunPlatform -TimeoutSeconds $LaunchTimeoutSeconds
    }
    else {
        Write-Host "Skipping launch smoke verification."
    }

    Write-Host "Validation complete."
}
finally {
    Pop-Location
}
