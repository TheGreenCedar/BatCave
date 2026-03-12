using BatCave.Controls;
using BatCave.ViewModels;
using Windows.UI;

namespace BatCave.Tests.ViewModels;

public sealed class GlobalResourceRowViewStateTests
{
    [Fact]
    public void Update_RaisesSubtitleAndValuePropertyChanges()
    {
        GlobalResourceRowViewState state = new(
            resourceId: "cpu",
            kind: GlobalResourceKind.Cpu,
            title: "CPU",
            subtitle: "10%",
            valueText: string.Empty,
            chartIdentityKey: "cpu",
            miniTrendValues: [10d],
            miniScaleMode: MetricTrendScaleMode.CpuPercent,
            miniStrokeColor: Color.FromArgb(0xFF, 0x00, 0x7A, 0xCC),
            miniFillColor: Color.FromArgb(0x33, 0x00, 0x7A, 0xCC),
            miniDomainMax: double.NaN);

        List<string> changed = [];
        state.PropertyChanged += (_, args) =>
        {
            if (!string.IsNullOrWhiteSpace(args.PropertyName))
            {
                changed.Add(args.PropertyName!);
            }
        };

        state.Update(
            subtitle: "22%",
            valueText: "42 C",
            chartIdentityKey: "cpu",
            miniTrendValues: [10d, 22d],
            miniScaleMode: MetricTrendScaleMode.CpuPercent,
            miniStrokeColor: Color.FromArgb(0xFF, 0x00, 0x7A, 0xCC),
            miniFillColor: Color.FromArgb(0x33, 0x00, 0x7A, 0xCC),
            miniDomainMax: double.NaN);

        Assert.Contains(nameof(GlobalResourceRowViewState.Subtitle), changed);
        Assert.Contains(nameof(GlobalResourceRowViewState.SubtitleVisibility), changed);
        Assert.Contains(nameof(GlobalResourceRowViewState.ValueText), changed);
        Assert.Contains(nameof(GlobalResourceRowViewState.ValueVisibility), changed);
        Assert.DoesNotContain("SetTextWithVisibility", changed);
    }
}
