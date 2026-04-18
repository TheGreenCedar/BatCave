namespace BatCave.Tests.Ui;

public sealed class AppXamlAccessibilityTests
{
    [Fact]
    public void AppXaml_DefinesHighContrastThemeDictionary_AndMotionResources()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "App.xaml"));

        Assert.Contains("<ResourceDictionary x:Key=\"HighContrast\">", xaml, StringComparison.Ordinal);
        Assert.Contains("<x:Double x:Key=\"BatCaveInteractivePointerOverOpacity\">", xaml, StringComparison.Ordinal);
        Assert.Contains("<x:Double x:Key=\"BatCaveInteractivePressedOpacity\">", xaml, StringComparison.Ordinal);
        Assert.Contains("<x:Boolean x:Key=\"BatCaveChartSmoothTransitionsEnabled\">", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void AppXaml_RestoresFocusVisualsAndSegmentedRadioStyles()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "App.xaml"));

        Assert.Contains("<Style x:Key=\"BatCaveSegmentRadioButtonStyle\" TargetType=\"RadioButton\">", xaml, StringComparison.Ordinal);
        Assert.Contains("<Style x:Key=\"BatCaveInspectorTabRadioButtonStyle\" TargetType=\"RadioButton\"", xaml, StringComparison.Ordinal);
        Assert.Contains("<VisualStateGroup x:Name=\"FocusStates\">", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"FocusBorder\"", xaml, StringComparison.Ordinal);
        Assert.Contains("UseSystemFocusVisuals\" Value=\"False\"", xaml, StringComparison.Ordinal);
    }

    private static string ResolveRepoPath(params string[] relativeSegments)
    {
        DirectoryInfo? current = new(AppContext.BaseDirectory);
        while (current is not null)
        {
            string candidate = Path.Combine(current.FullName, "BatCave.slnx");
            if (File.Exists(candidate))
            {
                string resolved = current.FullName;
                foreach (string segment in relativeSegments)
                {
                    resolved = Path.Combine(resolved, segment);
                }

                return resolved;
            }

            current = current.Parent;
        }

        throw new DirectoryNotFoundException("Could not locate repository root from test base directory.");
    }
}
