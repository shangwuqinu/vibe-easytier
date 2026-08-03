[CmdletBinding()]
param(
    [ValidateSet('x64')]
    [string]$Architecture = 'x64',

    [string]$Destination = (Join-Path (Split-Path -Parent $PSScriptRoot) 'resources\iperf3'),

    [switch]$Force,

    [switch]$KeepArchive,

    [switch]$DryRun,

    [ValidateRange(30, 7200)]
    [int]$DownloadTimeoutSeconds = 900,

    [switch]$SkipReleaseMetadataCheck
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Require-File {
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

function Get-ReleaseAsset {
    param(
        [Parameter(Mandatory)]
        $Manifest,

        [Parameter(Mandatory)]
        $Asset
    )

    $tag = [uri]::EscapeDataString([string]$Manifest.binaryDistribution.tag)
    $repository = [string]$Manifest.binaryDistribution.repository
    $uri = "https://api.github.com/repos/$repository/releases/tags/$tag"
    $headers = @{
        'Accept' = 'application/vnd.github+json'
        'User-Agent' = 'VibeEasyTier-Iperf3Fetcher'
    }
    $release = Invoke-RestMethod -Method Get -Uri $uri -Headers $headers
    if ([string]$release.tag_name -ne [string]$Manifest.binaryDistribution.tag) {
        throw "iperf3 release tag mismatch. Expected $($Manifest.binaryDistribution.tag), got $($release.tag_name)."
    }

    $matches = @($release.assets | Where-Object { $_.name -eq $Asset.assetName })
    if ($matches.Count -ne 1) {
        throw "Expected exactly one iperf3 asset named $($Asset.assetName)."
    }
    $releaseAsset = $matches[0]
    $expectedDigest = "sha256:$([string]$Asset.sha256)".ToLowerInvariant()
    if (([string]$releaseAsset.digest).ToLowerInvariant() -ne $expectedDigest) {
        throw "GitHub release digest mismatch for $($Asset.assetName)."
    }
    if ([int64]$releaseAsset.size -ne [int64]$Asset.size) {
        throw "GitHub release size mismatch for $($Asset.assetName)."
    }
}

function Download-Archive {
    param(
        [Parameter(Mandatory)]
        [string]$Uri,

        [Parameter(Mandatory)]
        [string]$OutputPath,

        [Parameter(Mandatory)]
        [int]$TimeoutSeconds
    )

    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    if ($null -ne $curl) {
        & $curl.Source '--fail' '--location' '--silent' '--show-error' `
            '--connect-timeout' '30' '--max-time' ([string]$TimeoutSeconds) `
            '--output' $OutputPath $Uri
        if ($LASTEXITCODE -ne 0) {
            throw "curl.exe failed to download $Uri with exit code $LASTEXITCODE."
        }
        return
    }

    Invoke-WebRequest -Uri $Uri -OutFile $OutputPath -TimeoutSec $TimeoutSeconds
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repositoryRoot 'resources\iperf3-runtime.manifest.json'
Require-File -Path $manifestPath -Description 'iperf3 runtime manifest'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ([string]$manifest.version -ne '3.21' -or [string]$manifest.binaryDistribution.tag -ne '3.21') {
    throw 'The iperf3 runtime manifest must remain pinned to version 3.21.'
}
$asset = $manifest.assets.$Architecture
if ($null -eq $asset) {
    throw "No $Architecture iperf3 asset exists in $manifestPath."
}
if (-not ([string]$asset.sha256 -match '^[0-9a-fA-F]{64}$')) {
    throw "Invalid iperf3 SHA-256 in the manifest for $Architecture."
}

$Destination = [System.IO.Path]::GetFullPath($Destination)
$targetPath = Join-Path $Destination "windows-$Architecture"
$noticesPath = Join-Path $repositoryRoot 'resources\iperf3\THIRD_PARTY_NOTICES.md'
$licensesPath = Join-Path $repositoryRoot 'resources\iperf3\licenses'
Require-File -Path $noticesPath -Description 'iperf3 third-party notices'
Require-File -Path (Join-Path $licensesPath 'iperf3-LICENSE.txt') -Description 'iperf3 license'
Require-File -Path (Join-Path $licensesPath 'LGPL-3.0.txt') -Description 'LGPL license'

if (-not $SkipReleaseMetadataCheck) {
    Get-ReleaseAsset -Manifest $manifest -Asset $asset
    Write-Output "Verified GitHub metadata for $($asset.assetName)."
}

if ($DryRun) {
    Write-Output "DRY-RUN: download $($asset.url)"
    Write-Output "DRY-RUN: verify SHA-256 $($asset.sha256) and size $($asset.size)"
    Write-Output "DRY-RUN: extract iperf3 runtime to $targetPath"
    return
}
if ((Test-Path -LiteralPath $targetPath) -and -not $Force) {
    throw "iperf3 runtime target already exists: $targetPath. Re-run with -Force to replace it."
}

$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("vibe-iperf3-runtime-" + [guid]::NewGuid().ToString('N'))
$archivePath = Join-Path $temporaryRoot ([string]$asset.assetName)
$extractedPath = Join-Path $temporaryRoot 'extracted'
$stagedRuntimePath = Join-Path $temporaryRoot 'runtime'
$backupPath = Join-Path $temporaryRoot 'previous-runtime'

try {
    New-Item -ItemType Directory -Path $temporaryRoot | Out-Null
    Download-Archive -Uri ([string]$asset.url) -OutputPath $archivePath -TimeoutSeconds $DownloadTimeoutSeconds
    $archiveLength = (Get-Item -LiteralPath $archivePath).Length
    if ([int64]$archiveLength -ne [int64]$asset.size) {
        throw "Downloaded iperf3 archive size mismatch. Expected $($asset.size), got $archiveLength."
    }
    $archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($archiveHash -ne ([string]$asset.sha256).ToLowerInvariant()) {
        throw "Downloaded iperf3 archive SHA-256 mismatch."
    }

    Expand-Archive -LiteralPath $archivePath -DestinationPath $extractedPath -Force
    $executables = @(Get-ChildItem -LiteralPath $extractedPath -Filter 'iperf3.exe' -File -Recurse)
    if ($executables.Count -ne 1) {
        throw "Expected one iperf3.exe in the archive, found $($executables.Count)."
    }
    $payloadDirectory = $executables[0].Directory.FullName
    Require-File -Path (Join-Path $payloadDirectory 'cygwin1.dll') -Description 'Cygwin runtime DLL'

    New-Item -ItemType Directory -Path $stagedRuntimePath | Out-Null
    Copy-Item -LiteralPath (Join-Path $payloadDirectory 'iperf3.exe') -Destination $stagedRuntimePath
    Copy-Item -LiteralPath (Join-Path $payloadDirectory 'cygwin1.dll') -Destination $stagedRuntimePath
    Copy-Item -LiteralPath $noticesPath -Destination $stagedRuntimePath
    Copy-Item -LiteralPath $licensesPath -Destination $stagedRuntimePath -Recurse
    foreach ($requiredFile in @($manifest.requiredFiles)) {
        Require-File -Path (Join-Path $stagedRuntimePath ([string]$requiredFile)) -Description 'Staged iperf3 runtime file'
    }

    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    if (Test-Path -LiteralPath $targetPath) {
        Move-Item -LiteralPath $targetPath -Destination $backupPath
    }
    try {
        Move-Item -LiteralPath $stagedRuntimePath -Destination $targetPath
    }
    catch {
        $originalError = $_
        if (Test-Path -LiteralPath $targetPath) {
            Remove-Item -LiteralPath $targetPath -Recurse -Force
        }
        if (Test-Path -LiteralPath $backupPath) {
            Move-Item -LiteralPath $backupPath -Destination $targetPath
        }
        throw $originalError
    }

    if ($KeepArchive) {
        $archiveDirectory = Join-Path $Destination 'archives'
        New-Item -ItemType Directory -Path $archiveDirectory -Force | Out-Null
        Copy-Item -LiteralPath $archivePath -Destination (Join-Path $archiveDirectory ([string]$asset.assetName)) -Force
    }
    Write-Output "iperf3 $($manifest.version) runtime installed at $targetPath."
}
finally {
    if (Test-Path -LiteralPath $temporaryRoot) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
