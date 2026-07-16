[CmdletBinding(PositionalBinding = $false)]
param(
    [string]$BaselineInstaller = "",
    [string]$FinalInstaller = "",
    [switch]$SkipBuild,
    [switch]$Run
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$cargoManifest = Join-Path $repoRoot "src/BatCave.App/src-tauri/Cargo.toml"
$planPath = Join-Path $repoRoot "src/BatCave.App/src-tauri/src/windows_lifecycle_proof_plan.v1.json"
$artifactRoot = Join-Path $repoRoot "artifacts/windows-lifecycle-proof"
$builtControllerPath = Join-Path $repoRoot "src/BatCave.App/src-tauri/target/release/batcave-windows-lifecycle-proof.exe"
$controllerPath = Join-Path $artifactRoot "batcave-windows-lifecycle-proof.exe"
$plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json

function Assert-FixedArtifact {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [object]$Candidate,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    $resolved = (Resolve-Path -LiteralPath $Path).Path
    $item = Get-Item -LiteralPath $resolved
    if (-not $item.PSIsContainer -and $item.Length -eq [long]$Candidate.installer_size) {
        $actualSha256 = (Get-FileHash -LiteralPath $resolved -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualSha256 -ceq [string]$Candidate.installer_sha256) {
            Write-Host "Verified $Label artifact: $resolved"
            return $resolved
        }
    }
    throw "$Label artifact does not match the embedded size and SHA-256."
}

function Stage-FixedArtifact {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Source,

        [Parameter(Mandatory = $true)]
        [object]$Candidate,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    $verifiedSource = Assert-FixedArtifact -Path $Source -Candidate $Candidate -Label $Label
    $target = Join-Path $repoRoot ([string]$Candidate.installer_relative_path)
    Copy-Item -LiteralPath $verifiedSource -Destination $target -Force
    Assert-FixedArtifact -Path $target -Candidate $Candidate -Label "$Label staged"
}

function Assert-FixedServiceFixture {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [object]$Fixture
    )

    $resolved = (Resolve-Path -LiteralPath $Path).Path
    $item = Get-Item -LiteralPath $resolved
    if ($item.PSIsContainer -or $item.Length -ne [long]$Fixture.size) {
        throw "The incompatible service fixture size does not match the embedded plan."
    }
    $actualSha256 = (Get-FileHash -LiteralPath $resolved -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualSha256 -cne [string]$Fixture.sha256) {
        throw "The incompatible service fixture SHA-256 does not match the embedded plan."
    }
    if ($item.VersionInfo.ProductVersion -cne [string]$Fixture.product_version) {
        throw "The incompatible service fixture ProductVersion does not match the embedded plan."
    }
    Write-Host "Verified incompatible service fixture: $resolved"
}

if ([string]::IsNullOrWhiteSpace($BaselineInstaller) -xor [string]::IsNullOrWhiteSpace($FinalInstaller)) {
    throw "Specify both -BaselineInstaller and -FinalInstaller, or neither."
}

New-Item -ItemType Directory -Path $artifactRoot -Force | Out-Null
if (-not [string]::IsNullOrWhiteSpace($BaselineInstaller)) {
    Stage-FixedArtifact -Source $BaselineInstaller -Candidate $plan.baseline -Label "baseline"
    Stage-FixedArtifact -Source $FinalInstaller -Candidate $plan.final_candidate -Label "final"
}
else {
    Assert-FixedArtifact -Path (Join-Path $repoRoot ([string]$plan.baseline.installer_relative_path)) -Candidate $plan.baseline -Label "baseline staged" | Out-Null
    Assert-FixedArtifact -Path (Join-Path $repoRoot ([string]$plan.final_candidate.installer_relative_path)) -Candidate $plan.final_candidate -Label "final staged" | Out-Null
}
Assert-FixedServiceFixture -Path (Join-Path $repoRoot ([string]$plan.incompatible_service_fixture.relative_path)) -Fixture $plan.incompatible_service_fixture

$sourceCommit = (git -C $repoRoot rev-parse HEAD).Trim().ToLowerInvariant()
if ($LASTEXITCODE -ne 0 -or $sourceCommit -notmatch '^[0-9a-f]{40}$') {
    throw "Could not resolve the exact controller source commit."
}
$worktreeStatus = git -C $repoRoot status --porcelain=v1 --untracked-files=normal
if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrWhiteSpace(($worktreeStatus -join "`n"))) {
    throw "The controller worktree must be clean before it can be built or run."
}

$env:BATCAVE_SOURCE_COMMIT_SHA = $sourceCommit
if (-not $SkipBuild.IsPresent) {
    cargo build --locked --release --manifest-path $cargoManifest --bin batcave-windows-lifecycle-proof --features private-windows-lifecycle-proof
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}
if (-not (Test-Path -LiteralPath $builtControllerPath -PathType Leaf)) {
    throw "Built lifecycle proof controller not found: $builtControllerPath"
}
$builtControllerSha256 = (Get-FileHash -LiteralPath $builtControllerPath -Algorithm SHA256).Hash
Copy-Item -LiteralPath $builtControllerPath -Destination $controllerPath -Force
$stagedControllerSha256 = (Get-FileHash -LiteralPath $controllerPath -Algorithm SHA256).Hash
if ($stagedControllerSha256 -cne $builtControllerSha256) {
    throw "Staged lifecycle proof controller does not match the built bytes."
}

$action = if ($Run.IsPresent) { "run" } else { "preflight" }
& $controllerPath $action
exit $LASTEXITCODE
