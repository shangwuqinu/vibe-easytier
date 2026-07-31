[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64')]
    [string]$Architecture = 'x64',

    [switch]$RequireRuntime,

    [switch]$RequireServiceBinary,

    [switch]$VerifyReleaseMetadata
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-File {
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

function Assert-PowerShellSyntax {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    $tokens = $null
    $parseErrors = $null
    [System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$parseErrors) | Out-Null
    if ($parseErrors.Count -gt 0) {
        $details = $parseErrors | ForEach-Object { $_.Message }
        throw "PowerShell parser errors in $Path. $($details -join [Environment]::NewLine)"
    }
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repositoryRoot 'resources\easytier-runtime.manifest.json'
Assert-File -Path $manifestPath -Description 'EasyTier runtime manifest'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json

if ([string]$manifest.version -ne '2.6.4' -or [string]$manifest.upstream.tag -ne 'v2.6.4') {
    throw 'EasyTier runtime manifest is not pinned to v2.6.4.'
}

foreach ($arch in @('x64', 'arm64')) {
    $asset = $manifest.assets.$arch
    if ($null -eq $asset) {
        throw "Missing $arch asset in the EasyTier runtime manifest."
    }
    if (-not ([string]$asset.sha256 -match '^[0-9a-fA-F]{64}$')) {
        throw "Invalid SHA-256 for $arch."
    }
    if ([int64]$asset.size -le 0) {
        throw "Invalid asset size for $arch."
    }
    if ([string]$asset.url -notmatch "/v2\.6\.4/") {
        throw "Runtime URL for $arch is not pinned to v2.6.4."
    }
}

$scriptNames = @(
    'Fetch-EasyTierRuntime.ps1',
    'Stage-VibeEasyTierService.ps1',
    'Register-EasyTierService.ps1',
    'Unregister-EasyTierService.ps1',
    'Remove-VibeEasyTierState.ps1',
    'Invoke-EasyTierInstallerSmoke.ps1',
    'Test-EasyTierService.ps1',
    'Test-EasyTierPackaging.ps1'
)
foreach ($scriptName in $scriptNames) {
    $scriptPath = Join-Path $PSScriptRoot $scriptName
    Assert-File -Path $scriptPath -Description 'Packaging script'
    Assert-PowerShellSyntax -Path $scriptPath
}

$fragmentPath = Join-Path $repositoryRoot "installer\tauri-nsis.$Architecture.fragment.json"
Assert-File -Path $fragmentPath -Description 'Tauri NSIS fragment'
$fragment = Get-Content -LiteralPath $fragmentPath -Raw | ConvertFrom-Json
if ([string]$fragment.bundle.windows.nsis.installMode -ne 'perMachine') {
    throw 'Tauri NSIS fragment must use perMachine installation mode.'
}
if ([string]$fragment.bundle.windows.nsis.template -ne '../installer/tauri-nsis.template.nsi') {
    throw 'Tauri NSIS fragment must use the Vibe EasyTier in-place-upgrade template.'
}
foreach ($fragmentArchitecture in @('x64', 'arm64')) {
    $candidatePath = Join-Path $repositoryRoot "installer\tauri-nsis.$fragmentArchitecture.fragment.json"
    Assert-File -Path $candidatePath -Description "$fragmentArchitecture Tauri NSIS fragment"
    $candidate = Get-Content -LiteralPath $candidatePath -Raw | ConvertFrom-Json
    if ([string]$candidate.bundle.windows.nsis.template -ne '../installer/tauri-nsis.template.nsi') {
        throw "$fragmentArchitecture Tauri NSIS fragment must use the Vibe EasyTier in-place-upgrade template."
    }
}

$expectedRuntimeResource = "../resources/easytier/windows-$Architecture"
$expectedServiceResource = "../resources/service/windows-$Architecture"
if ([string]$fragment.bundle.resources.$expectedRuntimeResource -ne 'resources/easytier') {
    throw 'Tauri NSIS fragment does not map the EasyTier runtime to the easytier resource directory.'
}
if ([string]$fragment.bundle.resources.$expectedServiceResource -ne 'resources/service') {
    throw 'Tauri NSIS fragment does not map the service host to the service resource directory.'
}
if ([string]$fragment.bundle.resources.'../scripts/Remove-VibeEasyTierState.ps1' -ne 'resources/scripts/Remove-VibeEasyTierState.ps1') {
    throw 'Tauri NSIS fragment does not bundle the state-cleanup script.'
}

$tauriConfigPath = Join-Path $repositoryRoot 'src-tauri\tauri.conf.json'
Assert-File -Path $tauriConfigPath -Description 'Tauri configuration'
$tauriConfig = Get-Content -LiteralPath $tauriConfigPath -Raw | ConvertFrom-Json
if ([string]$tauriConfig.bundle.windows.nsis.installMode -ne 'perMachine') {
    throw 'The active Tauri configuration must use perMachine installation mode.'
}
if ([string]$tauriConfig.bundle.windows.nsis.installerHooks -ne '../installer/tauri-nsis-hooks.nsh') {
    throw 'The active Tauri configuration does not use the Vibe EasyTier NSIS hooks.'
}
if ([string]$tauriConfig.bundle.windows.nsis.template -ne '../installer/tauri-nsis.template.nsi') {
    throw 'The active Tauri configuration does not use the Vibe EasyTier NSIS in-place-upgrade template.'
}
if ([string]$tauriConfig.bundle.resources.$expectedRuntimeResource -ne 'resources/easytier') {
    throw 'The active Tauri configuration does not map the EasyTier runtime resource.'
}
if ([string]$tauriConfig.bundle.resources.$expectedServiceResource -ne 'resources/service') {
    throw 'The active Tauri configuration does not map the service host resource.'
}
if ([string]$tauriConfig.bundle.resources.'../scripts/Remove-VibeEasyTierState.ps1' -ne 'resources/scripts/Remove-VibeEasyTierState.ps1') {
    throw 'The active Tauri configuration does not bundle the state-cleanup script.'
}

$hooksPath = Join-Path $repositoryRoot 'installer\tauri-nsis-hooks.nsh'
Assert-File -Path $hooksPath -Description 'Tauri NSIS hooks'
$hooks = Get-Content -LiteralPath $hooksPath -Raw
foreach ($macro in @('NSIS_HOOK_PREINSTALL', 'NSIS_HOOK_POSTINSTALL', 'NSIS_HOOK_PREUNINSTALL')) {
    if ($hooks.IndexOf($macro, [StringComparison]::Ordinal) -lt 0) {
        throw "Missing $macro in the Tauri NSIS hooks."
    }
}
if ($hooks.IndexOf('$COMMONAPPDATA\VibeEasyTier\state.v1.json', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
    throw 'NSIS hooks must register the service on a first install without requiring a state file.'
}
if ($hooks.IndexOf('$COMMONAPPDATA', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
    throw 'NSIS hooks must use the service scripts'' ProgramData defaults rather than an unsupported NSIS common-app-data variable.'
}
if ($hooks.IndexOf('Remove-VibeEasyTierState.ps1', [StringComparison]::OrdinalIgnoreCase) -lt 0) {
    throw 'NSIS hooks must clean the Vibe EasyTier ProgramData state during uninstall.'
}
if ($hooks.IndexOf('NSIS_HOOK_PREINSTALL', [StringComparison]::Ordinal) -lt 0 -or $hooks.IndexOf('-KeepRegistration', [StringComparison]::OrdinalIgnoreCase) -lt 0) {
    throw 'NSIS preinstall must stop the old service without deleting its owner-bearing SCM registration.'
}
if ($hooks.IndexOf('${GetOptions} $CMDLINE "_?="', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
    throw 'NSIS hooks must not treat NSIS _?= self-copy arguments as upgrade markers.'
}
if ($hooks.IndexOf('/VIBE_EASYTIER_UPGRADE', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
    throw 'NSIS hooks must not rely on an old full-uninstaller upgrade marker.'
}
if ($hooks.IndexOf('Abort "Vibe EasyTier could not register its boot service.', [StringComparison]::OrdinalIgnoreCase) -lt 0) {
    throw 'NSIS hooks must fail installation when the boot service cannot be registered.'
}
if ($hooks.IndexOf('Abort "Vibe EasyTier could not remove its service.', [StringComparison]::OrdinalIgnoreCase) -lt 0) {
    throw 'NSIS hooks must fail uninstallation when the boot service cannot be removed.'
}

$registerScript = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'Register-EasyTierService.ps1') -Raw
if ($registerScript.IndexOf('Resolve-InteractiveOwnerSid', [StringComparison]::Ordinal) -lt 0 -or $registerScript.IndexOf('-IncludeUserName', [StringComparison]::Ordinal) -lt 0 -or $registerScript.IndexOf('WindowsIdentity]::GetCurrent().User.Value', [StringComparison]::Ordinal) -lt 0) {
    throw 'Service registration must derive the pipe owner from the interactive desktop rather than the elevated installer token.'
}
if ($registerScript.IndexOf('Get-ServiceOwnerSid', [StringComparison]::Ordinal) -lt 0 -or $registerScript.IndexOf('existingServiceOwnedByThisInstall', [StringComparison]::Ordinal) -lt 0) {
    throw 'Service registration must preserve the established pipe owner when an authorized upgrade runs under a different administrator.'
}
if ($registerScript -notmatch '(?s)\[string\]\$StateDirectory\s*=\s*\(Join-Path\s+\$env:ProgramData\s+''VibeEasyTier''\)') {
    throw 'Service registration must default protected state to ProgramData when called by NSIS hooks.'
}

$unregisterScript = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'Unregister-EasyTierService.ps1') -Raw
if ($unregisterScript.IndexOf('Wait-ServiceDeletion', [StringComparison]::Ordinal) -lt 0) {
    throw 'Service removal must wait for SCM to finish deleting the old service before an upgrade recreates it.'
}
if ($unregisterScript.IndexOf('KeepRegistration', [StringComparison]::Ordinal) -lt 0) {
    throw 'Service removal must support upgrade-only stop behavior that preserves the established SCM registration.'
}
foreach ($serviceScript in @($registerScript, $unregisterScript)) {
    if ($serviceScript -match '(?m)&\s*sc\.exe') {
        throw 'Service scripts must not directly invoke the console sc.exe process from the NSIS PowerShell host.'
    }
    foreach ($requiredFragment in @(
            '[System.Diagnostics.ProcessStartInfo]::new()',
            '$startInfo.UseShellExecute = $false',
            '$startInfo.CreateNoWindow = $true',
            '$startInfo.RedirectStandardOutput = $true',
            '$startInfo.RedirectStandardError = $true',
            'ConvertTo-WindowsCommandLineArgument'
        )) {
        if ($serviceScript.IndexOf($requiredFragment, [StringComparison]::Ordinal) -lt 0) {
            throw "Service scripts must run sc.exe invisibly with redirected output. Missing: $requiredFragment"
        }
    }
}

$templatePath = Join-Path $repositoryRoot 'installer\tauri-nsis.template.nsi'
Assert-File -Path $templatePath -Description 'custom Tauri NSIS template'
$template = Get-Content -LiteralPath $templatePath -Raw
$forceInPlaceOffset = $template.IndexOf('VIBE_EASYTIER_FORCE_IN_PLACE_REINSTALL', [StringComparison]::Ordinal)
$compareVersionOffset = $template.IndexOf('compare_version:', [StringComparison]::Ordinal)
if ($forceInPlaceOffset -lt 0 -or $compareVersionOffset -lt $forceInPlaceOffset) {
    throw 'The custom NSIS template must skip the generic maintenance page for existing Vibe NSIS installations.'
}
if ($template -notmatch '(?s)VIBE_EASYTIER_FORCE_IN_PLACE_REINSTALL.*?\$\{If\} \$WixMode <> 1\s+Abort\s+\$\{EndIf\}') {
    throw 'The custom NSIS template must force existing non-WiX Vibe installations to remain in place.'
}
$inPlaceGuardOffset = $template.IndexOf('VIBE_EASYTIER_IN_PLACE_UPGRADE', [StringComparison]::Ordinal)
$oldUninstallerOffset = $template.IndexOf("ExecWait '`$R1' `$0", [StringComparison]::Ordinal)
if ($inPlaceGuardOffset -lt 0 -or $oldUninstallerOffset -lt $inPlaceGuardOffset) {
    throw 'The custom NSIS template must bypass the old full uninstaller before a Vibe EasyTier in-place upgrade.'
}
if ($template.IndexOf('/VIBE_EASYTIER_UPGRADE', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
    throw 'The custom NSIS template must not use an old-uninstaller marker that can orphan the boot service if the new install is cancelled.'
}
if ($template.IndexOf('tauri-cli-v2.11.4', [StringComparison]::Ordinal) -lt 0) {
    throw 'The custom NSIS template must identify the pinned Tauri CLI source it was derived from.'
}
$desktopPackage = Get-Content -LiteralPath (Join-Path $repositoryRoot 'apps\desktop\package.json') -Raw | ConvertFrom-Json
if ([string]$desktopPackage.devDependencies.'@tauri-apps/cli' -ne '2.11.4') {
    throw 'The Tauri CLI must remain exactly pinned while the NSIS template carries a targeted upgrade-flow modification.'
}
foreach ($desktopScript in @('desktop:dev', 'desktop:build')) {
    if ([string]$desktopPackage.scripts.$desktopScript -notmatch 'cd\s+\.\./\.\./src-tauri') {
        throw "$desktopScript must run Tauri from src-tauri so normal npm commands discover the packaged Tauri project."
    }
}

$runtimeDirectory = Join-Path $repositoryRoot "resources\easytier\windows-$Architecture"
$runtimeCore = Join-Path $runtimeDirectory 'easytier-core.exe'
$runtimeCli = Join-Path $runtimeDirectory 'easytier-cli.exe'
if (Test-Path -LiteralPath $runtimeDirectory -PathType Container) {
    Assert-File -Path $runtimeCore -Description 'Staged EasyTier core'
    Assert-File -Path $runtimeCli -Description 'Staged EasyTier CLI'
    $runtimeVersion = (& $runtimeCore --version 2>&1) -join [Environment]::NewLine
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to run the staged EasyTier core. Exit code: $LASTEXITCODE. Output: $runtimeVersion"
    }
    if ($runtimeVersion -notmatch 'easytier-core\s+2\.6\.4') {
        throw "Staged EasyTier core does not report version 2.6.4. Output: $runtimeVersion"
    }
}
elseif ($RequireRuntime) {
    throw "Staged EasyTier runtime is required but missing: $runtimeDirectory"
}
else {
    Write-Output "Runtime not staged yet: $runtimeDirectory"
}

$serviceBinary = Join-Path $repositoryRoot "resources\service\windows-$Architecture\vibe-easytier-service.exe"
if (-not (Test-Path -LiteralPath $serviceBinary -PathType Leaf)) {
    if ($RequireServiceBinary) {
        throw "Staged Vibe EasyTier service is required but missing: $serviceBinary"
    }
    Write-Output "Service host not staged yet: $serviceBinary"
}

if ($VerifyReleaseMetadata) {
    & (Join-Path $PSScriptRoot 'Fetch-EasyTierRuntime.ps1') -Architecture $Architecture -DryRun
}

Write-Output "Packaging assets passed static validation for $Architecture."
