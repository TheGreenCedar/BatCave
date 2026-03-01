namespace BatCave.Core.Operations;

public interface ICliOperationsHost
{
    bool IsCliMode(string[] args);

    Task<int> ExecuteAsync(string[] args, CancellationToken ct);
}
