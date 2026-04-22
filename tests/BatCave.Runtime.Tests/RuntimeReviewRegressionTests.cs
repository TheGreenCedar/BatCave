using BatCave.Runtime.Benchmarking;
using BatCave.Runtime.Collectors;
using BatCave.Runtime.Contracts;
using BatCave.Runtime.Operations;
using BatCave.Runtime.Persistence;
using BatCave.Runtime.Serialization;
using BatCave.Runtime.Store;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text.Json;

namespace BatCave.Runtime.Tests;

public sealed class RuntimeReviewRegressionTests
{
    [Fact]
    public async Task CliOperationsHost_ElevatedHelperRequiresPathTokenArguments()
    {
        CliOperationsHost host = new(new PassingLaunchPolicyGate(), new NullRuntimeStore());

        int exitCode = await host.ExecuteAsync(["--elevated-helper"], CancellationToken.None);

        Assert.Equal(2, exitCode);
    }

    [Fact]
    public async Task CliOperationsHost_RejectsUnknownAndMissingValueArguments()
    {
        CliOperationsHost host = new(new PassingLaunchPolicyGate(), new NullRuntimeStore());

        int unknownExitCode = await host.ExecuteAsync(["--benchmark", "--unknown-option"], CancellationToken.None);
        int missingValueExitCode = await host.ExecuteAsync(
            ["--elevated-helper", "--data-file", "--stop-file", "stop", "--token", "token"],
            CancellationToken.None);

        Assert.Equal(2, unknownExitCode);
        Assert.Equal(2, missingValueExitCode);
    }

    [Fact]
    public async Task CliOperationsHost_ElevatedHelperRunsWhenArgumentsArePresent()
    {
        string directory = Path.Combine(Path.GetTempPath(), "BatCaveRuntimeTests", Guid.NewGuid().ToString("N"));
        string dataFile = Path.Combine(directory, "snapshot.json");
        string stopFile = Path.Combine(directory, "stop.signal");
        const string token = "cli-helper-token";
        CliOperationsHost host = new(new PassingLaunchPolicyGate(), new NullRuntimeStore());

        try
        {
            Directory.CreateDirectory(directory);
            using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
            Task<int> helper = Task.Run(() => host.ExecuteAsync(
                ["--elevated-helper", "--data-file", dataFile, "--stop-file", stopFile, "--token", token],
                timeout.Token));

            while (!File.Exists(dataFile))
            {
                await Task.Delay(10, timeout.Token);
            }

            File.WriteAllText(stopFile, "stop");
            Assert.Equal(0, await helper.WaitAsync(timeout.Token));

            using JsonDocument document = JsonDocument.Parse(File.ReadAllText(dataFile));
            Assert.Equal(token, document.RootElement.GetProperty("token").GetString());
            Assert.True(document.RootElement.GetProperty("seq").GetUInt64() >= 1);
        }
        finally
        {
            if (Directory.Exists(directory))
            {
                Directory.Delete(directory, recursive: true);
            }
        }
    }

    [Fact]
    public async Task ElevatedBridgeHelper_WritesSnakeCaseTokenedSnapshot()
    {
        string directory = Path.Combine(Path.GetTempPath(), "BatCaveRuntimeTests", Guid.NewGuid().ToString("N"));
        string dataFile = Path.Combine(directory, "snapshot.json");
        string stopFile = Path.Combine(directory, "stop.signal");
        const string token = "review-token";

        try
        {
            Directory.CreateDirectory(directory);
            using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
            Task<int> helper = Task.Run(() => ElevatedBridgeHelper.RunElevatedHelper(
                dataFile,
                stopFile,
                token,
                new FixedProcessCollector(Sample(42, "HelperRow", 1)),
                TimeSpan.FromMilliseconds(25),
                timeout.Token));

            while (!File.Exists(dataFile))
            {
                await Task.Delay(10, timeout.Token);
            }

            File.WriteAllText(stopFile, "stop");
            Assert.Equal(0, await helper.WaitAsync(timeout.Token));

            using JsonDocument document = JsonDocument.Parse(File.ReadAllText(dataFile));
            JsonElement root = document.RootElement;
            Assert.Equal(token, root.GetProperty("token").GetString());
            Assert.True(root.GetProperty("seq").GetUInt64() >= 1);
            JsonElement row = root.GetProperty("rows")[0];
            Assert.Equal(42u, row.GetProperty("pid").GetUInt32());
            Assert.Equal(420UL, row.GetProperty("disk_bps").GetUInt64());
            Assert.True(row.TryGetProperty("start_time_ms", out _));
            Assert.False(row.TryGetProperty("StartTimeMs", out _));
        }
        finally
        {
            if (Directory.Exists(directory))
            {
                Directory.Delete(directory, recursive: true);
            }
        }
    }

    [Fact]
    public void ElevatedBridgeClient_PollsSnakeCaseHelperSnapshot()
    {
        string directory = Path.Combine(Path.GetTempPath(), "BatCaveRuntimeTests", Guid.NewGuid().ToString("N"));
        string dataFile = Path.Combine(directory, "snapshot.json");
        string stopFile = Path.Combine(directory, "stop.signal");
        const string token = "client-token";

        try
        {
            Directory.CreateDirectory(directory);
            string json = JsonSerializer.Serialize(new
            {
                token,
                seq = 7UL,
                rows = new[] { Sample(77, "ElevatedRow", 3) },
            }, JsonDefaults.SnakeCase);
            File.WriteAllText(dataFile, json);
            using ElevatedBridgeClient client = ElevatedBridgeClient.CreateForTest(
                dataFile,
                stopFile,
                token,
                launchedMs: 0,
                nowMs: () => 1);

            BridgePollResult result = client.PollRows();

            Assert.Equal(BridgePollState.Rows, result.State);
            ProcessSample row = Assert.Single(result.Rows);
            Assert.Equal(77u, row.Pid);
            Assert.Equal("ElevatedRow", row.Name);
        }
        finally
        {
            if (Directory.Exists(directory))
            {
                Directory.Delete(directory, recursive: true);
            }
        }
    }

    [Fact]
    public void WindowsProcessCollector_ReportsIoTransferRatesFromCounterDeltas()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return;
        }

        ulong nowMs = 10_000;
        ProcessIoCounters totals = new(ReadTransferCount: 100, WriteTransferCount: 50, OtherTransferCount: 10);
        WindowsProcessCollector collector = new(
            () => nowMs,
            process => process.Id == Environment.ProcessId ? totals : null);

        _ = collector.Collect(1);
        nowMs += 1_000;
        totals = new ProcessIoCounters(ReadTransferCount: 600, WriteTransferCount: 250, OtherTransferCount: 110);

        ProcessSample row = Assert.Single(collector.Collect(2), row => row.Pid == Environment.ProcessId);

        Assert.Equal(700UL, row.DiskBps);
        Assert.Equal(100UL, row.OtherIoBps);
    }

    [Fact]
    public void WindowsProcessCollector_MarksSamplesPartialWhenIoProbeFails()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return;
        }

        WindowsProcessCollector collector = new(
            () => 10_000,
            process => process.Id == Environment.ProcessId ? (ProcessIoCounters?)null : null);

        ProcessSample row = Assert.Single(collector.Collect(1), row => row.Pid == Environment.ProcessId);

        Assert.Equal(AccessState.Partial, row.AccessState);
    }

    [Fact]
    public void WindowsProcessCollector_UsesUnixEpochMillisecondsForStartTime()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return;
        }

        WindowsProcessCollector collector = new();
        ProcessSample row = Assert.Single(collector.Collect(1), row => row.Pid == Environment.ProcessId);
        ulong nowMs = (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();

        Assert.InRange(row.StartTimeMs, 1_500_000_000_000UL, nowMs);
    }

    [Fact]
    public void WindowsProcessCollector_PopulatesParentPidFromProcessSnapshot()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return;
        }

        WindowsProcessCollector collector = new();
        ProcessSample row = Assert.Single(collector.Collect(1), row => row.Pid == Environment.ProcessId);

        Assert.NotEqual(0u, row.ParentPid);
    }

    [Fact]
    public void JsonDefaults_SerializesPublicEnumsAsSnakeCaseStrings()
    {
        string json = JsonSerializer.Serialize(new RuntimeSnapshot
        {
            Settings = new RuntimeSettings
            {
                Query = new RuntimeQuery
                {
                    SortColumn = SortColumn.MemoryBytes,
                    SortDirection = SortDirection.Asc,
                },
            },
            Rows = [new ProcessSample { Pid = 42, Name = "EnumSample", AccessState = AccessState.Partial }],
        }, JsonDefaults.SnakeCase);

        Assert.Contains("\"access_state\": \"partial\"", json);
        Assert.Contains("\"sort_column\": \"memory_bytes\"", json);
        Assert.Contains("\"sort_direction\": \"asc\"", json);
        Assert.DoesNotContain("\"access_state\": 1", json);
    }

    [Fact]
    public void WindowsSystemMetricsCollector_UsesRateMetricSamplerForIoTelemetry()
    {
        using WindowsSystemMetricsCollector collector = new(() => (11UL, 22UL, 33UL));

        SystemMetricsSnapshot snapshot = collector.Sample();

        Assert.Equal(11UL, snapshot.DiskReadBps);
        Assert.Equal(22UL, snapshot.DiskWriteBps);
        Assert.Equal(33UL, snapshot.OtherIoBps);
        Assert.True(snapshot.IsReady);
    }

    [Fact]
    public void WindowsSystemMetricsCollector_SubtractsIdleFromKernelCpuPct()
    {
        (double? cpu, double? kernel) = WindowsSystemMetricsCollector.CalculateCpuPercentages(
            previousIdle: 100,
            previousKernel: 1_100,
            previousUser: 100,
            currentIdle: 200,
            currentKernel: 1_200,
            currentUser: 100);

        Assert.Equal(0d, cpu);
        Assert.Equal(0d, kernel);
    }

    [Fact]
    public async Task RuntimeStore_DoesNotUpsertRowsForOnlySeqAndTimestampChanges()
    {
        await using RuntimeStore store = CreateStore(new FixedProcessCollector(Sample(10, "Stable", 1)));
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
        await store.StartAsync(timeout.Token);
        await store.ExecuteAsync(new RefreshNowCommand(), timeout.Token);

        await using IAsyncEnumerator<RuntimeDelta> subscription = store.SubscribeAsync(timeout.Token).GetAsyncEnumerator(timeout.Token);
        RuntimeDelta second = await ReadNextAfterAsync(
            subscription,
            () => store.ExecuteAsync(new RefreshNowCommand(), timeout.Token),
            timeout.Token);

        Assert.Empty(second.Upserts);
        Assert.Single(second.Snapshot.Rows);
        Assert.Equal(second.Seq, second.Snapshot.Rows[0].Seq);
    }

    [Fact]
    public async Task RuntimeStore_CommandSnapshotsUseIncreasingSequences()
    {
        await using RuntimeStore store = CreateStore(new FixedProcessCollector(Sample(10, "Stable", 1)));
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
        await store.StartAsync(timeout.Token);
        await store.ExecuteAsync(new RefreshNowCommand(), timeout.Token);

        await using IAsyncEnumerator<RuntimeDelta> subscription = store.SubscribeAsync(timeout.Token).GetAsyncEnumerator(timeout.Token);
        RuntimeDelta queryDelta = await ReadNextAfterAsync(
            subscription,
            () => store.ExecuteAsync(new SetProcessQueryCommand(new RuntimeQuery { FilterText = "stable" }), timeout.Token),
            timeout.Token);
        RuntimeDelta pauseDelta = await ReadNextAfterAsync(
            subscription,
            () => store.ExecuteAsync(new PauseRuntimeCommand(), timeout.Token),
            timeout.Token);

        Assert.True(pauseDelta.Seq > queryDelta.Seq);
        Assert.Equal(pauseDelta.Seq, pauseDelta.Snapshot.Seq);
    }

    [Fact]
    public async Task RuntimeStore_PausedLoopStillProcessesCommands()
    {
        await using RuntimeStore store = CreatePausedStore(new FixedProcessCollector(Sample(10, "Stable", 1)));
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
        await store.StartAsync(timeout.Token);

        await using IAsyncEnumerator<RuntimeDelta> subscription = store.SubscribeAsync(timeout.Token).GetAsyncEnumerator(timeout.Token);
        RuntimeDelta resumeDelta = await ReadNextAfterAsync(
            subscription,
            () => store.ExecuteAsync(new ResumeRuntimeCommand(), timeout.Token),
            timeout.Token);

        Assert.False(resumeDelta.Snapshot.Settings.Paused);
        Assert.Equal(resumeDelta.Seq, resumeDelta.Snapshot.Seq);
    }

    [Fact]
    public async Task RuntimeStore_AdminModeRequestStaysRequestedWhenActivationFails()
    {
        await using RuntimeStore store = CreateStore(
            new FixedProcessCollector(Sample(10, "Stable", 1)),
            new TestProcessCollectorFactory(new CollectorActivationResult(
                new FixedProcessCollector(Sample(10, "Stable", 1)),
                EffectiveAdminMode: false,
                Warning: "admin_mode_start_failed test failure")));
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
        await store.StartAsync(timeout.Token);

        await store.ExecuteAsync(new SetAdminModeCommand(true), timeout.Token);

        RuntimeSnapshot snapshot = store.GetSnapshot();
        Assert.True(snapshot.Settings.AdminModeRequested);
        Assert.False(snapshot.Settings.AdminModeEnabled);
        Assert.Contains(snapshot.Warnings, warning => warning.Category == "admin_mode");
        Assert.Contains("admin_mode_start_failed", snapshot.Health.StatusSummary, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task RuntimeStore_AdminModeRequestUsesActivatedElevatedCollector()
    {
        FixedProcessCollector elevatedCollector = new(Sample(42, "Elevated", 7));
        TestProcessCollectorFactory factory = new(new CollectorActivationResult(
            elevatedCollector,
            EffectiveAdminMode: true,
            Warning: null));
        await using RuntimeStore store = CreateStore(new FixedProcessCollector(Sample(10, "Local", 1)), factory);
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
        await store.StartAsync(timeout.Token);

        await store.ExecuteAsync(new SetAdminModeCommand(true), timeout.Token);
        await store.ExecuteAsync(new RefreshNowCommand(), timeout.Token);

        RuntimeSnapshot snapshot = store.GetSnapshot();
        Assert.True(factory.WasCalled);
        Assert.True(factory.RequestedAdminMode);
        Assert.True(snapshot.Settings.AdminModeRequested);
        Assert.True(snapshot.Settings.AdminModeEnabled);
        ProcessSample row = Assert.Single(snapshot.Rows);
        Assert.Equal("Elevated", row.Name);
    }

    [Fact]
    public async Task RuntimeStore_StartupPreservesRequestedAdminButClearsEffectiveElevation()
    {
        await using RuntimeStore store = CreateStore(
            new FixedProcessCollector(Sample(10, "Stable", 1)),
            options: new RuntimeStoreOptions
            {
                DefaultSettings = new RuntimeSettings
                {
                    AdminModeRequested = true,
                    AdminModeEnabled = true,
                },
            });

        RuntimeSnapshot snapshot = store.GetSnapshot();
        Assert.True(snapshot.Settings.AdminModeRequested);
        Assert.False(snapshot.Settings.AdminModeEnabled);
    }

    [Fact]
    public async Task RuntimeStore_DefaultsToRequestedAdminWithoutEffectiveElevation()
    {
        await using RuntimeStore store = CreateStore(new FixedProcessCollector(Sample(10, "Stable", 1)));

        RuntimeSnapshot snapshot = store.GetSnapshot();

        Assert.True(snapshot.Settings.AdminModeRequested);
        Assert.False(snapshot.Settings.AdminModeEnabled);
        Assert.Contains("standard access active", snapshot.Health.StatusSummary, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task RuntimeStore_DisablingAdminClearsRequestedAndEffectiveElevation()
    {
        TestProcessCollectorFactory factory = new(new CollectorActivationResult(
            new FixedProcessCollector(Sample(10, "Stable", 1)),
            EffectiveAdminMode: false,
            Warning: null));
        await using RuntimeStore store = CreateStore(
            new FixedProcessCollector(Sample(10, "Stable", 1)),
            factory,
            new RuntimeStoreOptions
            {
                DefaultSettings = new RuntimeSettings
                {
                    AdminModeRequested = true,
                    AdminModeEnabled = true,
                    MetricWindowSeconds = 60,
                },
            });
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
        await store.StartAsync(timeout.Token);

        await store.ExecuteAsync(new SetAdminModeCommand(false), timeout.Token);

        RuntimeSnapshot snapshot = store.GetSnapshot();
        Assert.False(snapshot.Settings.AdminModeRequested);
        Assert.False(snapshot.Settings.AdminModeEnabled);
        Assert.True(factory.WasCalled);
        Assert.False(factory.RequestedAdminMode);
    }

    [Fact]
    public void WindowsProcessCollector_RetainsRowsAndWarnsOnTransientEnumerationFailure()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return;
        }

        ulong nowMs = 10_000;
        bool fail = false;
        WindowsProcessCollector collector = new(
            () => nowMs,
            _ => null,
            () =>
            {
                if (fail)
                {
                    throw new InvalidOperationException("enumeration failed");
                }

                return [Process.GetCurrentProcess()];
            });

        IReadOnlyList<ProcessSample> first = collector.Collect(1);
        fail = true;
        nowMs += 1_000;
        IReadOnlyList<ProcessSample> second = collector.Collect(2);

        Assert.NotEmpty(first);
        ProcessSample retained = Assert.Single(second);
        Assert.Equal(first[0].Pid, retained.Pid);
        Assert.Equal(2UL, retained.Seq);
        Assert.Equal(nowMs, retained.TsMs);
        Assert.Contains("process_collect_failed", collector.TakeWarning());
    }

    [Fact]
    public void WindowsProcessCollector_MapsDeniedPartialAndFullAccessStates()
    {
        Assert.Equal(AccessState.Denied, WindowsProcessCollector.ResolveAccessState(false, false, false));
        Assert.Equal(AccessState.Partial, WindowsProcessCollector.ResolveAccessState(false, true, false));
        Assert.Equal(AccessState.Full, WindowsProcessCollector.ResolveAccessState(true, true, true));
    }

    [Fact]
    public void LocalJsonPersistence_MigratesLegacySettingsShape()
    {
        string directory = Path.Combine(Path.GetTempPath(), "BatCaveRuntimeTests", Guid.NewGuid().ToString("N"));
        try
        {
            Directory.CreateDirectory(directory);
            File.WriteAllText(Path.Combine(directory, "settings.json"), """
                {
                  "sort_col": "rss_bytes",
                  "sort_dir": "asc",
                  "filter_text": "code",
                  "admin_mode": true,
                  "admin_preference_initialized": true,
                  "metric_trend_window_seconds": 120
                }
                """);
            LocalJsonRuntimePersistenceStore persistence = new(directory);

            RuntimeSettings settings = Assert.IsType<RuntimeSettings>(persistence.LoadSettings());

            Assert.Equal(SortColumn.MemoryBytes, settings.Query.SortColumn);
            Assert.Equal(SortDirection.Asc, settings.Query.SortDirection);
            Assert.Equal("code", settings.Query.FilterText);
            Assert.True(settings.AdminModeRequested);
            Assert.False(settings.AdminModeEnabled);
            Assert.Equal(120, settings.MetricWindowSeconds);
        }
        finally
        {
            if (Directory.Exists(directory))
            {
                Directory.Delete(directory, recursive: true);
            }
        }
    }

    [Fact]
    public void LocalJsonPersistence_MigratesLegacyWarmCacheAndSkipsUnreadableRows()
    {
        string directory = Path.Combine(Path.GetTempPath(), "BatCaveRuntimeTests", Guid.NewGuid().ToString("N"));
        try
        {
            Directory.CreateDirectory(directory);
            File.WriteAllText(Path.Combine(directory, "warm-cache.json"), """
                {
                  "seq": 12,
                  "rows": [
                    {
                      "seq": 10,
                      "ts_ms": 20,
                      "pid": 42,
                      "start_time_ms": 1000,
                      "name": "Legacy",
                      "cpu_pct": 3.5,
                      "rss_bytes": 4096,
                      "private_bytes": 2048,
                      "io_read_bps": 10,
                      "io_write_bps": 15,
                      "io_other_bps": 7,
                      "threads": 2,
                      "handles": 4,
                      "access_state": "partial"
                    },
                    { "pid": 99, "rss_bytes": 1 }
                  ]
                }
                """);
            LocalJsonRuntimePersistenceStore persistence = new(directory);

            WarmCache cache = Assert.IsType<WarmCache>(persistence.LoadWarmCache());
            ProcessSample row = Assert.Single(cache.Rows);

            Assert.Equal(12UL, cache.Seq);
            Assert.Equal(42u, row.Pid);
            Assert.Equal("Legacy", row.Name);
            Assert.Equal(4096UL, row.MemoryBytes);
            Assert.Equal(2048UL, row.PrivateBytes);
            Assert.Equal(25UL, row.DiskBps);
            Assert.Equal(7UL, row.OtherIoBps);
            Assert.Equal(AccessState.Partial, row.AccessState);
            Assert.Contains("legacy_warm_cache_row_skipped", persistence.TakeWarning());
        }
        finally
        {
            if (Directory.Exists(directory))
            {
                Directory.Delete(directory, recursive: true);
            }
        }
    }

    [Fact]
    public async Task RuntimeStore_ImportsPersistenceLoadWarningsIntoSnapshot()
    {
        string directory = Path.Combine(Path.GetTempPath(), "BatCaveRuntimeTests", Guid.NewGuid().ToString("N"));
        try
        {
            Directory.CreateDirectory(directory);
            File.WriteAllText(Path.Combine(directory, "settings.json"), "{not-json");
            LocalJsonRuntimePersistenceStore persistence = new(directory);
            await using RuntimeStore store = new(
                new FixedProcessCollector(Sample(10, "Stable", 1)),
                new FixedSystemMetricsCollector(),
                persistence,
                new RuntimeStoreOptions
                {
                    RuntimeLoopEnabled = false,
                    TickInterval = TimeSpan.FromHours(1),
                });

            RuntimeSnapshot snapshot = store.GetSnapshot();

            Assert.Contains(snapshot.Warnings, warning =>
                warning.Category == "persistence"
                && warning.Message.Contains("persistence_load_json_failed", StringComparison.OrdinalIgnoreCase));
        }
        finally
        {
            if (Directory.Exists(directory))
            {
                Directory.Delete(directory, recursive: true);
            }
        }
    }

    [Fact]
    public async Task RuntimeStore_WarmCachePersistsFullRowsWhenSnapshotIsFiltered()
    {
        CapturingPersistenceStore persistence = new();
        await using RuntimeStore store = new(
            new FixedProcessCollector(Sample(10, "Alpha", 1), Sample(20, "Code", 2), Sample(30, "Beta", 3)),
            new FixedSystemMetricsCollector(),
            persistence,
            new RuntimeStoreOptions
            {
                TickInterval = TimeSpan.FromHours(1),
                SubscriberBufferCapacity = 4,
                WarmCacheWriteIntervalTicks = 1,
            });
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
        await store.StartAsync(timeout.Token);

        await store.ExecuteAsync(new RefreshNowCommand(), timeout.Token);
        await WaitForWarmCacheAsync(persistence, minSeq: 1, timeout.Token);
        await store.ExecuteAsync(new SetProcessQueryCommand(new RuntimeQuery { FilterText = "Code" }), timeout.Token);
        await store.ExecuteAsync(new RefreshNowCommand(), timeout.Token);

        WarmCache cache = await WaitForWarmCacheAsync(persistence, minSeq: store.GetSnapshot().Seq, timeout.Token);
        Assert.Single(store.GetSnapshot().Rows);
        Assert.Equal(3, cache.Rows.Count);
        Assert.Contains(cache.Rows, row => row.Name == "Alpha");
        Assert.Contains(cache.Rows, row => row.Name == "Code");
        Assert.Contains(cache.Rows, row => row.Name == "Beta");
    }

    [Fact]
    public void RuntimeServiceRegistration_CanOmitHostedRuntimeLoopForCliCommands()
    {
        ServiceCollection cliServices = new();
        cliServices.AddBatCaveRuntime(registerHostedService: false);

        Assert.DoesNotContain(cliServices, service => service.ServiceType == typeof(IHostedService));
        ServiceDescriptor cliOptions = Assert.Single(cliServices, service => service.ServiceType == typeof(RuntimeStoreOptions));
        Assert.False(Assert.IsType<RuntimeStoreOptions>(cliOptions.ImplementationInstance).RuntimeLoopEnabled);

        ServiceCollection interactiveServices = new();
        interactiveServices.AddBatCaveRuntime();

        Assert.Contains(interactiveServices, service => service.ServiceType == typeof(IHostedService));
        ServiceDescriptor interactiveOptions = Assert.Single(interactiveServices, service => service.ServiceType == typeof(RuntimeStoreOptions));
        Assert.True(Assert.IsType<RuntimeStoreOptions>(interactiveOptions.ImplementationInstance).RuntimeLoopEnabled);
    }

    [Fact]
    public async Task RuntimeStore_DisabledLoopReportsDisabledHealth()
    {
        await using RuntimeStore store = new(
            new FixedProcessCollector(Sample(10, "Stable", 1)),
            new FixedSystemMetricsCollector(),
            new MemoryPersistenceStore(),
            new RuntimeStoreOptions
            {
                RuntimeLoopEnabled = false,
                TickInterval = TimeSpan.FromHours(1),
            });

        RuntimeSnapshot snapshot = store.GetSnapshot();
        Assert.False(snapshot.Health.RuntimeLoopEnabled);
        Assert.False(snapshot.Health.RuntimeLoopRunning);
        Assert.Equal("Runtime loop disabled.", snapshot.Health.StatusSummary);
    }

    [Fact]
    public void BenchmarkRunner_WinUiFallbackExercisesRuntimeReducerWithoutClaimingDispatcher()
    {
        BenchmarkSummary summary = BenchmarkRunner.Run(
            ticks: 1,
            sleepMs: 0,
            CancellationToken.None,
            new BenchmarkGateOptions
            {
                Host = "winui",
                MeasurementOrigin = BenchmarkRunner.WinUiMeasurementOrigin,
                UsesAttachedDispatcher = false,
                RssBudgetBytes = 256UL * 1024UL * 1024UL,
            });

        Assert.Equal("winui", summary.Host);
        Assert.Equal(BenchmarkRunner.WinUiMeasurementOrigin, summary.MeasurementOrigin);
        Assert.False(summary.UsesAttachedDispatcher);
        Assert.NotNull(summary.InteractionProbeP95);
        Assert.Equal(1, summary.Ticks);
    }

    [Fact]
    public async Task CliOperationsHost_DelegatesWinUiBenchmarkToInjectedRunner()
    {
        CapturingWinUiBenchmarkRunner runner = new();
        CliOperationsHost host = new(new PassingLaunchPolicyGate(), new NullRuntimeStore(), [runner]);

        int exitCode = await host.ExecuteAsync(
            ["--benchmark", "--benchmark-host", "winui", "--ticks", "1", "--sleep-ms", "0"],
            CancellationToken.None);

        Assert.Equal(0, exitCode);
        Assert.True(runner.WasCalled);
        Assert.Equal(1, runner.Ticks);
        Assert.Equal(0, runner.SleepMs);
    }

    [Fact]
    public void BenchmarkRunner_PreservesBenchmarkInteractionProbeJsonShape()
    {
        string json = JsonSerializer.Serialize(new BenchmarkSummary
        {
            InteractionProbeP95 = new BenchmarkInteractionProbeP95
            {
                FilterP95Ms = 1,
                SortP95Ms = 2,
                SelectionP95Ms = 3,
                BatchP95Ms = 4,
                PlotP95Ms = 5,
            },
        }, JsonDefaults.SnakeCase);

        Assert.Contains("\"batch_p95_ms\": 4", json);
        Assert.DoesNotContain("ui_batch_p95_ms", json);
    }

    [Fact]
    public void BenchmarkRunner_BlocksStrictSpeedupWhenBaselineMetadataMismatches()
    {
        BenchmarkSummary summary = BenchmarkRunner.Run(
            ticks: 1,
            sleepMs: 0,
            CancellationToken.None,
            new BenchmarkGateOptions
            {
                Host = "core",
                MeasurementOrigin = BenchmarkRunner.CoreMeasurementOrigin,
                Baseline = new BenchmarkSummary
                {
                    Host = "winui",
                    MeasurementOrigin = BenchmarkRunner.WinUiMeasurementOrigin,
                    TickP95Ms = 100_000,
                    SortP95Ms = 100_000,
                },
                MinSpeedupMultiplier = 1.01,
                CpuBudgetPct = 1_000,
                RssBudgetBytes = ulong.MaxValue,
            });

        Assert.False(summary.BaselineMetadataMatched);
        Assert.False(summary.CoreSpeedupPassed);
        Assert.False(summary.StrictPassed);
    }

    [Fact]
    public void BenchmarkRunner_AcceptsLegacyCoreOriginBaselinesForStrictComparison()
    {
        BenchmarkSummary summary = BenchmarkRunner.Run(
            ticks: 1,
            sleepMs: 0,
            CancellationToken.None,
            new BenchmarkGateOptions
            {
                Host = "core",
                MeasurementOrigin = BenchmarkRunner.CoreMeasurementOrigin,
                Baseline = new BenchmarkSummary
                {
                    Host = "core",
                    MeasurementOrigin = "headless_runtime",
                    TickP95Ms = 100_000,
                    SortP95Ms = 100_000,
                },
                MinSpeedupMultiplier = 1.01,
                CpuBudgetPct = 1_000,
                RssBudgetBytes = ulong.MaxValue,
            });

        Assert.True(summary.BaselineMetadataMatched);
        Assert.NotNull(summary.BaselineComparison);
    }

    [Fact]
    public void BenchmarkRunner_RequiresComparableWinUiInteractionProbeWhenRequested()
    {
        BenchmarkSummary summary = BenchmarkRunner.Run(
            ticks: 1,
            sleepMs: 0,
            CancellationToken.None,
            new BenchmarkGateOptions
            {
                Host = "winui",
                MeasurementOrigin = BenchmarkRunner.WinUiMeasurementOrigin,
                UsesAttachedDispatcher = true,
                Baseline = new BenchmarkSummary
                {
                    Host = "winui",
                    MeasurementOrigin = BenchmarkRunner.WinUiMeasurementOrigin,
                    TickP95Ms = 100_000,
                    SortP95Ms = 100_000,
                },
                MinSpeedupMultiplier = 1.01,
                RequireInteractionProbeSpeedup = true,
                CpuBudgetPct = 1_000,
                RssBudgetBytes = ulong.MaxValue,
            });

        Assert.False(summary.InteractionSpeedupPassed);
        Assert.False(summary.StrictPassed);
        Assert.Null(summary.InteractionBaselineComparison);
    }

    private static ProcessSample Sample(uint pid, string name, double cpu)
    {
        return new ProcessSample
        {
            Seq = 1,
            TsMs = 1,
            Pid = pid,
            StartTimeMs = 1_700_000_000_000UL + pid,
            Name = name,
            CpuPct = cpu,
            MemoryBytes = pid * 1024UL,
            DiskBps = pid * 10UL,
            OtherIoBps = pid * 5UL,
            Threads = pid,
            Handles = pid * 2,
        };
    }

    private static RuntimeStore CreateStore(
        IProcessCollector processCollector,
        IProcessCollectorFactory? processCollectorFactory = null,
        RuntimeStoreOptions? options = null)
    {
        return new RuntimeStore(
            processCollector,
            new FixedSystemMetricsCollector(),
            new MemoryPersistenceStore(),
            options ?? new RuntimeStoreOptions
            {
                TickInterval = TimeSpan.FromHours(1),
                SubscriberBufferCapacity = 4,
                WarmCacheWriteIntervalTicks = 1000,
            },
            processCollectorFactory: processCollectorFactory);
    }

    private static RuntimeStore CreatePausedStore(IProcessCollector processCollector)
    {
        return new RuntimeStore(
            processCollector,
            new FixedSystemMetricsCollector(),
            new MemoryPersistenceStore(),
            new RuntimeStoreOptions
            {
                TickInterval = TimeSpan.FromHours(1),
                SubscriberBufferCapacity = 4,
                WarmCacheWriteIntervalTicks = 1000,
                DefaultSettings = new RuntimeSettings
                {
                    Paused = true,
                },
            });
    }

    private static async Task<RuntimeDelta> ReadNextAfterAsync(
        IAsyncEnumerator<RuntimeDelta> subscription,
        Func<Task> trigger,
        CancellationToken ct)
    {
        Task<bool> moveNext = Task.Run(async () => await subscription.MoveNextAsync().AsTask().ConfigureAwait(false), ct);
        await Task.Delay(25, ct);
        await trigger().WaitAsync(ct);
        Assert.True(await moveNext.WaitAsync(ct));
        return subscription.Current;
    }

    private static async Task<WarmCache> WaitForWarmCacheAsync(CapturingPersistenceStore persistence, ulong minSeq, CancellationToken ct)
    {
        while (persistence.SavedWarmCache is not WarmCache cache || cache.Seq < minSeq)
        {
            await Task.Delay(10, ct);
        }

        return persistence.SavedWarmCache!;
    }

    private sealed class FixedProcessCollector(params ProcessSample[] rows) : IProcessCollector
    {
        public IReadOnlyList<ProcessSample> Collect(ulong seq)
        {
            return rows.Select(row => row with { Seq = seq, TsMs = (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() }).ToArray();
        }
    }

    private sealed class FixedSystemMetricsCollector : ISystemMetricsCollector
    {
        public SystemMetricsSnapshot Sample()
        {
            return new SystemMetricsSnapshot
            {
                TsMs = (ulong)Math.Max(0L, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()),
                CpuPct = 17.5,
                MemoryUsedBytes = 2UL * 1024UL * 1024UL,
                MemoryTotalBytes = 8UL * 1024UL * 1024UL,
                IsReady = true,
            };
        }
    }

    private sealed class MemoryPersistenceStore : IRuntimePersistenceStore
    {
        public string BaseDirectory { get; } = Path.GetTempPath();

        public RuntimeSettings? LoadSettings() => null;

        public Task SaveSettingsAsync(RuntimeSettings settings, CancellationToken ct) => Task.CompletedTask;

        public WarmCache? LoadWarmCache() => null;

        public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct) => Task.CompletedTask;

        public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct) => Task.CompletedTask;
    }

    private sealed class CapturingPersistenceStore : IRuntimePersistenceStore
    {
        private WarmCache? _savedWarmCache;

        public string BaseDirectory { get; } = Path.GetTempPath();

        public WarmCache? SavedWarmCache => Volatile.Read(ref _savedWarmCache);

        public RuntimeSettings? LoadSettings() => null;

        public Task SaveSettingsAsync(RuntimeSettings settings, CancellationToken ct) => Task.CompletedTask;

        public WarmCache? LoadWarmCache() => null;

        public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
        {
            Volatile.Write(ref _savedWarmCache, cache);
            return Task.CompletedTask;
        }

        public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct) => Task.CompletedTask;
    }

    private sealed class CapturingWinUiBenchmarkRunner : IWinUiBenchmarkRunner
    {
        public bool WasCalled { get; private set; }

        public int Ticks { get; private set; }

        public int SleepMs { get; private set; }

        public ValueTask<BenchmarkSummary> RunAsync(int ticks, int sleepMs, BenchmarkGateOptions gates, CancellationToken ct)
        {
            WasCalled = true;
            Ticks = ticks;
            SleepMs = sleepMs;
            return ValueTask.FromResult(new BenchmarkSummary
            {
                Host = "winui",
                MeasurementOrigin = BenchmarkRunner.WinUiMeasurementOrigin,
                UsesAttachedDispatcher = true,
                Ticks = ticks,
                SleepMs = sleepMs,
                BudgetPassed = true,
                StrictPassed = true,
            });
        }
    }

    private sealed class TestProcessCollectorFactory(CollectorActivationResult result) : IProcessCollectorFactory
    {
        public bool WasCalled { get; private set; }

        public bool RequestedAdminMode { get; private set; }

        public ValueTask<CollectorActivationResult> CreateAsync(bool adminMode, CancellationToken ct)
        {
            WasCalled = true;
            RequestedAdminMode = adminMode;
            return ValueTask.FromResult(result);
        }
    }

    private sealed class PassingLaunchPolicyGate : ILaunchPolicyGate
    {
        public StartupGateStatus Enforce()
        {
            return StartupGateStatus.PassedContext(new LaunchContext
            {
                Os = "windows",
                WindowsBuild = 22_000,
            });
        }
    }

    private sealed class NullRuntimeStore : IRuntimeStore
    {
        public RuntimeSnapshot GetSnapshot() => new();

        public async IAsyncEnumerable<RuntimeDelta> SubscribeAsync([System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken ct)
        {
            await Task.CompletedTask;
            yield break;
        }

        public Task ExecuteAsync(RuntimeCommand command, CancellationToken ct) => Task.CompletedTask;
    }
}
