using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;

namespace BatCave.Hosting;

public static class BatCaveUiServiceRegistration
{
    public static IServiceCollection AddBatCaveUiServices(this IServiceCollection services)
    {
        services.AddSingleton<MonitoringShellViewModel>();
        services.AddSingleton<MainWindow>();
        return services;
    }
}
