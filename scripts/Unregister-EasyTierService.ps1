[CmdletBinding()]
param(
    [ValidatePattern('^[A-Za-z0-9_.-]+$')]
    [string]$ServiceName = 'VibeEasyTierService',

    [string]$ExpectedServiceBinaryPath,

    [ValidateRange(1, 300)]
    [int]$StopTimeoutSeconds = 30,

    # Upgrades must release the running core before replacing bundled files,
    # but keep the SCM record so Register-EasyTierService can retain the
    # original interactive pipe owner on the following POSTINSTALL step.
    [switch]$KeepRegistration,

    [switch]$Force,

    [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$BandwidthFirewallRuleName = "$ServiceName-BandwidthTest"

function Assert-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Administrator privileges are required to unregister a Windows service.'
    }
}

function Remove-BandwidthFirewallRule {
    if ($DryRun) {
        Write-Output "DRY-RUN: remove firewall rule $BandwidthFirewallRuleName"
        return
    }

    Remove-NetFirewallRule -Name $BandwidthFirewallRuleName -ErrorAction SilentlyContinue
}

function Get-ServiceImagePath {
    param(
        [Parameter(Mandatory)]
        [string]$Name
    )

    $registryPath = "HKLM:\SYSTEM\CurrentControlSet\Services\$Name"
    if (-not (Test-Path -LiteralPath $registryPath)) {
        return $null
    }

    return [string](Get-ItemProperty -LiteralPath $registryPath -Name ImagePath).ImagePath
}

function Test-ServiceRegistrationExists {
    param(
        [Parameter(Mandatory)]
        [string]$Name
    )

    return Test-Path -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Services\$Name"
}

function Test-ServiceImageOwnership {
    param(
        [AllowNull()]
        [string]$ImagePath,

        [Parameter(Mandatory)]
        [string]$ExpectedBinaryPath
    )

    if ([string]::IsNullOrWhiteSpace($ImagePath)) {
        return $false
    }

    return $ImagePath.IndexOf($ExpectedBinaryPath, [StringComparison]::OrdinalIgnoreCase) -ge 0
}

function ConvertTo-WindowsCommandLineArgument {
    param(
        [AllowNull()]
        [AllowEmptyString()]
        [string]$Value
    )

    if ($null -eq $Value -or $Value.Length -eq 0) {
        return '""'
    }

    if ($Value -notmatch '[\s"]') {
        return $Value
    }

    # ProcessStartInfo.Arguments is a single Windows command line. Escape the
    # value using CommandLineToArgvW-compatible quoting before handing it to
    # sc.exe without a shell.
    $builder = [System.Text.StringBuilder]::new()
    [void]$builder.Append([char]34)
    $backslashCount = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq [char]92) {
            $backslashCount++
            continue
        }

        if ($character -eq [char]34) {
            [void]$builder.Append([char]92, ($backslashCount * 2) + 1)
            [void]$builder.Append([char]34)
            $backslashCount = 0
            continue
        }

        if ($backslashCount -gt 0) {
            [void]$builder.Append([char]92, $backslashCount)
            $backslashCount = 0
        }
        [void]$builder.Append($character)
    }

    if ($backslashCount -gt 0) {
        [void]$builder.Append([char]92, $backslashCount * 2)
    }
    [void]$builder.Append([char]34)
    return $builder.ToString()
}

function Invoke-Sc {
    param(
        [Parameter(Mandatory)]
        [string[]]$Arguments
    )

    $display = $Arguments -join ' '
    if ($DryRun) {
        Write-Output "DRY-RUN: sc.exe $display"
        return
    }

    $scPath = Join-Path $env:SystemRoot 'System32\sc.exe'
    if (-not (Test-Path -LiteralPath $scPath -PathType Leaf)) {
        throw "sc.exe was not found: $scPath"
    }

    # A direct PowerShell invocation of this console executable can flash a
    # System32\sc.exe window when NSIS hosts PowerShell without a console.
    # Use an explicitly shell-less, redirected process so it remains invisible
    # while preserving stdout/stderr and the exact process exit code.
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $scPath
    $startInfo.Arguments = (($Arguments | ForEach-Object {
                ConvertTo-WindowsCommandLineArgument -Value $_
            }) -join ' ')
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = [System.Diagnostics.Process]::new()
    try {
        $process.StartInfo = $startInfo
        if (-not $process.Start()) {
            throw "sc.exe $display could not be started."
        }

        $standardOutputTask = $process.StandardOutput.ReadToEndAsync()
        $standardErrorTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        $standardOutput = $standardOutputTask.GetAwaiter().GetResult()
        $standardError = $standardErrorTask.GetAwaiter().GetResult()
        $exitCode = $process.ExitCode
    }
    finally {
        $process.Dispose()
    }

    if ($exitCode -ne 0) {
        $details = (@($standardOutput, $standardError) |
                Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
                ForEach-Object { $_.TrimEnd() }) -join [Environment]::NewLine
        throw "sc.exe $display failed with exit code $exitCode. $details"
    }
}

function Wait-ServiceDeletion {
    param(
        [Parameter(Mandatory)]
        [string]$Name,

        [Parameter(Mandatory)]
        [int]$TimeoutSeconds
    )

    if ($DryRun) {
        Write-Output "DRY-RUN: wait for SCM to fully delete $Name"
        return
    }

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $remaining = Get-Service -Name $Name -ErrorAction SilentlyContinue
        $registryExists = Test-ServiceRegistrationExists -Name $Name
        if ($null -eq $remaining -and -not $registryExists) {
            return
        }
        Start-Sleep -Milliseconds 250
    }
    while ([DateTime]::UtcNow -lt $deadline)

    throw "Service $Name remains marked for deletion after $TimeoutSeconds seconds. Close applications holding a service handle and retry the installation."
}

if (-not [string]::IsNullOrWhiteSpace($ExpectedServiceBinaryPath)) {
    $ExpectedServiceBinaryPath = [System.IO.Path]::GetFullPath($ExpectedServiceBinaryPath)
}
elseif (-not $Force) {
    throw 'ExpectedServiceBinaryPath is required unless -Force is explicitly supplied.'
}

$service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
if ($null -eq $service) {
    if (Test-ServiceRegistrationExists -Name $ServiceName) {
        Wait-ServiceDeletion -Name $ServiceName -TimeoutSeconds $StopTimeoutSeconds
    }
    if (-not $KeepRegistration) {
        if (-not $DryRun) {
            Assert-Administrator
        }
        Remove-BandwidthFirewallRule
    }
    Write-Output "Service $ServiceName is not registered."
    return
}

$imagePath = Get-ServiceImagePath -Name $ServiceName
if (-not [string]::IsNullOrWhiteSpace($ExpectedServiceBinaryPath)) {
    if (-not (Test-ServiceImageOwnership -ImagePath $imagePath -ExpectedBinaryPath $ExpectedServiceBinaryPath)) {
        if (-not $Force) {
            throw "Service $ServiceName does not point to the expected service binary. Refusing to remove it."
        }

        Write-Warning "Removing service $ServiceName whose ImagePath does not match the supplied service binary because -Force was specified."
    }
}

if ($DryRun) {
    Write-Output "DRY-RUN: stop service $ServiceName if it is running"
    if ($KeepRegistration) {
        Write-Output "DRY-RUN: preserve the SCM registration for a Vibe EasyTier upgrade"
        return
    }
    Invoke-Sc -Arguments @('delete', $ServiceName)
    Remove-BandwidthFirewallRule
    return
}

Assert-Administrator
if ($service.Status -ne [System.ServiceProcess.ServiceControllerStatus]::Stopped) {
    Stop-Service -Name $ServiceName -Force
    $service.WaitForStatus([System.ServiceProcess.ServiceControllerStatus]::Stopped, [TimeSpan]::FromSeconds($StopTimeoutSeconds))
}

# ServiceController can retain an SCM handle after the status query. Release it
# before `sc delete`, otherwise this script itself can keep the registration
# marked for deletion through the following installer step.
$service.Dispose()

if ($KeepRegistration) {
    Write-Output "Stopped $ServiceName and preserved its SCM registration for upgrade. ProgramData state was not deleted."
    return
}

Invoke-Sc -Arguments @('delete', $ServiceName)
Wait-ServiceDeletion -Name $ServiceName -TimeoutSeconds $StopTimeoutSeconds
Remove-BandwidthFirewallRule
Write-Output "Unregistered $ServiceName. ProgramData state was not deleted."
