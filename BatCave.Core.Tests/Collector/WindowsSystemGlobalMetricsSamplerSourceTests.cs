namespace BatCave.Core.Tests.Collector;

public sealed class WindowsSystemGlobalMetricsSamplerSourceTests
{
    [Fact]
    public void SamplerExtensionsSource_DisposesHotWmiRowsInsteadOfLeavingThemToGc()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave.Core", "Collector", "WindowsSystemGlobalMetricsSampler.Extensions.cs"));

        Assert.Contains("private static ManagementBaseObject? TakeFirstManagementRow(", source, StringComparison.Ordinal);
        Assert.Contains("private static ManagementBaseObject? FindManagementRow(", source, StringComparison.Ordinal);
        Assert.Contains("using ManagementBaseObject rowScope = row;", source, StringComparison.Ordinal);
        Assert.DoesNotContain(".Cast<ManagementBaseObject>().FirstOrDefault()", source, StringComparison.Ordinal);
    }

    [Fact]
    public void ProcessMetadataProviderSource_DisposesFirstWmiRowAfterLookup()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave.Core", "Metadata", "ProcessMetadataProvider.cs"));

        Assert.Contains("using ManagementBaseObject? row = TakeFirstManagementRow(results);", source, StringComparison.Ordinal);
        Assert.Contains("private static ManagementBaseObject? TakeFirstManagementRow(ManagementObjectCollection rows)", source, StringComparison.Ordinal);
        Assert.DoesNotContain(".Cast<ManagementBaseObject>().FirstOrDefault()", source, StringComparison.Ordinal);
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
