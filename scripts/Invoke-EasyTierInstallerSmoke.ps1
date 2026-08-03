[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$InstallerPath,

    [string]$InstallDirectory = (Join-Path $env:ProgramFiles 'Vibe EasyTier'),

    [string]$StateDirectory = (Join-Path $env:ProgramData 'VibeEasyTier'),

    [ValidatePattern('^[A-Za-z0-9_.-]+$')]
    [string]$ServiceName = 'VibeEasyTierService'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Administrator privileges are required for the Vibe EasyTier installer smoke test.'
    }
}

function Invoke-SilentInstaller {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Action
    )

    $process = Start-Process -FilePath $Path -ArgumentList '/S' -PassThru -Wait
    if ($process.ExitCode -ne 0) {
        throw "$Action failed with exit code $($process.ExitCode)."
    }
}

function Wait-ServiceAbsent {
    param(
        [Parameter(Mandatory)]
        [string]$Name,

        [ValidateRange(1, 120)]
        [int]$TimeoutSeconds = 30
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if ($null -eq (Get-Service -Name $Name -ErrorAction SilentlyContinue)) {
            return
        }
        Start-Sleep -Milliseconds 250
    }
    while ([DateTime]::UtcNow -lt $deadline)

    throw "Service $Name still exists after $TimeoutSeconds seconds."
}

function Wait-PathAbsent {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [ValidateRange(1, 120)]
        [int]$TimeoutSeconds = 30
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (-not (Test-Path -LiteralPath $Path)) {
            return
        }
        Start-Sleep -Milliseconds 250
    }
    while ([DateTime]::UtcNow -lt $deadline)

    throw "Path still exists after $TimeoutSeconds seconds: $Path"
}

function Assert-CleanPrecondition {
    param(
        [Parameter(Mandatory)]
        [string]$Name,

        [Parameter(Mandatory)]
        [string]$InstallPath,

        [Parameter(Mandatory)]
        [string]$StatePath
    )

    if ($null -ne (Get-Service -Name $Name -ErrorAction SilentlyContinue)) {
        throw "Refusing to run smoke test because service $Name already exists. Remove the existing Vibe EasyTier installation first."
    }
    if (Test-Path -LiteralPath $InstallPath) {
        throw "Refusing to run smoke test because the installation directory already exists: $InstallPath"
    }
    if (Test-Path -LiteralPath $StatePath) {
        throw "Refusing to run smoke test because the protected state directory already exists: $StatePath"
    }
}

$InstallerPath = [System.IO.Path]::GetFullPath($InstallerPath)
$InstallDirectory = [System.IO.Path]::GetFullPath($InstallDirectory)
$StateDirectory = [System.IO.Path]::GetFullPath($StateDirectory)
if (-not (Test-Path -LiteralPath $InstallerPath -PathType Leaf)) {
    throw "NSIS installer was not found: $InstallerPath"
}

Assert-Administrator
Assert-CleanPrecondition -Name $ServiceName -InstallPath $InstallDirectory -StatePath $StateDirectory

$serviceTestScript = Join-Path $PSScriptRoot 'Test-EasyTierService.ps1'
$serviceBinary = Join-Path $InstallDirectory 'resources\service\vibe-easytier-service.exe'
$runtimeDirectory = Join-Path $InstallDirectory 'resources\easytier'
$iperf3Directory = Join-Path $InstallDirectory 'resources\iperf3'
$uninstaller = Join-Path $InstallDirectory 'uninstall.exe'
$upgradeSentinel = Join-Path $StateDirectory 'upgrade-state-sentinel.txt'
$upgradeSentinelValue = 'vibe-easytier-upgrade-state-preserved'
$installedBySmokeTest = $false

try {
    Invoke-SilentInstaller -Path $InstallerPath -Action 'Fresh NSIS install'
    $installedBySmokeTest = $true

    & $serviceTestScript `
        -ServiceName $ServiceName `
        -ExpectedServiceBinaryPath $serviceBinary `
        -ExpectedRuntimeDirectory $runtimeDirectory `
        -ExpectedIperf3Directory $iperf3Directory `
        -ExpectedStateDirectory $StateDirectory `
        -RequireRunning

    # A same-version installation must update in place. This catches an old
    # uninstaller running during upgrade and deleting durable user intent.
    [System.IO.File]::WriteAllText($upgradeSentinel, $upgradeSentinelValue)
    Invoke-SilentInstaller -Path $InstallerPath -Action 'In-place NSIS upgrade'

    & $serviceTestScript `
        -ServiceName $ServiceName `
        -ExpectedServiceBinaryPath $serviceBinary `
        -ExpectedRuntimeDirectory $runtimeDirectory `
        -ExpectedIperf3Directory $iperf3Directory `
        -ExpectedStateDirectory $StateDirectory `
        -RequireRunning
    if (-not (Test-Path -LiteralPath $upgradeSentinel) -or
        [System.IO.File]::ReadAllText($upgradeSentinel) -ne $upgradeSentinelValue) {
        throw 'In-place NSIS upgrade did not preserve the protected desired-state sentinel.'
    }

    Invoke-SilentInstaller -Path $uninstaller -Action 'NSIS uninstall'
    Wait-ServiceAbsent -Name $ServiceName
    Wait-PathAbsent -Path $StateDirectory

    [pscustomobject]@{
        FreshInstall = 'passed'
        InPlaceUpgrade = 'passed'
        Uninstall = 'passed'
    }
}
finally {
    if ($installedBySmokeTest -and (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
        $cleanup = Start-Process -FilePath $uninstaller -ArgumentList '/S' -PassThru -Wait
        if ($cleanup.ExitCode -ne 0) {
            Write-Warning "Best-effort NSIS cleanup failed with exit code $($cleanup.ExitCode)."
        }
    }
}
