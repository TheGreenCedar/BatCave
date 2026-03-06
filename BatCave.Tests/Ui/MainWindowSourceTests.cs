namespace BatCave.Tests.Ui;

public class MainWindowSourceTests
{
    [Fact]
    public void MainWindowSource_AppliesPackagedWindowIconFromAssets()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("TryApplyWindowIcon();", source, StringComparison.Ordinal);
        Assert.Contains("Path.Combine(AppContext.BaseDirectory, \"Assets\", \"BatCaveLogo.ico\")", source, StringComparison.Ordinal);
        Assert.Contains("WindowNative.GetWindowHandle(this)", source, StringComparison.Ordinal);
        Assert.Contains("AppWindow.GetFromWindowId(windowId)", source, StringComparison.Ordinal);
        Assert.Contains("appWindow.SetIcon(iconPath);", source, StringComparison.Ordinal);
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
