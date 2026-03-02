param(
    [string]$Platform = "ARM64"
)

$ErrorActionPreference = "Stop"

function Assert-LastExitCode {
    param(
        [string]$CommandName
    )

    if ($LASTEXITCODE -ne 0) {
        throw "$CommandName failed with exit code $LASTEXITCODE."
    }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    Write-Host "Validating WinUI compile path (Platform=$Platform)..."
    dotnet build BatCave/BatCave.csproj -p:Platform=$Platform
    Assert-LastExitCode "dotnet build BatCave/BatCave.csproj"

    Write-Host "Running solution tests..."
    dotnet test BatCave.slnx
    Assert-LastExitCode "dotnet test BatCave.slnx"

    Write-Host "Validation complete."
}
finally {
    Pop-Location
}
