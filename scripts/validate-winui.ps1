param(
    [string]$Platform = "ARM64",
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

function Assert-LastExitCode {
    param(
        [string]$CommandName
    )

    if ($LASTEXITCODE -ne 0) {
        throw "$CommandName failed with exit code $LASTEXITCODE."
    }
}

function Invoke-RuntimeHealthDiagnostics {
    param(
        [string]$ProjectPath,
        [string]$RuntimePlatform
    )

    Write-Host "Verifying runtime health diagnostics surface..."
    $output = dotnet run --project $ProjectPath "-p:Platform=$RuntimePlatform" -- --print-runtime-health
    Assert-LastExitCode "dotnet run -- --print-runtime-health"

    $payload = $output | Out-String
    if ([string]::IsNullOrWhiteSpace($payload)) {
        throw "Runtime health diagnostics surface produced no output."
    }

    $json = $payload | ConvertFrom-Json
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
            -ArgumentList @("run", "--project", $ProjectPath, "-p:Platform=$RuntimePlatform") `
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
Push-Location $repoRoot
try {
    Write-Host "Validating WinUI compile path (Platform=$Platform)..."
    dotnet build BatCave/BatCave.csproj -p:Platform=$Platform
    Assert-LastExitCode "dotnet build BatCave/BatCave.csproj"

    Write-Host "Running solution tests..."
    dotnet test BatCave.slnx
    Assert-LastExitCode "dotnet test BatCave.slnx"

    if ($RunPerformanceGate) {
        if ([string]::IsNullOrWhiteSpace($BaselineJsonPath) -and [string]::IsNullOrWhiteSpace($BaselineArtifactPath)) {
            throw "RunPerformanceGate requires -BaselineJsonPath or -BaselineArtifactPath."
        }

        if (-not [string]::IsNullOrWhiteSpace($BaselineJsonPath) -and -not [string]::IsNullOrWhiteSpace($BaselineArtifactPath)) {
            throw "Specify either -BaselineJsonPath or -BaselineArtifactPath, not both."
        }

        Write-Host "Running strict performance gate benchmark..."
        $benchmarkArgs = @{
            BenchmarkHost = $BenchmarkHost
            Platform = $Platform
            Ticks = $Ticks
            SleepMs = $SleepMs
            MinSpeedupMultiplier = $MinSpeedupMultiplier
            NoBuild = $true
            Strict = $true
        }

        if (-not [string]::IsNullOrWhiteSpace($BaselineJsonPath)) {
            $benchmarkArgs["BaselineJsonPath"] = $BaselineJsonPath
        }

        if (-not [string]::IsNullOrWhiteSpace($BaselineArtifactPath)) {
            $benchmarkArgs["BaselineArtifactPath"] = $BaselineArtifactPath
        }

        if (-not [string]::IsNullOrWhiteSpace($MaxP95Ms)) {
            $benchmarkArgs["MaxP95Ms"] = $MaxP95Ms
        }

        & "$PSScriptRoot/run-benchmark.ps1" @benchmarkArgs
        Assert-LastExitCode "scripts/run-benchmark.ps1 strict gate"
    }

    if ($isScriptHarness) {
        Write-Host "Skipping runtime health diagnostics (script harness mode)."
    }
    else {
        Invoke-RuntimeHealthDiagnostics -ProjectPath $projectPath -RuntimePlatform $Platform
    }

    if ($isScriptHarness) {
        Write-Host "Skipping launch smoke verification (script harness mode)."
    }
    elseif (-not $SkipLaunchSmoke) {
        Invoke-WinUiLaunchSmoke -ProjectPath $projectPath -RuntimePlatform $Platform -TimeoutSeconds $LaunchTimeoutSeconds
    } else {
        Write-Host "Skipping launch smoke verification."
    }

    Write-Host "Validation complete."
}
finally {
    Pop-Location
}
