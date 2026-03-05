namespace BatCave.Layouts;

public enum ShellAdaptiveMode
{
    Phone,
    Medium,
    Wide,
}

public static class ShellAdaptiveLayout
{
    public const double MediumBreakpoint = 760;
    public const double WideBreakpoint = 1200;

    public static ShellAdaptiveMode Resolve(double windowWidth)
    {
        if (windowWidth >= WideBreakpoint)
        {
            return ShellAdaptiveMode.Wide;
        }

        return windowWidth >= MediumBreakpoint
            ? ShellAdaptiveMode.Medium
            : ShellAdaptiveMode.Phone;
    }
}
