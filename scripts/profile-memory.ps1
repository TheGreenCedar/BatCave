<#
.SYNOPSIS
    Profiles BatCave memory growth while the unpackaged WinUI app is running.

.DESCRIPTION
    Launches BatCave through the existing WinUI run helper, samples the
    process working set and private bytes at a fixed interval, optionally
    captures a heap dump, and returns a non-zero exit code when the
    post-warmup slope exceeds the configured thresholds.

.EXAMPLE
    .\scripts\profile-memory.ps1

.EXAMPLE
    .\scripts\profile-memory.ps1 -CollectHeapDump -HeapDumpPath "$env:TEMP\batcave.dmp"
#>
[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet("x86", "x64", "ARM64")]
    [string]$Platform = "x64",
    [int]$WarmupSeconds = 20,
    [int]$DurationSeconds = 80,
    [int]$SampleIntervalSeconds = 5,
    [double]$MaxPrivateBytesGrowthMB = 25,
    [double]$MaxStepGrowthMB = 5,
    [switch]$CollectRuntimeCounters,
    [switch]$CollectGCDumpSnapshots,
    [string]$ArtifactDirectory = "",
    [switch]$CollectHeapDump,
    [string]$HeapDumpPath = "",
    [switch]$NoBuild,
    [switch]$Help
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$projectPath = Join-Path $repoRoot "BatCave/BatCave.csproj"
. "$PSScriptRoot/winui-run-helpers.ps1"

function Show-Help {
    Get-Help -Name $PSCommandPath -Full
}

function Convert-ToMegabytes {
    param([long]$Bytes)
    return [Math]::Round($Bytes / 1MB, 1)
}

function Resolve-ArtifactDirectory {
    param([string]$RequestedPath)

    if (-not [string]::IsNullOrWhiteSpace($RequestedPath)) {
        return $RequestedPath
    }

    return Join-Path $repoRoot ("artifacts/memory/" + [DateTime]::UtcNow.ToString("yyyyMMdd-HHmmss"))
}

function Collect-GCDumpSnapshot {
    param(
        [int]$ProcessId,
        [string]$OutputPath,
        [string]$Label
    )

    dotnet-gcdump collect -p $ProcessId -o $OutputPath
    if ($LASTEXITCODE -ne 0) {
        throw "dotnet-gcdump collect failed for $Label with exit code $LASTEXITCODE."
    }

    Write-Host "GCDump ($Label) written to $OutputPath"
}

function Get-ProcessSnapshot {
    param([System.Diagnostics.Process]$Process, [int]$SampleIndex, [double]$ElapsedSeconds)

    return [pscustomobject]@{
        SampleIndex = $SampleIndex
        ElapsedSeconds = [Math]::Round($ElapsedSeconds, 1)
        WorkingSetMB = Convert-ToMegabytes $Process.WorkingSet64
        PrivateMB = Convert-ToMegabytes $Process.PrivateMemorySize64
        Handles = $Process.Handles
        Threads = $Process.Threads.Count
    }
}

function Write-Summary {
    param(
        [System.Collections.Generic.List[object]]$Snapshots,
        [int]$WarmupSecondsValue,
        [double]$MaxPrivateBytesGrowthMBValue,
        [double]$MaxStepGrowthMBValue,
        [bool]$DiagnosticsActive
    )

    if ($Snapshots.Count -eq 0) {
        Write-Host "No samples captured."
        return $false
    }

    $postWarmup = @($Snapshots | Where-Object { $_.ElapsedSeconds -ge $WarmupSecondsValue })
    if ($postWarmup.Count -eq 0) {
        $postWarmup = @($Snapshots)
    }

    $warmupSnapshot = $Snapshots | Where-Object { $_.ElapsedSeconds -ge $WarmupSecondsValue } | Select-Object -First 1
    if ($null -eq $warmupSnapshot) {
        $warmupSnapshot = $Snapshots[0]
    }

    $finalSnapshot = $Snapshots[-1]
    $privateGrowthMB = [Math]::Round($finalSnapshot.PrivateMB - $warmupSnapshot.PrivateMB, 1)
    $workingSetGrowthMB = [Math]::Round($finalSnapshot.WorkingSetMB - $warmupSnapshot.WorkingSetMB, 1)
    $durationMinutes = [Math]::Max(0.1, ($finalSnapshot.ElapsedSeconds - $warmupSnapshot.ElapsedSeconds) / 60.0)
    $privateSlopeMBPerMin = [Math]::Round($privateGrowthMB / $durationMinutes, 2)
    $workingSetSlopeMBPerMin = [Math]::Round($workingSetGrowthMB / $durationMinutes, 2)

    $isMonotonicGrowth = $false
    if ($postWarmup.Count -ge 3) {
        $lastThree = $postWarmup | Select-Object -Last 3
        $step1 = [Math]::Round($lastThree[1].PrivateMB - $lastThree[0].PrivateMB, 1)
        $step2 = [Math]::Round($lastThree[2].PrivateMB - $lastThree[1].PrivateMB, 1)
        $isMonotonicGrowth = $step1 -ge $MaxStepGrowthMBValue -and $step2 -ge $MaxStepGrowthMBValue
    }

    Write-Host ""
    Write-Host "Memory profile summary"
    Write-Host ("  Warmup snapshot:   {0}s  Private={1} MB  WorkingSet={2} MB" -f $warmupSnapshot.ElapsedSeconds, $warmupSnapshot.PrivateMB, $warmupSnapshot.WorkingSetMB)
    Write-Host ("  Final snapshot:    {0}s  Private={1} MB  WorkingSet={2} MB" -f $finalSnapshot.ElapsedSeconds, $finalSnapshot.PrivateMB, $finalSnapshot.WorkingSetMB)
    Write-Host ("  Post-warmup delta: Private={0} MB  WorkingSet={1} MB" -f $privateGrowthMB, $workingSetGrowthMB)
    Write-Host ("  Slope:             Private={0} MB/min  WorkingSet={1} MB/min" -f $privateSlopeMBPerMin, $workingSetSlopeMBPerMin)
    Write-Host ("  Thresholds:        Private <= {0} MB  Step <= {1} MB" -f $MaxPrivateBytesGrowthMBValue, $MaxStepGrowthMBValue)
    Write-Host ("  Monotonic last 3:   {0}" -f ($(if ($isMonotonicGrowth) { "FAILED" } else { "OK" })))

    $fail = $privateGrowthMB -gt $MaxPrivateBytesGrowthMBValue -or $isMonotonicGrowth
    if ($fail) {
        if ($DiagnosticsActive) {
            Write-Warning "Memory growth exceeded the configured thresholds while runtime diagnostics were active. Treat this run as evidence-gathering, not as a regression gate."
            return $false
        }

        Write-Warning "Memory growth exceeded the configured thresholds."
    }

    return $fail
}

if ($Help.IsPresent) {
    Show-Help
    exit 0
}

if ($WarmupSeconds -lt 0) {
    throw "-WarmupSeconds must be greater than or equal to 0."
}

if ($DurationSeconds -le 0) {
    throw "-DurationSeconds must be greater than 0."
}

if ($SampleIntervalSeconds -le 0) {
    throw "-SampleIntervalSeconds must be greater than 0."
}

if ($WarmupSeconds -ge $DurationSeconds) {
    throw "-WarmupSeconds must be less than -DurationSeconds."
}

if ([string]::IsNullOrWhiteSpace($HeapDumpPath)) {
    $HeapDumpPath = Join-Path ([System.IO.Path]::GetTempPath()) ("batcave-memory-" + [DateTime]::UtcNow.ToString("yyyyMMdd-HHmmss") + ".dmp")
}

if (-not $NoBuild) {
    dotnet build (Join-Path $repoRoot "BatCave.slnx")
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

$exitCode = 0
$existingIds = @(Get-Process -Name "BatCave" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
$runArgs = Get-WinUiRunArguments -ProjectPath $projectPath -RuntimePlatform $Platform
$runner = Start-Process -FilePath "dotnet" -ArgumentList $runArgs -PassThru
$appProcess = $null
$windowDetected = $false
$counterProcess = $null
$runtimeCounterPath = ""
$warmupGCDumpPath = ""
$finalGCDumpPath = ""
$artifactRoot = ""
$warmupDumpCaptured = $false
$diagnosticsActive = $CollectRuntimeCounters.IsPresent -or $CollectGCDumpSnapshots.IsPresent -or $CollectHeapDump.IsPresent
$deadline = (Get-Date).AddSeconds([Math]::Max($DurationSeconds, $WarmupSeconds + 30))

try {
    while ((Get-Date) -lt $deadline -and $null -eq $appProcess) {
        $appProcess = Get-Process -Name "BatCave" -ErrorAction SilentlyContinue | Where-Object {
            $existingIds -notcontains $_.Id
        } | Select-Object -First 1

        if ($null -ne $appProcess) {
            break
        }

        if ($runner.HasExited) {
            throw "dotnet run exited before BatCave opened a top-level window."
        }

        Start-Sleep -Milliseconds 500
    }

    if ($null -eq $appProcess) {
        throw "BatCave launch failed: no new BatCave process was detected within the timeout."
    }

    Start-Sleep -Seconds 2
    $appProcess = Get-Process -Id $appProcess.Id -ErrorAction Stop
    $windowDetected = $appProcess.MainWindowHandle -ne 0
    if (-not $windowDetected) {
        Write-Warning "BatCave started without a detectable top-level window handle; continuing by sampling the new process instance."
    }

    if ($CollectRuntimeCounters.IsPresent -or $CollectGCDumpSnapshots.IsPresent) {
        $artifactRoot = Resolve-ArtifactDirectory -RequestedPath $ArtifactDirectory
        New-Item -ItemType Directory -Force -Path $artifactRoot | Out-Null
    }

    if ($CollectRuntimeCounters.IsPresent) {
        if (-not (Get-Command dotnet-counters -ErrorAction SilentlyContinue)) {
            throw "dotnet-counters is not installed, but -CollectRuntimeCounters was requested."
        }

        $runtimeCounterPath = Join-Path $artifactRoot "system-runtime.csv"
        $counterDuration = [TimeSpan]::FromSeconds($DurationSeconds + $SampleIntervalSeconds)
        $counterProcess = Start-Process `
            -FilePath "dotnet-counters" `
            -ArgumentList @(
                "collect",
                "-p", "$($appProcess.Id)",
                "--counters", "System.Runtime",
                "--refresh-interval", "$SampleIntervalSeconds",
                "--duration", $counterDuration.ToString("dd\:hh\:mm\:ss"),
                "--format", "csv",
                "-o", $runtimeCounterPath
            ) `
            -PassThru
    }

    Write-Host ("Profiling BatCave PID={0} Title='{1}' WindowDetected={2}" -f $appProcess.Id, $appProcess.MainWindowTitle, $windowDetected)

    $snapshots = [System.Collections.Generic.List[object]]::new()
    $sampleCount = [Math]::Floor($DurationSeconds / $SampleIntervalSeconds) + 1
    if ($sampleCount -lt 1) {
        $sampleCount = 1
    }

    for ($sampleIndex = 1; $sampleIndex -le $sampleCount; $sampleIndex++) {
        $elapsedSeconds = ($sampleIndex - 1) * $SampleIntervalSeconds
        $current = Get-Process -Id $appProcess.Id -ErrorAction Stop
        $snapshot = Get-ProcessSnapshot -Process $current -SampleIndex $sampleIndex -ElapsedSeconds $elapsedSeconds
        $snapshots.Add($snapshot)
        Write-Host ("[{0,2}] t={1,5}s  WS={2,7} MB  Private={3,7} MB  Handles={4}  Threads={5}" -f $snapshot.SampleIndex, $snapshot.ElapsedSeconds, $snapshot.WorkingSetMB, $snapshot.PrivateMB, $snapshot.Handles, $snapshot.Threads)

        if ($CollectGCDumpSnapshots.IsPresent -and -not $warmupDumpCaptured -and $elapsedSeconds -ge $WarmupSeconds) {
            if (-not (Get-Command dotnet-gcdump -ErrorAction SilentlyContinue)) {
                throw "dotnet-gcdump is not installed, but -CollectGCDumpSnapshots was requested."
            }

            $warmupGCDumpPath = Join-Path $artifactRoot ("idle-t{0}.gcdump" -f $WarmupSeconds)
            Collect-GCDumpSnapshot -ProcessId $appProcess.Id -OutputPath $warmupGCDumpPath -Label ("t+{0}s" -f $WarmupSeconds)
            $warmupDumpCaptured = $true
        }

        if ($sampleIndex -lt $sampleCount) {
            Start-Sleep -Seconds $SampleIntervalSeconds
        }
    }

    if ($CollectGCDumpSnapshots.IsPresent) {
        if (-not (Get-Command dotnet-gcdump -ErrorAction SilentlyContinue)) {
            throw "dotnet-gcdump is not installed, but -CollectGCDumpSnapshots was requested."
        }

        $finalGCDumpPath = Join-Path $artifactRoot ("idle-t{0}.gcdump" -f $DurationSeconds)
        Collect-GCDumpSnapshot -ProcessId $appProcess.Id -OutputPath $finalGCDumpPath -Label ("t+{0}s" -f $DurationSeconds)
    }

    if ($CollectHeapDump.IsPresent) {
        if (-not (Get-Command dotnet-dump -ErrorAction SilentlyContinue)) {
            throw "dotnet-dump is not installed, but -CollectHeapDump was requested."
        }

        dotnet-dump collect -p $appProcess.Id --type Heap -o $HeapDumpPath
        if ($LASTEXITCODE -ne 0) {
            throw "dotnet-dump collect failed with exit code $LASTEXITCODE."
        }

        Write-Host "Heap dump written to $HeapDumpPath"
    }

    $failed = Write-Summary `
        -Snapshots $snapshots `
        -WarmupSecondsValue $WarmupSeconds `
        -MaxPrivateBytesGrowthMBValue $MaxPrivateBytesGrowthMB `
        -MaxStepGrowthMBValue $MaxStepGrowthMB `
        -DiagnosticsActive $diagnosticsActive
    if ($failed) {
        $exitCode = 1
    }

    if (-not [string]::IsNullOrWhiteSpace($runtimeCounterPath)) {
        if ($counterProcess) {
            try {
                Wait-Process -Id $counterProcess.Id -Timeout ([Math]::Max(15, $SampleIntervalSeconds * 2))
            }
            catch {
                Write-Warning "dotnet-counters did not exit before the timeout; stopping it after sampling completed."
                if (-not $counterProcess.HasExited) {
                    Stop-Process -Id $counterProcess.Id -Force -ErrorAction SilentlyContinue
                }
            }
        }

        Write-Host "Runtime counters written to $runtimeCounterPath"
    }

    if (-not [string]::IsNullOrWhiteSpace($warmupGCDumpPath)) {
        Write-Host "Warmup gcdump: $warmupGCDumpPath"
    }

    if (-not [string]::IsNullOrWhiteSpace($finalGCDumpPath)) {
        Write-Host "Final gcdump:  $finalGCDumpPath"
    }
}
finally {
    if ($counterProcess -and -not $counterProcess.HasExited) {
        Stop-Process -Id $counterProcess.Id -Force -ErrorAction SilentlyContinue
    }

    if ($appProcess -and -not $appProcess.HasExited) {
        Stop-Process -Id $appProcess.Id -Force -ErrorAction SilentlyContinue
    }

    if ($runner -and -not $runner.HasExited) {
        Stop-Process -Id $runner.Id -Force -ErrorAction SilentlyContinue
    }
}

exit $exitCode
