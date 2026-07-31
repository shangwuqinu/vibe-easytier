[CmdletBinding()]
param(
    [string]$StateDirectory = (Join-Path $env:ProgramData 'VibeEasyTier'),

    [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Administrator privileges are required to remove Vibe EasyTier state.'
    }
}

function Get-CanonicalPath {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    return [System.IO.Path]::GetFullPath($Path).TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
}

$expectedStateDirectory = Get-CanonicalPath -Path (Join-Path $env:ProgramData 'VibeEasyTier')
$targetStateDirectory = Get-CanonicalPath -Path $StateDirectory
if (-not [string]::Equals($targetStateDirectory, $expectedStateDirectory, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to delete a state directory outside the Vibe EasyTier ProgramData location: $targetStateDirectory"
}

if ($DryRun) {
    Write-Output "DRY-RUN: remove Vibe EasyTier state directory $targetStateDirectory"
    return
}

Assert-Administrator
if (-not (Test-Path -LiteralPath $targetStateDirectory -PathType Container)) {
    Write-Output "Vibe EasyTier state directory does not exist: $targetStateDirectory"
    return
}

$item = Get-Item -LiteralPath $targetStateDirectory -Force
if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Refusing to recursively remove a reparse-point state directory: $targetStateDirectory"
}

# The target is canonicalized and fixed to %ProgramData%\VibeEasyTier above.
Remove-Item -LiteralPath $targetStateDirectory -Recurse -Force
Write-Output "Removed Vibe EasyTier state directory: $targetStateDirectory"
