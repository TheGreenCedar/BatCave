using BatCave.Core.Domain;
using Microsoft.Extensions.Options;

namespace BatCave.Core.Runtime;

public sealed class RuntimeHostOptions
{
    public bool EnableRuntimeLoop { get; set; } = true;

    public SortColumn DefaultSortColumn { get; set; } = SortColumn.CpuPct;

    public SortDirection DefaultSortDirection { get; set; } = SortDirection.Desc;

    public string DefaultFilterText { get; set; } = string.Empty;

    public bool DefaultAdminMode { get; set; } = true;

    public int DefaultMetricTrendWindowSeconds { get; set; } = 60;
}

public sealed class RuntimeHostOptionsValidator : IValidateOptions<RuntimeHostOptions>
{
    private const int MaxDefaultFilterTextLength = 256;

    public ValidateOptionsResult Validate(string? name, RuntimeHostOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);

        List<string> failures = [];
        if (!Enum.IsDefined(options.DefaultSortColumn))
        {
            failures.Add($"Invalid RuntimeHostOptions.{nameof(RuntimeHostOptions.DefaultSortColumn)} value: {options.DefaultSortColumn}.");
        }

        if (!Enum.IsDefined(options.DefaultSortDirection))
        {
            failures.Add($"Invalid RuntimeHostOptions.{nameof(RuntimeHostOptions.DefaultSortDirection)} value: {options.DefaultSortDirection}.");
        }

        if ((options.DefaultFilterText ?? string.Empty).Length > MaxDefaultFilterTextLength)
        {
            failures.Add(
                $"Invalid RuntimeHostOptions.{nameof(RuntimeHostOptions.DefaultFilterText)} length {(options.DefaultFilterText ?? string.Empty).Length}. Maximum allowed length is {MaxDefaultFilterTextLength}.");
        }

        if (!IsSupportedMetricTrendWindowSeconds(options.DefaultMetricTrendWindowSeconds))
        {
            failures.Add(
                $"Invalid RuntimeHostOptions.{nameof(RuntimeHostOptions.DefaultMetricTrendWindowSeconds)} value: {options.DefaultMetricTrendWindowSeconds}. Supported values are 60 or 120.");
        }

        return failures.Count == 0
            ? ValidateOptionsResult.Success
            : ValidateOptionsResult.Fail(failures);
    }

    public static RuntimeHostOptions Validate(RuntimeHostOptions options)
    {
        RuntimeHostOptions normalized = Normalize(options);
        ValidateOptionsResult result = new RuntimeHostOptionsValidator().Validate(Options.DefaultName, normalized);
        if (result.Failed)
        {
            throw new InvalidOperationException(result.FailureMessage);
        }

        return normalized;
    }

    public static RuntimeHostOptions Normalize(RuntimeHostOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);

        return new RuntimeHostOptions
        {
            EnableRuntimeLoop = options.EnableRuntimeLoop,
            DefaultSortColumn = options.DefaultSortColumn,
            DefaultSortDirection = options.DefaultSortDirection,
            DefaultFilterText = (options.DefaultFilterText ?? string.Empty).Trim(),
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
