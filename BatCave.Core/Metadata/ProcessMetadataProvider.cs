using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using System.Diagnostics;
using System.Management;

namespace BatCave.Core.Metadata;

public sealed class ProcessMetadataProvider : IProcessMetadataProvider
{
    public async Task<ProcessMetadata?> GetAsync(uint pid, ulong startTimeMs, CancellationToken ct)
    {
        ct.ThrowIfCancellationRequested();

        if (!TryGetStartTime(pid, out ulong observedStartTimeMs))
        {
            return null;
        }

        if (observedStartTimeMs != startTimeMs)
        {
            return null;
        }

        return await Task.Run(() => QueryProcessMetadata(pid, startTimeMs, ct), ct).ConfigureAwait(false);
    }

    private static ProcessMetadata? QueryProcessMetadata(uint pid, ulong startTimeMs, CancellationToken ct)
    {
        ct.ThrowIfCancellationRequested();

        string query = $"SELECT ParentProcessId, CommandLine, ExecutablePath, CreationDate FROM Win32_Process WHERE ProcessId = {pid}";

        try
        {
            using ManagementObjectSearcher searcher = new(query);
            using ManagementObjectCollection results = searcher.Get();

            ManagementBaseObject? row = results.Cast<ManagementBaseObject>().FirstOrDefault();
            if (row is null)
            {
                return null;
            }

            if (row["CreationDate"] is string rawCreationDate)
            {
                DateTime creationUtc = ManagementDateTimeConverter.ToDateTime(rawCreationDate).ToUniversalTime();
                ulong rowStartTimeMs = (ulong)new DateTimeOffset(creationUtc).ToUnixTimeMilliseconds();
                if (rowStartTimeMs != startTimeMs)
                {
                    return null;
                }
            }

            uint parentPid = 0;
            if (row["ParentProcessId"] is not null && uint.TryParse(row["ParentProcessId"].ToString(), out uint parsedParentPid))
            {
                parentPid = parsedParentPid;
            }

            return new ProcessMetadata
            {
                Pid = pid,
                ParentPid = parentPid,
                CommandLine = row["CommandLine"] as string,
                ExecutablePath = row["ExecutablePath"] as string,
            };
        }
        catch (ManagementException ex)
        {
            throw new InvalidOperationException($"metadata lookup failed for pid {pid}: {ex.Message}", ex);
        }
    }

    private static bool TryGetStartTime(uint pid, out ulong startTimeMs)
    {
        startTimeMs = 0;

        try
        {
            using Process process = Process.GetProcessById((int)pid);
            DateTime utcStart = process.StartTime.ToUniversalTime();
            startTimeMs = (ulong)new DateTimeOffset(utcStart).ToUnixTimeMilliseconds();
            return true;
        }
        catch (Exception ex) when (ex is ArgumentException or InvalidOperationException or System.ComponentModel.Win32Exception)
        {
            return false;
        }
    }
}
