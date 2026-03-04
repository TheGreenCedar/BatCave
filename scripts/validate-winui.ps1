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
    [switch]$RunPerformanceGate
)

$ErrorActionPreference = "Stop"

function Assert-LastExitCode {
    param(
        [string]$CommandName
    )

    if ($LASTEXITCODE -ne 0) {
        throw "$CommandName failed with exit code $LASTEXITCODE."
    }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
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

    Write-Host "Validation complete."
}
finally {
    Pop-Location
}
