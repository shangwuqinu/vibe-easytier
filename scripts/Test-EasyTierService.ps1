[CmdletBinding()]
param(
    [ValidatePattern('^[A-Za-z0-9_.-]+$')]
    [string]$ServiceName = 'VibeEasyTierService',

    [string]$ExpectedServiceBinaryPath,

    [string]$ExpectedRuntimeDirectory,

    [string]$ExpectedStateDirectory = (Join-Path $env:ProgramData 'VibeEasyTier'),

    [string]$ExpectedOwnerSid,

    [ValidateRange(1, 120)]
    [int]$RegistrationTimeoutSeconds = 30,

    [switch]$RequireRunning
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

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

function Wait-RegisteredService {
    param(
        [Parameter(Mandatory)]
        [string]$Name,

        [Parameter(Mandatory)]
        [int]$TimeoutSeconds,

        [switch]$Running
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $service = Get-CimInstance -ClassName Win32_Service -Filter "Name='$Name'" -ErrorAction SilentlyContinue
        if ($null -ne $service -and (-not $Running -or [string]$service.State -eq 'Running')) {
            return $service
        }
        Start-Sleep -Milliseconds 250
    }
    while ([DateTime]::UtcNow -lt $deadline)

    if ($Running) {
        throw "Service $Name did not reach Running within $TimeoutSeconds seconds."
    }
    throw "Service $Name is not registered after $TimeoutSeconds seconds."
}

function Assert-ImageContains {
    param(
        [Parameter(Mandatory)]
        [string]$ImagePath,

        [Parameter(Mandatory)]
        [string]$Expected,

        [Parameter(Mandatory)]
        [string]$Description
    )

    if ($ImagePath.IndexOf($Expected, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
        throw "The service ImagePath does not contain the expected $($Description): $Expected"
    }
}

$service = Wait-RegisteredService -Name $ServiceName -TimeoutSeconds $RegistrationTimeoutSeconds -Running:$RequireRunning

$imagePath = Get-ServiceImagePath -Name $ServiceName
if ([string]::IsNullOrWhiteSpace($imagePath)) {
    throw "Service $ServiceName has no readable ImagePath."
}

if ($imagePath -notmatch '(^|\s)--service(\s|$)') {
    throw "Service $ServiceName is not configured to run in Windows service mode."
}
if ($imagePath -notmatch '(^|\s)--owner-sid(\s|$)') {
    throw "Service $ServiceName is not configured with a named-pipe owner SID."
}

if (-not [string]::IsNullOrWhiteSpace($ExpectedServiceBinaryPath)) {
    Assert-ImageContains -ImagePath $imagePath -Expected ([System.IO.Path]::GetFullPath($ExpectedServiceBinaryPath)) -Description 'service binary path'
}

if (-not [string]::IsNullOrWhiteSpace($ExpectedRuntimeDirectory)) {
    Assert-ImageContains -ImagePath $imagePath -Expected ([System.IO.Path]::GetFullPath($ExpectedRuntimeDirectory)) -Description 'runtime directory'
}

if (-not [string]::IsNullOrWhiteSpace($ExpectedStateDirectory)) {
    Assert-ImageContains -ImagePath $imagePath -Expected ([System.IO.Path]::GetFullPath($ExpectedStateDirectory)) -Description 'state directory'
}
if (-not [string]::IsNullOrWhiteSpace($ExpectedOwnerSid)) {
    Assert-ImageContains -ImagePath $imagePath -Expected $ExpectedOwnerSid -Description 'named-pipe owner SID'
}

if ([string]$service.StartMode -ne 'Auto') {
    throw "Service $ServiceName is not configured for automatic start. Current mode: $($service.StartMode)"
}

$registryPath = "HKLM:\SYSTEM\CurrentControlSet\Services\$ServiceName"
$delayedAutoStart = (Get-ItemProperty -LiteralPath $registryPath -Name DelayedAutoStart -ErrorAction Stop).DelayedAutoStart
if ([int]$delayedAutoStart -ne 1) {
    throw "Service $ServiceName is not configured for delayed automatic start."
}

$failureActions = (Get-ItemProperty -LiteralPath $registryPath -Name FailureActions -ErrorAction Stop).FailureActions
if ($null -eq $failureActions -or $failureActions.Length -eq 0) {
    throw "Service $ServiceName does not have recovery actions configured."
}

$failureQuery = & sc.exe qfailure $ServiceName 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "Unable to query recovery actions for $ServiceName."
}

$stateAclProtected = $null
if (Test-Path -LiteralPath $ExpectedStateDirectory -PathType Container) {
    $stateAcl = Get-Acl -LiteralPath $ExpectedStateDirectory
    $stateAclProtected = [bool]$stateAcl.AreAccessRulesProtected
    if (-not $stateAclProtected) {
        throw "State directory ACL must be protected from inherited user access: $ExpectedStateDirectory"
    }
}

[pscustomobject]@{
    ServiceName = $ServiceName
    State = [string]$service.State
    StartMode = [string]$service.StartMode
    DelayedAutoStart = [int]$delayedAutoStart
    ImagePath = $imagePath
    StateDirectoryExists = Test-Path -LiteralPath $ExpectedStateDirectory -PathType Container
    StateAclProtected = $stateAclProtected
    RecoveryActions = $failureQuery -join [Environment]::NewLine
}
