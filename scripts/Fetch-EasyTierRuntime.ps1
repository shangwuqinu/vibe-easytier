[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64')]
    [string]$Architecture = 'x64',

    [string]$Destination = (Join-Path (Split-Path -Parent $PSScriptRoot) 'resources\easytier'),

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

    $tag = [uri]::EscapeDataString([string]$Manifest.upstream.tag)
    $uri = "https://api.github.com/repos/$($Manifest.upstream.repository)/releases/tags/$tag"
    $headers = @{
        'Accept' = 'application/vnd.github+json'
        'User-Agent' = 'VibeEasyTier-RuntimeFetcher'
    }

    $release = Invoke-RestMethod -Method Get -Uri $uri -Headers $headers
    if ([string]$release.tag_name -ne [string]$Manifest.upstream.tag) {
        throw "Release tag mismatch. Expected $($Manifest.upstream.tag), got $($release.tag_name)."
    }

    $matches = @($release.assets | Where-Object { $_.name -eq $Asset.assetName })
    if ($matches.Count -ne 1) {
        throw "Expected exactly one asset named $($Asset.assetName) in release $($Manifest.upstream.tag)."
    }

    $releaseAsset = $matches[0]
    $expectedDigest = "sha256:$([string]$Asset.sha256)".ToLowerInvariant()
    $actualDigest = ([string]$releaseAsset.digest).ToLowerInvariant()
    if ($actualDigest -ne $expectedDigest) {
        throw "GitHub release digest mismatch for $($Asset.assetName). Expected $expectedDigest, got $actualDigest."
    }

    if ([int64]$releaseAsset.size -ne [int64]$Asset.size) {
        throw "GitHub release size mismatch for $($Asset.assetName). Expected $($Asset.size), got $($releaseAsset.size)."
    }
}

function Get-PayloadDirectory {
    param(
        [Parameter(Mandatory)]
        [string]$ExtractedPath,

        [Parameter(Mandatory)]
        $Manifest
    )

    $coreFiles = @(Get-ChildItem -LiteralPath $ExtractedPath -Filter 'easytier-core.exe' -File -Recurse)
    if ($coreFiles.Count -ne 1) {
        throw "Expected one easytier-core.exe in the extracted archive, found $($coreFiles.Count)."
    }

    $payloadDirectory = $coreFiles[0].Directory.FullName
    foreach ($requiredFile in @($Manifest.requiredFiles)) {
        $candidate = Join-Path $payloadDirectory ([string]$requiredFile)
        Require-File -Path $candidate -Description "Required EasyTier runtime file"
    }

    return $payloadDirectory
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
        $curlArguments = @(
            '--fail',
            '--location',
            '--silent',
            '--show-error',
            '--connect-timeout',
            '30',
            '--max-time',
            ([string]$TimeoutSeconds),
            '--output',
            $OutputPath,
            $Uri
        )
        & $curl.Source @curlArguments
        if ($LASTEXITCODE -ne 0) {
            throw "curl.exe failed to download $Uri with exit code $LASTEXITCODE."
        }
        return
    }

    Invoke-WebRequest -Uri $Uri -OutFile $OutputPath -TimeoutSec $TimeoutSeconds
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repositoryRoot 'resources\easytier-runtime.manifest.json'
Require-File -Path $manifestPath -Description 'EasyTier runtime manifest'

$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ([string]$manifest.version -ne '2.6.4' -or [string]$manifest.upstream.tag -ne 'v2.6.4') {
    throw 'The runtime manifest must remain pinned to EasyTier v2.6.4.'
}

$asset = $manifest.assets.$Architecture
if ($null -eq $asset) {
    throw "No $Architecture asset exists in $manifestPath."
}

if (-not ([string]$asset.sha256 -match '^[0-9a-fA-F]{64}$')) {
    throw "Invalid SHA-256 in the manifest for $Architecture."
}

$Destination = [System.IO.Path]::GetFullPath($Destination)
$targetPath = Join-Path $Destination "windows-$Architecture"

if (-not $SkipReleaseMetadataCheck) {
    Get-ReleaseAsset -Manifest $manifest -Asset $asset
    Write-Output "Verified GitHub metadata for $($asset.assetName)."
}

if ($DryRun) {
    Write-Output "DRY-RUN: download $($asset.url)"
    Write-Output "DRY-RUN: verify SHA-256 $($asset.sha256) and size $($asset.size)"
    Write-Output "DRY-RUN: extract runtime to $targetPath"
    if (Test-Path -LiteralPath $targetPath) {
        Write-Output "DRY-RUN: existing target would be replaced only with -Force: $targetPath"
    }
    return
}

if ((Test-Path -LiteralPath $targetPath) -and -not $Force) {
    throw "Runtime target already exists: $targetPath. Re-run with -Force to replace it."
}

$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("vibe-easytier-runtime-" + [guid]::NewGuid().ToString('N'))
$archivePath = Join-Path $temporaryRoot ([string]$asset.assetName)
$extractedPath = Join-Path $temporaryRoot 'extracted'
$stagedRuntimePath = Join-Path $temporaryRoot 'runtime'
$backupPath = Join-Path $temporaryRoot 'previous-runtime'

try {
    New-Item -ItemType Directory -Path $temporaryRoot | Out-Null
    Download-Archive -Uri ([string]$asset.url) -OutputPath $archivePath -TimeoutSeconds $DownloadTimeoutSeconds

    $archiveLength = (Get-Item -LiteralPath $archivePath).Length
    if ([int64]$archiveLength -ne [int64]$asset.size) {
        throw "Downloaded archive size mismatch. Expected $($asset.size), got $archiveLength."
    }

    $archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($archiveHash -ne ([string]$asset.sha256).ToLowerInvariant()) {
        throw "Downloaded archive SHA-256 mismatch. Expected $($asset.sha256), got $archiveHash."
    }

    Expand-Archive -LiteralPath $archivePath -DestinationPath $extractedPath -Force
    $payloadDirectory = Get-PayloadDirectory -ExtractedPath $extractedPath -Manifest $manifest

    New-Item -ItemType Directory -Path $stagedRuntimePath | Out-Null
    foreach ($item in Get-ChildItem -LiteralPath $payloadDirectory -Force) {
        Copy-Item -LiteralPath $item.FullName -Destination $stagedRuntimePath -Recurse -Force
    }

    foreach ($requiredFile in @($manifest.requiredFiles)) {
        Require-File -Path (Join-Path $stagedRuntimePath ([string]$requiredFile)) -Description 'Staged EasyTier runtime file'
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
        try {
            if (Test-Path -LiteralPath $targetPath) {
                Remove-Item -LiteralPath $targetPath -Recurse -Force
            }
            if (Test-Path -LiteralPath $backupPath) {
                Move-Item -LiteralPath $backupPath -Destination $targetPath
            }
        }
        catch {
            Write-Warning "Failed to restore the previous EasyTier runtime. $($_.Exception.Message)"
        }
        throw $originalError
    }

    if ($KeepArchive) {
        $archiveDirectory = Join-Path $Destination 'archives'
        New-Item -ItemType Directory -Path $archiveDirectory -Force | Out-Null
        Copy-Item -LiteralPath $archivePath -Destination (Join-Path $archiveDirectory ([string]$asset.assetName)) -Force
    }

    Write-Output "EasyTier $($manifest.version) runtime installed at $targetPath."
}
finally {
    if (Test-Path -LiteralPath $temporaryRoot) {
        try {
            Remove-Item -LiteralPath $temporaryRoot -Recurse -Force -ErrorAction Stop
        }
        catch {
            Write-Warning "Could not remove temporary runtime directory $temporaryRoot. $($_.Exception.Message)"
        }
    }
}
