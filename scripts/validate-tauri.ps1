[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$SkipBundle,
    [switch]$BenchmarkGate,
    [ValidateSet("x86", "x64", "ARM64")]
    [string]$BenchmarkPlatform = "x64",
    [int]$BenchmarkTicks = 120,
    [int]$BenchmarkSleepMs = 1000,
    [string]$BenchmarkBaselineJsonPath = "",
    [string]$BenchmarkBaselineArtifactPath = "",
    [string]$BenchmarkMinSpeedupMultiplier = "0.90",
    [string]$BenchmarkMaxP95Ms = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$appRoot = Join-Path $repoRoot "src/BatCave.App"
$cargoManifest = Join-Path $appRoot "src-tauri/Cargo.toml"
$tauriBuildScript = "tauri:build:windows"

function Run-Step {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $true)]
        [scriptblock]$ScriptBlock
    )

    Write-Host "==> $Name"
    & $ScriptBlock
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

function Get-PeSubsystem {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $stream = [System.IO.File]::OpenRead($Path)
    $reader = New-Object System.IO.BinaryReader($stream)
    try {
        $stream.Position = 0x3c
        $peOffset = $reader.ReadInt32()
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "'$Path' is not a valid PE executable."
        }

        $optionalHeaderOffset = $peOffset + 24
        $stream.Position = $optionalHeaderOffset
        $magic = $reader.ReadUInt16()
        if ($magic -notin @(0x10b, 0x20b)) {
            throw "'$Path' has an unsupported PE optional header."
        }

        $stream.Position = $optionalHeaderOffset + 0x44
        return $reader.ReadUInt16()
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

Push-Location $appRoot
try {
    npm run verify
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    cargo fmt --manifest-path "$cargoManifest" --check
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    cargo check --manifest-path "$cargoManifest"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    cargo test --manifest-path "$cargoManifest"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    if ($BenchmarkGate.IsPresent) {
        Run-Step "Rust benchmark regression gate" {
            Push-Location $repoRoot
            try {
                $gateScript = Join-Path $repoRoot "scripts/run-benchmark-gate.ps1"
                $gateArgs = @{
                    BenchmarkHost = "core"
                    Platform = $BenchmarkPlatform
                    Ticks = $BenchmarkTicks
                    SleepMs = $BenchmarkSleepMs
                }

                if (-not [string]::IsNullOrWhiteSpace($BenchmarkBaselineJsonPath)) {
                    $gateArgs["BaselineJsonPath"] = $BenchmarkBaselineJsonPath
                }
                if (-not [string]::IsNullOrWhiteSpace($BenchmarkBaselineArtifactPath)) {
                    $gateArgs["BaselineArtifactPath"] = $BenchmarkBaselineArtifactPath
                }
                if (-not [string]::IsNullOrWhiteSpace($BenchmarkMinSpeedupMultiplier)) {
                    $gateArgs["MinSpeedupMultiplier"] = $BenchmarkMinSpeedupMultiplier
                }
                if (-not [string]::IsNullOrWhiteSpace($BenchmarkMaxP95Ms)) {
                    $gateArgs["MaxP95Ms"] = $BenchmarkMaxP95Ms
                }

                & $gateScript @gateArgs
            }
            finally {
                Pop-Location
            }
        }
    }
    else {
        Run-Step "Rust benchmark smoke" {
            Push-Location $repoRoot
            try {
                $benchmarkScript = Join-Path $repoRoot "scripts/run-benchmark.ps1"
                & $benchmarkScript -BenchmarkHost core -Platform $BenchmarkPlatform -WarmupTicks 0 -Ticks 2 -SleepMs 0 -Repeats 1 -Strict -MaxP95Ms 10000
            }
            finally {
                Pop-Location
            }
        }
    }

    if (-not $SkipBundle) {
        npm run $tauriBuildScript
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }

        $releaseExecutable = Join-Path $appRoot "src-tauri/target/release/batcave-monitor.exe"
        $subsystem = Get-PeSubsystem -Path $releaseExecutable
        if ($subsystem -ne 2) {
            throw "BatCave must be built as a Windows GUI executable (subsystem 2); found subsystem $subsystem."
        }
        Write-Host "Verified Windows GUI subsystem: $releaseExecutable"
    }
}
finally {
    Pop-Location
}

exit 0
