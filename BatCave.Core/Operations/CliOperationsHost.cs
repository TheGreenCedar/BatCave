namespace BatCave.Core.Operations;

public sealed class CliOperationsHost : ICliOperationsHost
{
    private static readonly HashSet<string> CliFlags = new(StringComparer.OrdinalIgnoreCase)
    {
        "--print-gate-status",
        "--benchmark",
        "--elevated-helper",
    };

    public bool IsCliMode(string[] args)
    {
        return args.Any(CliFlags.Contains);
    }

    public Task<int> ExecuteAsync(string[] args, CancellationToken ct)
    {
        Console.Error.WriteLine("CLI mode is recognized but not fully implemented yet.");
        return Task.FromResult(2);
    }
}
