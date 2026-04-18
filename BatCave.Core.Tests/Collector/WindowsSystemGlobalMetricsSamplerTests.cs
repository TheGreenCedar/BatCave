using BatCave.Core.Collector;
using BatCave.Core.Domain;
using System.Diagnostics;

namespace BatCave.Core.Tests.Collector;

public class WindowsSystemGlobalMetricsSamplerTests
{
    [Fact]
    public void Sample_WhenProbeTimesOut_KeepsLastSuccessfulSectionUntilNextCycle()
    {
        int memoryProbeCalls = 0;
        using WindowsSystemGlobalMetricsSampler sampler = new(
            cpuSnapshotFactory: (_, _) => new SystemGlobalCpuSnapshot { ProcessorName = "cpu" },
            memorySnapshotFactory: _ =>
            {
                int call = Interlocked.Increment(ref memoryProbeCalls);
                if (call == 2)
                {
                    Thread.Sleep(180);
                }

                return new SystemGlobalMemorySnapshot
                {
                    TotalBytes = (ulong)call,
                };
            },
            diskSnapshotFactory: () => [],
            networkSnapshotFactory: () => [],
            metadataRefreshAction: static () => { },
            extendedProbeSoftTimeout: TimeSpan.FromMilliseconds(40),
            initializePdhCounters: false);

        Stopwatch stopwatch = Stopwatch.StartNew();
        _ = sampler.Sample();
        stopwatch.Stop();
        Assert.True(stopwatch.ElapsedMilliseconds < 100, $"sample call took {stopwatch.ElapsedMilliseconds}ms");

        Thread.Sleep(120);
        SystemGlobalMetricsSample afterFirstCycle = sampler.Sample();
        Assert.Equal(1UL, afterFirstCycle.MemorySnapshot?.TotalBytes);

        _ = sampler.Sample();
        Thread.Sleep(80);
        SystemGlobalMetricsSample duringTimedOutCycle = sampler.Sample();

        Assert.Equal(1UL, duringTimedOutCycle.MemorySnapshot?.TotalBytes);
        Assert.True(duringTimedOutCycle.ExtendedProbeCycleCompleted);
    }

    [Fact]
    public void Sample_AfterTimedOutProbe_RetriesOnLaterCycleAndUpdatesSection()
    {
        int memoryProbeCalls = 0;
        using WindowsSystemGlobalMetricsSampler sampler = new(
            cpuSnapshotFactory: (_, _) => new SystemGlobalCpuSnapshot { ProcessorName = "cpu" },
            memorySnapshotFactory: _ =>
            {
                int call = Interlocked.Increment(ref memoryProbeCalls);
                if (call == 2)
                {
                    Thread.Sleep(180);
                }

                ulong value = call switch
                {
                    1 => 1UL,
                    2 => 2UL,
                    _ => 3UL,
                };

                return new SystemGlobalMemorySnapshot
                {
                    TotalBytes = value,
                };
            },
            diskSnapshotFactory: () => [],
            networkSnapshotFactory: () => [],
            metadataRefreshAction: static () => { },
            extendedProbeSoftTimeout: TimeSpan.FromMilliseconds(40),
            initializePdhCounters: false);

        _ = sampler.Sample();
        Thread.Sleep(120);
        _ = sampler.Sample();

        _ = sampler.Sample();
        Thread.Sleep(260);
        _ = sampler.Sample();
        Thread.Sleep(120);

        SystemGlobalMetricsSample afterRetry = sampler.Sample();
        Assert.Equal(3UL, afterRetry.MemorySnapshot?.TotalBytes);
        Assert.True(memoryProbeCalls >= 3);
    }

    [Fact]
    public void Sample_WhenProbeCompletesAfterTimeout_KeepsNewestCycleResult()
    {
        int cpuProbeCalls = 0;
        using WindowsSystemGlobalMetricsSampler sampler = new(
            cpuSnapshotFactory: (_, _) =>
            {
                int call = Interlocked.Increment(ref cpuProbeCalls);
                if (call == 1)
                {
                    Thread.Sleep(180);
                }

                return new SystemGlobalCpuSnapshot
                {
                    ProcessorName = $"cpu-{call}",
                };
            },
            memorySnapshotFactory: _ => new SystemGlobalMemorySnapshot(),
            diskSnapshotFactory: () => [],
            networkSnapshotFactory: () => [],
            metadataRefreshAction: static () => { },
            extendedProbeSoftTimeout: TimeSpan.FromMilliseconds(40),
            initializePdhCounters: false);

        _ = sampler.Sample();
        Thread.Sleep(80);

        SystemGlobalMetricsSample whileTimedOut = sampler.Sample();
        Assert.Null(whileTimedOut.CpuSnapshot);

        Thread.Sleep(180);
        SystemGlobalMetricsSample afterLateCompletion = sampler.Sample();

        Assert.Equal("cpu-2", afterLateCompletion.CpuSnapshot?.ProcessorName);
        Assert.True(cpuProbeCalls >= 2);
    }

    [Fact]
    public void Sample_WhenProbeNeverCompletes_FutureCyclesStillRun()
    {
        TaskCompletionSource<bool> neverComplete = new(TaskCreationOptions.RunContinuationsAsynchronously);
        int memoryProbeCalls = 0;
        using WindowsSystemGlobalMetricsSampler sampler = new(
            cpuSnapshotFactory: (_, _) => new SystemGlobalCpuSnapshot { ProcessorName = "cpu" },
            memorySnapshotFactory: _ =>
            {
                int call = Interlocked.Increment(ref memoryProbeCalls);
                if (call == 2)
                {
                    neverComplete.Task.GetAwaiter().GetResult();
                }

                return new SystemGlobalMemorySnapshot
                {
                    TotalBytes = (ulong)call,
                };
            },
            diskSnapshotFactory: () => [],
            networkSnapshotFactory: () => [],
            metadataRefreshAction: static () => { },
            extendedProbeSoftTimeout: TimeSpan.FromMilliseconds(40),
            initializePdhCounters: false);

        _ = sampler.Sample();
        Thread.Sleep(120);
        Assert.Equal(1UL, sampler.Sample().MemorySnapshot?.TotalBytes);

        _ = sampler.Sample();
        Thread.Sleep(80);
        Assert.Equal(1UL, sampler.Sample().MemorySnapshot?.TotalBytes);

        _ = sampler.Sample();
        Thread.Sleep(120);

        SystemGlobalMetricsSample afterRetry = sampler.Sample();
        Assert.Equal(3UL, afterRetry.MemorySnapshot?.TotalBytes);
        Assert.True(memoryProbeCalls >= 3);
    }

    [Fact]
    public void MergeCpuSnapshotKernelPct_WhenSnapshotMissing_ReturnsNull()
    {
        SystemGlobalCpuSnapshot? merged = WindowsSystemGlobalMetricsSampler.MergeCpuSnapshotKernelPct(snapshot: null, kernelPct: 21.5d);
        Assert.Null(merged);
    }

    [Fact]
    public void MergeCpuSnapshotKernelPct_WhenKernelUnchanged_ReusesSnapshot()
    {
        SystemGlobalCpuSnapshot snapshot = new()
        {
            ProcessorName = "cpu",
            KernelPct = 14.25d,
        };

        SystemGlobalCpuSnapshot? merged = WindowsSystemGlobalMetricsSampler.MergeCpuSnapshotKernelPct(snapshot, 14.25d);
        Assert.Same(snapshot, merged);
    }

    [Fact]
    public void MergeCpuSnapshotKernelPct_WhenKernelUpdated_PreservesMetadataAndUpdatesKernel()
    {
        double[] logicalValues = [12d, 34d];
        SystemGlobalCpuSnapshot snapshot = new()
        {
            ProcessorName = "cpu",
            KernelPct = 6.5d,
            SpeedMHz = 4200d,
            LogicalProcessorUtilizationPct = logicalValues,
        };

        SystemGlobalCpuSnapshot? merged = WindowsSystemGlobalMetricsSampler.MergeCpuSnapshotKernelPct(snapshot, 33.75d);

        Assert.NotNull(merged);
        Assert.Equal("cpu", merged.ProcessorName);
        Assert.Equal(4200d, merged.SpeedMHz);
        Assert.Equal(33.75d, merged.KernelPct);
        Assert.Same(logicalValues, merged.LogicalProcessorUtilizationPct);
    }

    [Fact]
    public void ResolveDiskActiveTimePct_WhenBothCountersMissing_ReturnsNull()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveDiskActiveTimePct(diskTimePct: null, idleTimePct: null);
        Assert.Null(resolved);
    }

    [Fact]
    public void ResolveDiskActiveTimePct_WhenDiskTimePresent_UsesDiskTime()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveDiskActiveTimePct(diskTimePct: 37, idleTimePct: null);
        Assert.Equal(37d, resolved);
    }

    [Fact]
    public void ResolveDiskActiveTimePct_WhenDiskTimeZeroAndIdlePresent_UsesIdleFallback()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveDiskActiveTimePct(diskTimePct: 0, idleTimePct: 72);
        Assert.Equal(28d, resolved);
    }

    [Fact]
    public void ResolveDiskActiveTimePct_WhenBothPresent_UsesHigherActiveValue()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveDiskActiveTimePct(diskTimePct: 5, idleTimePct: 70);
        Assert.Equal(30d, resolved);
    }

    [Fact]
    public void ResolveDiskActiveTimePct_WhenValuesOutOfRange_ClampsToPercentBounds()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveDiskActiveTimePct(diskTimePct: 140, idleTimePct: 180);
        Assert.Equal(100d, resolved);
    }

    [Fact]
    public void ResolvePreferredDiskActiveTimePct_WhenBothPresent_UsesHigherValue()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolvePreferredDiskActiveTimePct(primaryPct: 12, fallbackPct: 67);
        Assert.Equal(67d, resolved);
    }

    [Fact]
    public void ResolvePreferredDiskActiveTimePct_WhenPrimaryMissing_UsesFallback()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolvePreferredDiskActiveTimePct(primaryPct: null, fallbackPct: 31);
        Assert.Equal(31d, resolved);
    }

    [Fact]
    public void ResolvePreferredDiskActiveTimePct_WhenBothInvalid_ReturnsNull()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolvePreferredDiskActiveTimePct(primaryPct: double.NaN, fallbackPct: double.PositiveInfinity);
        Assert.Null(resolved);
    }

    [Fact]
    public void ResolveCpuSpeedMHz_WhenActualFrequencyValid_PrefersActualFrequency()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveCpuSpeedMHz(
            actualFrequencyMHz: 4200,
            processorFrequencyMHz: 3900,
            staticCurrentClockSpeedMHz: 3600);
        Assert.Equal(4200d, resolved);
    }

    [Fact]
    public void ResolveCpuSpeedMHz_WhenActualFrequencyInvalid_FallsBackToProcessorFrequency()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveCpuSpeedMHz(
            actualFrequencyMHz: 0,
            processorFrequencyMHz: 3900,
            staticCurrentClockSpeedMHz: 3600);
        Assert.Equal(3900d, resolved);
    }

    [Fact]
    public void ResolveCpuSpeedMHz_WhenDynamicValuesInvalid_FallsBackToStaticClockSpeed()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveCpuSpeedMHz(
            actualFrequencyMHz: double.NaN,
            processorFrequencyMHz: -1,
            staticCurrentClockSpeedMHz: 3600);
        Assert.Equal(3600d, resolved);
    }

    [Fact]
    public void ResolveCpuSpeedMHz_WhenAllCandidatesInvalid_ReturnsNull()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveCpuSpeedMHz(
            actualFrequencyMHz: double.PositiveInfinity,
            processorFrequencyMHz: 99,
            staticCurrentClockSpeedMHz: 25000);
        Assert.Null(resolved);
    }

    [Theory]
    [InlineData(double.NegativeInfinity)]
    [InlineData(double.PositiveInfinity)]
    [InlineData(double.NaN)]
    [InlineData(0d)]
    [InlineData(-5d)]
    [InlineData(99d)]
    [InlineData(20001d)]
    public void NormalizeCpuSpeedMHz_WhenValueIsNotReasonable_ReturnsNull(double value)
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.NormalizeCpuSpeedMHz(value);
        Assert.Null(resolved);
    }

    [Theory]
    [InlineData(100d)]
    [InlineData(3600d)]
    [InlineData(20000d)]
    public void NormalizeCpuSpeedMHz_WhenValueIsReasonable_ReturnsValue(double value)
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.NormalizeCpuSpeedMHz(value);
        Assert.Equal(value, resolved);
    }

    [Fact]
    public void ResolveHardwareReservedBytes_WhenInstalledOrVisibleMissing_ReturnsNull()
    {
        ulong? missingInstalled = WindowsSystemGlobalMetricsSampler.ResolveHardwareReservedBytes(installedBytes: null, visibleBytes: 8_000_000_000UL);
        ulong? missingVisible = WindowsSystemGlobalMetricsSampler.ResolveHardwareReservedBytes(installedBytes: 8_000_000_000UL, visibleBytes: null);

        Assert.Null(missingInstalled);
        Assert.Null(missingVisible);
    }

    [Fact]
    public void ResolveHardwareReservedBytes_WhenInstalledGreaterThanVisible_ReturnsDifference()
    {
        ulong? reserved = WindowsSystemGlobalMetricsSampler.ResolveHardwareReservedBytes(
            installedBytes: 17_179_869_184UL,
            visibleBytes: 17_179_344_896UL);

        Assert.Equal(524_288UL, reserved);
    }

    [Theory]
    [InlineData(8_000_000_000UL, 8_000_000_000UL)]
    [InlineData(7_999_000_000UL, 8_000_000_000UL)]
    public void ResolveHardwareReservedBytes_WhenDifferenceNotPositive_ReturnsZero(ulong installedBytes, ulong visibleBytes)
    {
        ulong? reserved = WindowsSystemGlobalMetricsSampler.ResolveHardwareReservedBytes(installedBytes, visibleBytes);
        Assert.Equal(0UL, reserved);
    }

    [Theory]
    [InlineData(3u, 1)]
    [InlineData(4u, 2)]
    [InlineData(5u, 3)]
    public void ResolveCacheTierFromWmiLevel_WhenMappedLevel_ReturnsCacheTier(uint wmiLevel, byte expectedTier)
    {
        byte? resolved = WindowsSystemGlobalMetricsSampler.ResolveCacheTierFromWmiLevel(wmiLevel);
        Assert.Equal(expectedTier, resolved);
    }

    [Theory]
    [InlineData(null)]
    [InlineData(0u)]
    [InlineData(1u)]
    [InlineData(2u)]
    [InlineData(6u)]
    public void ResolveCacheTierFromWmiLevel_WhenUnmappedLevel_ReturnsNull(uint? wmiLevel)
    {
        byte? resolved = WindowsSystemGlobalMetricsSampler.ResolveCacheTierFromWmiLevel(wmiLevel);
        Assert.Null(resolved);
    }

    [Fact]
    public void ResolveAvgResponseMsFromRawCounters_WhenValidDelta_ComputesMilliseconds()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveAvgResponseMsFromRawCounters(
            previousCounterValue: 1000,
            currentCounterValue: 1600,
            previousCounterBase: 10,
            currentCounterBase: 20,
            frequencyPerfTime: 100);

        Assert.Equal(600d, resolved);
    }

    [Fact]
    public void ResolveAvgResponseMsFromRawCounters_WhenFirstSample_ReturnsNull()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveAvgResponseMsFromRawCounters(
            previousCounterValue: null,
            currentCounterValue: 1600,
            previousCounterBase: null,
            currentCounterBase: 20,
            frequencyPerfTime: 100);

        Assert.Null(resolved);
    }

    [Theory]
    [InlineData(1000ul, 900ul, 10ul, 20ul, 100ul)]
    [InlineData(1000ul, 1600ul, 10ul, 10ul, 100ul)]
    [InlineData(1000ul, 1600ul, 10ul, 20ul, 0ul)]
    public void ResolveAvgResponseMsFromRawCounters_WhenInvalidDelta_ReturnsNull(
        ulong previousCounterValue,
        ulong currentCounterValue,
        ulong previousCounterBase,
        ulong currentCounterBase,
        ulong frequencyPerfTime)
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveAvgResponseMsFromRawCounters(
            previousCounterValue,
            currentCounterValue,
            previousCounterBase,
            currentCounterBase,
            frequencyPerfTime);

        Assert.Null(resolved);
    }
}
