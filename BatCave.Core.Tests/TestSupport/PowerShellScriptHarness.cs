using System.Diagnostics;
using System.Globalization;

namespace BatCave.Core.Tests.TestSupport;

internal sealed class PowerShellScriptHarness : IDisposable
{
    private const string WrapperScript = """
param(
    [Parameter(Mandatory = $true)]
    [string]$TargetScript,
    [string]$ScriptArgLine = ""
)

function global:dotnet {
    param(
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$DotnetArgs
    )

    if ($null -ne $DotnetArgs -and $DotnetArgs.Count -gt 0) {
        $logPath = $env:FAKE_DOTNET_LOG
        if (-not [string]::IsNullOrWhiteSpace($logPath)) {
            Add-Content -Path $logPath -Value ($DotnetArgs -join ' ')
        }
    }

    $command = if ($null -eq $DotnetArgs -or $DotnetArgs.Count -eq 0) { "" } else { $DotnetArgs[0].ToLowerInvariant() }
    switch ($command) {
        'build' {
            if (-not [string]::IsNullOrWhiteSpace($env:FAKE_DOTNET_BUILD_EXIT_CODE)) {
                $global:LASTEXITCODE = [int]$env:FAKE_DOTNET_BUILD_EXIT_CODE
                return
            }

            $contains = $env:FAKE_DOTNET_BUILD_FAIL_CONTAINS
            if (-not [string]::IsNullOrWhiteSpace($contains)) {
                $joined = $DotnetArgs -join ' '
                if ($joined.IndexOf($contains, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
                    $global:LASTEXITCODE = [int]$env:FAKE_DOTNET_BUILD_FAIL_CODE
                    return
                }
            }

            $global:LASTEXITCODE = 0
            return
        }
        'run' {
            if (-not [string]::IsNullOrWhiteSpace($env:FAKE_DOTNET_RUN_EXIT_CODE)) {
                $global:LASTEXITCODE = [int]$env:FAKE_DOTNET_RUN_EXIT_CODE
                return
            }

            $global:LASTEXITCODE = 0
            return
        }
        'test' {
            if (-not [string]::IsNullOrWhiteSpace($env:FAKE_DOTNET_TEST_EXIT_CODE)) {
                $global:LASTEXITCODE = [int]$env:FAKE_DOTNET_TEST_EXIT_CODE
                return
            }

            $global:LASTEXITCODE = 0
            return
        }
        default {
            if (-not [string]::IsNullOrWhiteSpace($env:FAKE_DOTNET_DEFAULT_EXIT_CODE)) {
                $global:LASTEXITCODE = [int]$env:FAKE_DOTNET_DEFAULT_EXIT_CODE
                return
            }

            $global:LASTEXITCODE = 0
            return
        }
    }
}

if ([string]::IsNullOrWhiteSpace($ScriptArgLine)) {
    & $TargetScript
}
else {
    Invoke-Expression "& '$TargetScript' $ScriptArgLine"
}

exit $LASTEXITCODE
""";

    private readonly Dictionary<string, string> _environment = new(StringComparer.OrdinalIgnoreCase);
    private readonly TestTempDirectory _tempDirectory;
    private readonly string _dotnetLogPath;
    private readonly string _wrapperPath;

    private PowerShellScriptHarness(string repoRoot, TestTempDirectory tempDirectory)
    {
        RepositoryRoot = repoRoot;
        _tempDirectory = tempDirectory;
        _dotnetLogPath = Path.Combine(_tempDirectory.DirectoryPath, "dotnet.log");
        _wrapperPath = Path.Combine(_tempDirectory.DirectoryPath, "invoke-script.ps1");
        File.WriteAllText(_wrapperPath, WrapperScript);
    }

    public string RepositoryRoot { get; }

    public static PowerShellScriptHarness Create()
    {
        string? current = AppContext.BaseDirectory;
        while (!string.IsNullOrWhiteSpace(current))
        {
            if (File.Exists(Path.Combine(current, "BatCave.slnx")))
            {
                return new PowerShellScriptHarness(current, TestTempDirectory.Create("batcave-script-tests"));
            }

            current = Directory.GetParent(current)?.FullName;
        }

        throw new DirectoryNotFoundException("Could not locate repository root containing BatCave.slnx.");
    }

    public void ConfigureBuildFailure(string contains, int exitCode)
    {
        _environment["FAKE_DOTNET_BUILD_FAIL_CONTAINS"] = contains;
        _environment["FAKE_DOTNET_BUILD_FAIL_CODE"] = exitCode.ToString(CultureInfo.InvariantCulture);
    }

    public void SetBuildExitCode(int exitCode)
    {
        _environment["FAKE_DOTNET_BUILD_EXIT_CODE"] = exitCode.ToString(CultureInfo.InvariantCulture);
    }

    public void SetRunExitCode(int exitCode)
    {
        _environment["FAKE_DOTNET_RUN_EXIT_CODE"] = exitCode.ToString(CultureInfo.InvariantCulture);
    }

    public ScriptRunResult Run(string scriptName, params string[] scriptArgs)
    {
        if (File.Exists(_dotnetLogPath))
        {
            File.Delete(_dotnetLogPath);
        }

        string scriptPath = Path.Combine(RepositoryRoot, "scripts", scriptName);
        ProcessStartInfo startInfo = new("powershell")
        {
            WorkingDirectory = RepositoryRoot,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        };

        startInfo.ArgumentList.Add("-NoProfile");
        startInfo.ArgumentList.Add("-ExecutionPolicy");
        startInfo.ArgumentList.Add("Bypass");
        startInfo.ArgumentList.Add("-File");
        startInfo.ArgumentList.Add(_wrapperPath);
        startInfo.ArgumentList.Add("-TargetScript");
        startInfo.ArgumentList.Add(scriptPath);
        if (scriptArgs.Length > 0)
        {
            startInfo.ArgumentList.Add("-ScriptArgLine");
            startInfo.ArgumentList.Add(string.Join(" ", scriptArgs));
        }

        startInfo.Environment["FAKE_DOTNET_LOG"] = _dotnetLogPath;
        foreach ((string key, string value) in _environment)
        {
            startInfo.Environment[key] = value;
        }

        using Process process = Process.Start(startInfo)!;
        string standardOutput = process.StandardOutput.ReadToEnd();
        string standardError = process.StandardError.ReadToEnd();
        process.WaitForExit();

        IReadOnlyList<string> invocations = File.Exists(_dotnetLogPath)
            ? File.ReadAllLines(_dotnetLogPath).Where(line => !string.IsNullOrWhiteSpace(line)).ToArray()
            : [];

        return new ScriptRunResult(process.ExitCode, standardOutput, standardError, invocations);
    }

    public void Dispose()
    {
        _tempDirectory.Dispose();
    }
}

internal sealed record ScriptRunResult(
    int ExitCode,
    string StandardOutput,
    string StandardError,
    IReadOnlyList<string> DotnetInvocations);
