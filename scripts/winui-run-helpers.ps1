function Get-WinUiRunArguments {
    param(
        [string]$ProjectPath,
        [string]$RuntimePlatform,
        [string[]]$CommandArgs = @()
    )

    $arguments = @(
        "run",
        "--no-launch-profile",
        "--project", $ProjectPath,
        "-p:Platform=$RuntimePlatform",
        "-p:WindowsPackageType=None",
        "-p:GenerateAppxPackageOnBuild=false",
        "-p:WindowsAppSdkBootstrapInitialize=true",
        "-p:WindowsAppSdkDeploymentManagerInitialize=false"
    )

    if ($CommandArgs.Count -gt 0) {
        $arguments += "--"
        $arguments += $CommandArgs
    }

    return $arguments
}
