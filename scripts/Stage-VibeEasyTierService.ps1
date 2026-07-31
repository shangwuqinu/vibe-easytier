[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64')]
    [string]$Architecture = 'x64',

    [string]$ServiceBinary,

    [string]$Destination = (Join-Path (Split-Path -Parent $PSScriptRoot) 'resources\service'),

    [switch]$Force,

    [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-PortableExecutableArchitecture {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    $stream = $null
    $reader = $null
    try {
        $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::Read)
        $reader = [System.IO.BinaryReader]::new($stream)

        if ($reader.ReadUInt16() -ne 0x5A4D) {
            throw "Service binary is not a PE file: $Path"
        }

        $stream.Position = 0x3C
        $peOffset = $reader.ReadInt32()
        if ($peOffset -lt 0 -or $peOffset -gt ($stream.Length - 6)) {
            throw "Service binary has an invalid PE header offset: $Path"
        }

        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "Service binary has no PE signature: $Path"
        }

        switch ($reader.ReadUInt16()) {
            0x8664 { return 'x64' }
            0xAA64 { return 'arm64' }
            default { throw "Service binary has an unsupported PE machine type: $Path" }
        }
    }
    finally {
        if ($null -ne $reader) {
            $reader.Dispose()
        }
        elseif ($null -ne $stream) {
            $stream.Dispose()
        }
    }
}

function Get-DefaultServiceBinaryCandidates {
    param(
        [Parameter(Mandatory)]
        [string]$RepositoryRoot,

        [Parameter(Mandatory)]
        [string]$TargetArchitecture
    )

    $targetTriple = if ($TargetArchitecture -eq 'x64') {
        'x86_64-pc-windows-msvc'
    }
    else {
        'aarch64-pc-windows-msvc'
    }

    return @(
        (Join-Path $RepositoryRoot "target\$targetTriple\release\vibe-easytier-service.exe"),
        (Join-Path $RepositoryRoot 'target\release\vibe-easytier-service.exe')
    )
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$Destination = [System.IO.Path]::GetFullPath($Destination)
$targetDirectory = Join-Path $Destination "windows-$Architecture"
$targetBinary = Join-Path $targetDirectory 'vibe-easytier-service.exe'

if ([string]::IsNullOrWhiteSpace($ServiceBinary)) {
    $candidates = Get-DefaultServiceBinaryCandidates -RepositoryRoot $repositoryRoot -TargetArchitecture $Architecture
    $ServiceBinary = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1

    if ([string]::IsNullOrWhiteSpace($ServiceBinary)) {
        if ($DryRun) {
            Write-Output 'DRY-RUN: stage the first existing service binary from:'
            $candidates | ForEach-Object { Write-Output "DRY-RUN:   $_" }
            Write-Output "DRY-RUN: to $targetBinary"
            return
        }

        throw "No compiled service binary was found. Build vibe-easytier-service for $Architecture or pass -ServiceBinary."
    }
}

$ServiceBinary = [System.IO.Path]::GetFullPath($ServiceBinary)
if ($DryRun) {
    Write-Output "DRY-RUN: copy $ServiceBinary to $targetBinary"
    if (Test-Path -LiteralPath $targetBinary) {
        Write-Output 'DRY-RUN: existing staged service would be replaced only with -Force.'
    }
    return
}

if (-not (Test-Path -LiteralPath $ServiceBinary -PathType Leaf)) {
    throw "Compiled service binary was not found: $ServiceBinary"
}

$actualArchitecture = Get-PortableExecutableArchitecture -Path $ServiceBinary
if ($actualArchitecture -ne $Architecture) {
    throw "Service binary architecture mismatch. Expected $Architecture, got $actualArchitecture."
}

if ((Test-Path -LiteralPath $targetBinary) -and -not $Force) {
    throw "Staged service already exists: $targetBinary. Re-run with -Force to replace it."
}

New-Item -ItemType Directory -Path $targetDirectory -Force | Out-Null
Copy-Item -LiteralPath $ServiceBinary -Destination $targetBinary -Force
Write-Output "Staged Vibe EasyTier service at $targetBinary."
