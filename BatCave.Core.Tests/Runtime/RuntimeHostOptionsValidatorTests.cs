using BatCave.Core.Domain;
using BatCave.Core.Runtime;

namespace BatCave.Core.Tests.Runtime;

public class RuntimeHostOptionsValidatorTests
{
    [Fact]
    public void Validate_WhenProcessTableAdvancedModeDefaultIsNotConfigured_DefaultsToFalse()
    {
        RuntimeHostOptions validated = RuntimeHostOptionsValidator.Validate(new RuntimeHostOptions());

        Assert.False(validated.DefaultProcessTableAdvancedMode);
    }

    [Fact]
    public void Validate_WhenProcessTableAdvancedModeDefaultIsConfigured_PropagatesToValidatedOptions()
    {
        RuntimeHostOptions options = new()
        {
            DefaultProcessTableAdvancedMode = true,
            DefaultFilterText = " svc ",
        };

        RuntimeHostOptions validated = RuntimeHostOptionsValidator.Validate(options);

        Assert.True(validated.DefaultProcessTableAdvancedMode);
        Assert.Equal("svc", validated.DefaultFilterText);
    }

    [Fact]
    public void Validate_WhenMetricTrendWindowDefaultIsUnsupported_Throws()
    {
        RuntimeHostOptions options = new()
        {
            DefaultMetricTrendWindowSeconds = 75,
        };

        InvalidOperationException exception = Assert.Throws<InvalidOperationException>(() => RuntimeHostOptionsValidator.Validate(options));
        Assert.Contains("60 or 120", exception.Message, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void ValidatePersistedSettings_WhenSortDirectionIsInvalid_Throws()
    {
        UserSettings settings = new()
        {
            SortDir = (SortDirection)22,
        };

        InvalidOperationException exception = Assert.Throws<InvalidOperationException>(() => RuntimeHostOptionsValidator.ValidatePersistedSettings(settings));
        Assert.Contains("sort direction", exception.Message, StringComparison.OrdinalIgnoreCase);
    }
}
