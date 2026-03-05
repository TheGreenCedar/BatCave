using BatCave.Core.Abstractions;
using BatCave.Core.Collector;
using BatCave.Core.Metadata;
using BatCave.Core.Operations;
using BatCave.Core.Persistence;
using BatCave.Core.Pipeline;
using BatCave.Core.Policy;
using BatCave.Core.Runtime;
using BatCave.Core.Sort;
using BatCave.Core.State;
using BatCave.Services;
using Microsoft.Extensions.DependencyInjection;
using System;

namespace BatCave.Hosting;

public static class BatCaveServiceRegistration
{
    public static IServiceCollection AddBatCaveRuntimeServices(
        this IServiceCollection services,
        RuntimeHostOptions runtimeHostOptions)
    {
        RuntimeHostOptions validatedOptions = RuntimeHostOptionsValidator.Validate(runtimeHostOptions);

        services.AddSingleton(validatedOptions);
        services.AddSingleton<ICliOperationsHost, CliOperationsHost>();
        services.AddSingleton<ILaunchPolicyGate, WindowsLaunchPolicyGate>();
        services.AddSingleton<IProcessCollectorFactory, DefaultProcessCollectorFactory>();
        services.AddSingleton<ISystemGlobalMetricsSampler, WindowsSystemGlobalMetricsSampler>();
        services.AddSingleton<ITelemetryPipeline, DeltaTelemetryPipeline>();
        services.AddSingleton<IStateStore, InMemoryStateStore>();
        services.AddSingleton<ISortIndexEngine, PassThroughSortIndexEngine>();
        services.AddSingleton<IPersistenceStore, LocalJsonPersistenceStore>();
        services.AddSingleton<IProcessMetadataProvider, ProcessMetadataProvider>();

        services.AddSingleton<MonitoringRuntime>();
        services.AddSingleton(TimeProvider.System);
        services.AddSingleton<RuntimeLoopService>();
        services.AddSingleton<IRuntimeLoopController>(provider => provider.GetRequiredService<RuntimeLoopService>());

        services.AddSingleton<IRuntimeHealthService, RuntimeHealthService>();
        services.AddSingleton<IRuntimeEventGateway, RuntimeGateway>();
        services.AddHostedService<RuntimeLoopHostedService>();
        return services;
    }

}
