namespace BatCave.Tests.Lifecycle;

public sealed class AppStartupSourceTests
{
    [Fact]
    public void AppSource_CatchesHostStartFailures_AndRoutesThemIntoShellStartupErrorPresentation()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "App.xaml.cs"));

        Assert.Contains("catch (Exception ex) when (!cliMode)", source, StringComparison.Ordinal);
        Assert.Contains("RouteStartupFailureToShell(ex);", source, StringComparison.Ordinal);
        Assert.Contains("viewModel.PresentStartupFailure(ex, RetryHostStartupAsync);", source, StringComparison.Ordinal);
    }

    [Fact]
    public void AppSource_DoesNotRethrowWhenHostConstructionFailsBeforeShellStartupErrorUi()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "App.xaml.cs"));

        Assert.DoesNotContain("ExceptionDispatchInfo.Capture(ex).Throw();", source, StringComparison.Ordinal);
        Assert.Contains("ShowHostConstructionFailureWindow(ex);", source, StringComparison.Ordinal);
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
