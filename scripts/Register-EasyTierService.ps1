[CmdletBinding()]
param(
    [ValidatePattern('^[A-Za-z0-9_.-]+$')]
    [string]$ServiceName = 'VibeEasyTierService',

    [string]$DisplayName = 'Vibe EasyTier Service',

    [Parameter(Mandatory)]
    [string]$ServiceBinaryPath,

    [Parameter(Mandatory)]
    [string]$RuntimeDirectory,

    [string]$StateDirectory = (Join-Path $env:ProgramData 'VibeEasyTier'),

    [string]$OwnerSid,

    [ValidateRange(1, 300)]
    [int]$StopTimeoutSeconds = 30,

    [switch]$NoStart,

    [switch]$Force,

    [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Administrator privileges are required to register a Windows service.'
    }
}

function Resolve-InteractiveOwnerSid {
    param(
        [AllowNull()]
        [string]$RequestedSid
    )

    if (-not [string]::IsNullOrWhiteSpace($RequestedSid)) {
        return $RequestedSid
    }

    # UAC can run this script under a different administrator account. The
    # named-pipe ACL must instead grant the user who owns the interactive
    # desktop, otherwise the installed client cannot reach its own service.
    $sessionId = (Get-Process -Id $PID -ErrorAction Stop).SessionId
    $owners = @(Get-Process -Name explorer -IncludeUserName -ErrorAction SilentlyContinue |
        Where-Object { $_.SessionId -eq $sessionId -and -not [string]::IsNullOrWhiteSpace($_.UserName) } |
        Select-Object -ExpandProperty UserName -Unique)

    if ($owners.Count -eq 1) {
        $account = [Security.Principal.NTAccount]::new([string]$owners[0])
        return $account.Translate([Security.Principal.SecurityIdentifier]).Value
    }

    if ($owners.Count -eq 0) {
        # Silent installers and managed deployment agents often have no Explorer
        # shell. There is no separate desktop identity to preserve in that
        # case, so grant the installing account rather than refusing a valid
        # noninteractive per-machine deployment.
        $currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        if (-not [string]::IsNullOrWhiteSpace($currentSid)) {
            Write-Warning 'No interactive Explorer shell was found; using the installing account as the Vibe EasyTier pipe owner.'
            return $currentSid
        }
    }

    throw 'OwnerSid was not supplied and the interactive desktop user could not be determined. Re-run with -OwnerSid <SID>.'
}

function Assert-ExistingFile {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Description
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Description was not found: $Path"
    }
}

function Assert-ExistingDirectory {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Description
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "$Description was not found: $Path"
    }
}

function Protect-StateDirectory {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    if ($DryRun) {
        Write-Output "DRY-RUN: create and restrict $Path to LocalSystem and Administrators"
        return
    }

    New-Item -ItemType Directory -Path $Path -Force | Out-Null

    # State contains DPAPI-protected network material. Normal desktop users use
    # the named-pipe boundary; they never need direct ProgramData access.
    $acl = [System.Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true, $false)
    $inheritance = [System.Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [System.Security.AccessControl.InheritanceFlags]::ObjectInherit
    $propagation = [System.Security.AccessControl.PropagationFlags]::None
    $allow = [System.Security.AccessControl.AccessControlType]::Allow
    $full = [System.Security.AccessControl.FileSystemRights]::FullControl

    foreach ($sidText in @('S-1-5-18', 'S-1-5-32-544')) {
        $sid = [System.Security.Principal.SecurityIdentifier]::new($sidText)
        $rule = [System.Security.AccessControl.FileSystemAccessRule]::new($sid, $full, $inheritance, $propagation, $allow)
        [void]$acl.AddAccessRule($rule)
    }

    Set-Acl -LiteralPath $Path -AclObject $acl
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

function Get-ServiceOwnerSid {
    param(
        [AllowNull()]
        [string]$ImagePath
    )

    if ([string]::IsNullOrWhiteSpace($ImagePath)) {
        return $null
    }

    $match = [regex]::Match(
        $ImagePath,
        '(?i)(?:^|\s)--owner-sid\s+(?:"(?<sid>S-1-[0-9-]+)"|(?<sid>S-1-[0-9-]+))'
    )
    if (-not $match.Success) {
        return $null
    }

    return $match.Groups['sid'].Value
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

function Stop-ExistingService {
    param(
        [Parameter(Mandatory)]
        [string]$Name
    )

    if ($DryRun) {
        Write-Output "DRY-RUN: stop service $Name if it is running"
        return
    }

    $service = Get-Service -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $service -or $service.Status -eq [System.ServiceProcess.ServiceControllerStatus]::Stopped) {
        return
    }

    Stop-Service -Name $Name -Force
    $service.WaitForStatus([System.ServiceProcess.ServiceControllerStatus]::Stopped, [TimeSpan]::FromSeconds($StopTimeoutSeconds))
}

function Set-ServiceRegistration {
    param(
        [Parameter(Mandatory)]
        [string]$Name,

        [Parameter(Mandatory)]
        [string]$Display,

        [Parameter(Mandatory)]
        [string]$BinaryPath,

        [Parameter(Mandatory)]
        [bool]$Exists
    )

    if ($DryRun) {
        if ($Exists) {
            Write-Output "DRY-RUN: update service $Name with its new binary path"
        }
        else {
            Write-Output "DRY-RUN: create service $Name with its binary path"
        }
        return
    }

    if (-not $Exists) {
        # New-Service passes BinaryPathName through the Service Control Manager
        # API rather than through cmd.exe argument parsing. That preserves the
        # quoted executable and its service-only arguments on Windows PowerShell.
        New-Service -Name $Name -BinaryPathName $BinaryPath -DisplayName $Display -StartupType Automatic | Out-Null
        return
    }

    $service = Get-CimInstance -ClassName Win32_Service -Filter "Name='$Name'" -ErrorAction Stop
    if ($null -eq $service) {
        throw "Service $Name disappeared before its registration could be updated."
    }

    $change = Invoke-CimMethod -InputObject $service -MethodName Change -Arguments @{
        DisplayName = $Display
        PathName = $BinaryPath
        StartMode = 'Automatic'
        StartName = 'LocalSystem'
    }
    if ([uint32]$change.ReturnValue -ne 0) {
        throw "Win32_Service.Change failed while updating $Name. Return value: $($change.ReturnValue)"
    }
}

$ServiceBinaryPath = [System.IO.Path]::GetFullPath($ServiceBinaryPath)
$RuntimeDirectory = [System.IO.Path]::GetFullPath($RuntimeDirectory)
$StateDirectory = [System.IO.Path]::GetFullPath($StateDirectory)
$existingService = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
$existingServiceOwnedByThisInstall = $false
$existingImagePath = $null
if ($null -ne $existingService) {
    $existingImagePath = Get-ServiceImagePath -Name $ServiceName
    $existingServiceOwnedByThisInstall = Test-ServiceImageOwnership -ImagePath $existingImagePath -ExpectedBinaryPath $ServiceBinaryPath
    if (-not $existingServiceOwnedByThisInstall) {
        if (-not $Force) {
            throw "Service $ServiceName already exists but is not owned by $ServiceBinaryPath. Refusing to replace it without -Force."
        }

        Write-Warning "Replacing service $ServiceName whose ImagePath does not match the supplied service binary because -Force was specified."
    }

    # Preserve the service's established desktop owner during a system-wide
    # update initiated by a different administrator. An explicit -OwnerSid is
    # still the supported way to deliberately transfer that access.
    if ([string]::IsNullOrWhiteSpace($OwnerSid) -and $existingServiceOwnedByThisInstall) {
        $OwnerSid = Get-ServiceOwnerSid -ImagePath $existingImagePath
    }
}

$OwnerSid = Resolve-InteractiveOwnerSid -RequestedSid $OwnerSid
$corePath = Join-Path $RuntimeDirectory 'easytier-core.exe'
$quote = [string][char]34
$serviceCommand = "$quote$ServiceBinaryPath$quote --service --state-root $quote$StateDirectory$quote --core $quote$corePath$quote --owner-sid $quote$OwnerSid$quote"

if ([string]::IsNullOrWhiteSpace($OwnerSid) -or $OwnerSid -notmatch '^S-1-') {
    throw 'OwnerSid must be a Windows security identifier.'
}

if (-not $DryRun) {
    Assert-Administrator
    Assert-ExistingFile -Path $ServiceBinaryPath -Description 'Vibe EasyTier service binary'
    Assert-ExistingDirectory -Path $RuntimeDirectory -Description 'EasyTier runtime directory'
    Assert-ExistingFile -Path $corePath -Description 'EasyTier core binary'
    Protect-StateDirectory -Path $StateDirectory
}

if ($null -ne $existingService) {

    Stop-ExistingService -Name $ServiceName
    Set-ServiceRegistration -Name $ServiceName -Display $DisplayName -BinaryPath $serviceCommand -Exists $true
}
else {
    Set-ServiceRegistration -Name $ServiceName -Display $DisplayName -BinaryPath $serviceCommand -Exists $false
}

Invoke-Sc -Arguments @('config', $ServiceName, 'start=', 'delayed-auto')

Invoke-Sc -Arguments @(
    'description',
    $ServiceName,
    'Supervises EasyTier and reconnects the selected private network after Windows starts.'
)
Invoke-Sc -Arguments @(
    'failure',
    $ServiceName,
    'reset=',
    '86400',
    'actions=',
    'restart/5000/restart/15000/restart/60000'
)
Invoke-Sc -Arguments @('failureflag', $ServiceName, '1')

if (-not $NoStart) {
    if ($DryRun) {
        Write-Output "DRY-RUN: start service $ServiceName"
    }
    else {
        Start-Service -Name $ServiceName
        $service = Get-Service -Name $ServiceName
        $service.WaitForStatus([System.ServiceProcess.ServiceControllerStatus]::Running, [TimeSpan]::FromSeconds($StopTimeoutSeconds))
    }
}

Write-Output "Registered $ServiceName with delayed automatic start and recovery actions."
