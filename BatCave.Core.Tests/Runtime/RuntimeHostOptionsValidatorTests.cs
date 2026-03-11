using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using Microsoft.Extensions.Options;

namespace BatCave.Core.Tests.Runtime;

public class RuntimeHostOptionsValidatorTests
{
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
    public void Validate_WithOptionsPattern_WhenMetricTrendWindowDefaultIsUnsupported_Fails()
    {
        RuntimeHostOptions options = new()
        {
            DefaultMetricTrendWindowSeconds = 75,
        };

        ValidateOptionsResult result = new RuntimeHostOptionsValidator().Validate(Options.DefaultName, options);

        Assert.True(result.Failed);
        Assert.Contains("60 or 120", result.FailureMessage, StringComparison.OrdinalIgnoreCase);
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
