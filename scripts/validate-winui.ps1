param(
    [string]$Platform = "ARM64"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    Write-Host "Validating WinUI compile path (Platform=$Platform)..."
    dotnet build BatCave/BatCave.csproj -p:Platform=$Platform

    Write-Host "Running solution tests..."
    dotnet test BatCave.slnx

    Write-Host "Validation complete."
}
finally {
    Pop-Location
}
