using BatCave.Layouts;

namespace BatCave.Tests.Layouts;

public class ShellAdaptiveLayoutTests
{
    [Theory]
    [InlineData(300, ShellAdaptiveMode.Phone)]
    [InlineData(759.99, ShellAdaptiveMode.Phone)]
    [InlineData(760, ShellAdaptiveMode.Medium)]
    [InlineData(1100, ShellAdaptiveMode.Medium)]
    [InlineData(1200, ShellAdaptiveMode.Wide)]
    [InlineData(1600, ShellAdaptiveMode.Wide)]
    public void Resolve_ReturnsExpectedModeForWidth(double width, ShellAdaptiveMode expected)
    {
        ShellAdaptiveMode actual = ShellAdaptiveLayout.Resolve(width);

        Assert.Equal(expected, actual);
    }
}
