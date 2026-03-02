using BatCave.Core.Domain;

namespace BatCave.Tests.TestSupport;

internal static class TestProcessSamples
{
    public static ProcessSample Create(
        uint pid = 1,
        ulong seq = 1,
        ulong? tsMs = null,
        uint parentPid = 1,
        ulong startTimeMs = 1,
        string? name = null,
        double cpuPct = 1,
        ulong rssBytes = 1024,
        ulong privateBytes = 512,
        ulong ioReadBps = 10,
        ulong ioWriteBps = 10,
        ulong otherIoBps = 10,
        uint threads = 2,
        uint handles = 3,
        AccessState accessState = AccessState.Full)
    {
        return new ProcessSample
        {
            Seq = seq,
            TsMs = tsMs ?? seq,
            Pid = pid,
            ParentPid = parentPid,
            StartTimeMs = startTimeMs,
            Name = name ?? $"proc-{pid}",
            CpuPct = cpuPct,
            RssBytes = rssBytes,
            PrivateBytes = privateBytes,
            IoReadBps = ioReadBps,
            IoWriteBps = ioWriteBps,
            OtherIoBps = otherIoBps,
            Threads = threads,
            Handles = handles,
            AccessState = accessState,
        };
    }
}
