using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

public sealed class RuntimeHostOptions
{
    public bool EnableRuntimeLoop { get; init; } = true;

    public SortColumn DefaultSortColumn { get; init; } = SortColumn.CpuPct;

    public SortDirection DefaultSortDirection { get; init; } = SortDirection.Desc;

    public string DefaultFilterText { get; init; } = string.Empty;

    public bool DefaultAdminMode { get; init; } = true;

    public int DefaultMetricTrendWindowSeconds { get; init; } = 60;
}

public static class RuntimeHostOptionsValidator
{
    private const int MaxDefaultFilterTextLength = 256;

    public static RuntimeHostOptions Validate(RuntimeHostOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);

        if (!Enum.IsDefined(options.DefaultSortColumn))
        {
            throw new InvalidOperationException($"Invalid RuntimeHostOptions.{nameof(RuntimeHostOptions.DefaultSortColumn)} value: {options.DefaultSortColumn}.");
        }

        if (!Enum.IsDefined(options.DefaultSortDirection))
        {
            throw new InvalidOperationException($"Invalid RuntimeHostOptions.{nameof(RuntimeHostOptions.DefaultSortDirection)} value: {options.DefaultSortDirection}.");
        }

        if (options.DefaultFilterText.Length > MaxDefaultFilterTextLength)
        {
            throw new InvalidOperationException(
                $"Invalid RuntimeHostOptions.{nameof(RuntimeHostOptions.DefaultFilterText)} length {options.DefaultFilterText.Length}. Maximum allowed length is {MaxDefaultFilterTextLength}.");
        }

        if (!IsSupportedMetricTrendWindowSeconds(options.DefaultMetricTrendWindowSeconds))
        {
            throw new InvalidOperationException(
                $"Invalid RuntimeHostOptions.{nameof(RuntimeHostOptions.DefaultMetricTrendWindowSeconds)} value: {options.DefaultMetricTrendWindowSeconds}. Supported values are 60 or 120.");
        }

        return new RuntimeHostOptions
        {
            EnableRuntimeLoop = options.EnableRuntimeLoop,
            DefaultSortColumn = options.DefaultSortColumn,
            DefaultSortDirection = options.DefaultSortDirection,
            DefaultFilterText = options.DefaultFilterText.Trim(),
            DefaultAdminMode = options.DefaultAdminMode,
            DefaultMetricTrendWindowSeconds = options.DefaultMetricTrendWindowSeconds,
        };
    }

    public static void ValidatePersistedSettings(UserSettings settings)
    {
        ArgumentNullException.ThrowIfNull(settings);

        if (!Enum.IsDefined(settings.SortCol))
        {
            throw new InvalidOperationException($"Persisted settings contain invalid sort column value: {settings.SortCol}.");
        }

        if (!Enum.IsDefined(settings.SortDir))
        {
            throw new InvalidOperationException($"Persisted settings contain invalid sort direction value: {settings.SortDir}.");
        }
    }

    public static bool IsSupportedMetricTrendWindowSeconds(int seconds)
    {
        return seconds is 60 or 120;
    }
}
