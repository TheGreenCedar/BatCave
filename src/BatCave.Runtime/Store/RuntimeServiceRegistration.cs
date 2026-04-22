using BatCave.Runtime.Collectors;
using BatCave.Runtime.Benchmarking;
using BatCave.Runtime.Contracts;
using BatCave.Runtime.Operations;
using BatCave.Runtime.Persistence;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;

namespace BatCave.Runtime.Store;

public static class RuntimeServiceRegistration
{
    public static IServiceCollection AddBatCaveRuntime(
        this IServiceCollection services,
        RuntimeStoreOptions? options = null,
        bool registerHostedService = true)
    {
        RuntimeStoreOptions effectiveOptions = options ?? new RuntimeStoreOptions();
        if (!registerHostedService)
        {
            effectiveOptions = effectiveOptions with { RuntimeLoopEnabled = false };
        }

        services.AddSingleton(effectiveOptions);
        services.AddSingleton<IProcessCollector, WindowsProcessCollector>();
        services.AddSingleton<IProcessCollectorFactory, DefaultProcessCollectorFactory>();
        services.AddSingleton<ISystemMetricsCollector, WindowsSystemMetricsCollector>();
        services.AddSingleton<IRuntimePersistenceStore, LocalJsonRuntimePersistenceStore>();
        services.AddSingleton<ILaunchPolicyGate, WindowsLaunchPolicyGate>();
        services.AddSingleton<IWinUiBenchmarkRunner, RuntimeWinUiBenchmarkRunner>();
        services.AddSingleton(provider => new RuntimeStore(
            provider.GetRequiredService<IProcessCollector>(),
            provider.GetRequiredService<ISystemMetricsCollector>(),
            provider.GetRequiredService<IRuntimePersistenceStore>(),
            provider.GetRequiredService<RuntimeStoreOptions>(),
            provider.GetRequiredService<ILogger<RuntimeStore>>(),
            provider.GetRequiredService<IProcessCollectorFactory>()));
        services.AddSingleton<IRuntimeStore>(provider => provider.GetRequiredService<RuntimeStore>());
        services.AddSingleton<CliOperationsHost>();
        if (registerHostedService)
        {
            services.AddHostedService(provider => provider.GetRequiredService<RuntimeStore>());
        }

        return services;
    }
}
