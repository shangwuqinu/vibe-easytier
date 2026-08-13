[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$EasyTierSource,

    [string]$AndroidNdk,

    [string]$ProtocPath,

    [string]$ProtocInclude,

    [string]$LibClangPath,

    [switch]$SkipRustTargetInstall
)

$ErrorActionPreference = 'Stop'
$PinnedCommit = '8428a89d2dabc94c97d370ec607c6ca142473626'
$RustToolchain = '1.95.0'
$RustTarget = 'aarch64-linux-android'
$AndroidAbi = 'arm64-v8a'
$ScriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$AndroidRoot = Split-Path -Parent $ScriptRoot
$Destination = Join-Path $AndroidRoot 'app\src\main\jniLibs\arm64-v8a'
$Source = (Resolve-Path -LiteralPath $EasyTierSource).Path

if (-not (Test-Path -LiteralPath (Join-Path $Source '.git'))) {
    throw 'EasyTierSource 必须是 EasyTier 的 Git 工作区。'
}

$Head = (& git -C $Source rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $Head -ne $PinnedCommit) {
    throw "EasyTier 源码必须固定在 v2.6.4 提交 $PinnedCommit，当前为 $Head。"
}
$SourceChanges = & git -C $Source status --porcelain
if ($LASTEXITCODE -ne 0 -or $SourceChanges) {
    throw 'EasyTier 源码工作区不干净；请使用未修改的 v2.6.4 工作区。'
}

if ([string]::IsNullOrWhiteSpace($AndroidNdk)) {
    $SdkRoot = if ($env:ANDROID_SDK_ROOT) { $env:ANDROID_SDK_ROOT } elseif ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
    $NdkRoot = Join-Path $SdkRoot 'ndk'
    $AndroidNdk = Get-ChildItem -LiteralPath $NdkRoot -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending |
        Select-Object -First 1 -ExpandProperty FullName
}
if ([string]::IsNullOrWhiteSpace($AndroidNdk) -or -not (Test-Path -LiteralPath $AndroidNdk)) {
    throw '未找到 Android NDK。请安装 NDK r26，并通过 -AndroidNdk 指定目录。'
}
$env:ANDROID_NDK_HOME = (Resolve-Path -LiteralPath $AndroidNdk).Path
$env:ANDROID_NDK_ROOT = $env:ANDROID_NDK_HOME
$NdkProperties = Join-Path $env:ANDROID_NDK_HOME 'source.properties'
$NdkRevision = if (Test-Path -LiteralPath $NdkProperties) {
    Get-Content -LiteralPath $NdkProperties |
        Where-Object { $_ -match '^Pkg\.Revision\s*=\s*(.+)$' } |
        ForEach-Object { $Matches[1].Trim() } |
        Select-Object -First 1
} else { $null }
if (-not $NdkRevision -or $NdkRevision -notmatch '^26\.') {
    throw "Android 原生层固定使用 NDK r26，当前检测到：$NdkRevision"
}

if ([string]::IsNullOrWhiteSpace($ProtocPath)) {
    if ($env:PROTOC -and (Test-Path -LiteralPath $env:PROTOC)) {
        $ProtocPath = $env:PROTOC
    } else {
        $ProtocCommand = Get-Command protoc -ErrorAction SilentlyContinue
        $ProtocPath = if ($ProtocCommand) { $ProtocCommand.Source } else { $null }
    }
}
if ([string]::IsNullOrWhiteSpace($ProtocPath) -or -not (Test-Path -LiteralPath $ProtocPath)) {
    throw '未找到 protoc。请安装 Protocol Buffers 编译器，并通过 -ProtocPath 指定 protoc.exe。'
}
$env:PROTOC = (Resolve-Path -LiteralPath $ProtocPath).Path
if ([string]::IsNullOrWhiteSpace($ProtocInclude)) {
    if ($env:PROTOC_INCLUDE -and (Test-Path -LiteralPath $env:PROTOC_INCLUDE)) {
        $ProtocInclude = $env:PROTOC_INCLUDE
    } else {
        $SiblingInclude = Join-Path (Split-Path -Parent $env:PROTOC) 'include'
        $ProtocInclude = if (Test-Path -LiteralPath $SiblingInclude) { $SiblingInclude } else { $null }
    }
}
if ([string]::IsNullOrWhiteSpace($ProtocInclude) -or
    -not (Test-Path -LiteralPath (Join-Path $ProtocInclude 'google\protobuf\timestamp.proto'))) {
    throw '未找到 protoc 标准 .proto 文件。请通过 -ProtocInclude 指定包含 google/protobuf 的 include 目录。'
}
$env:PROTOC_INCLUDE = (Resolve-Path -LiteralPath $ProtocInclude).Path

if ([string]::IsNullOrWhiteSpace($LibClangPath)) {
    if ($env:LIBCLANG_PATH -and (Test-Path -LiteralPath (Join-Path $env:LIBCLANG_PATH 'libclang.dll'))) {
        $LibClangPath = $env:LIBCLANG_PATH
    } else {
        $DefaultLibClang = Join-Path $env:ProgramFiles 'LLVM\bin'
        $LibClangPath = if (Test-Path -LiteralPath (Join-Path $DefaultLibClang 'libclang.dll')) {
            $DefaultLibClang
        } else { $null }
    }
}
if ([string]::IsNullOrWhiteSpace($LibClangPath) -or
    -not (Test-Path -LiteralPath (Join-Path $LibClangPath 'libclang.dll'))) {
    throw '未找到 libclang.dll。请安装 LLVM，并通过 -LibClangPath 指定包含 libclang.dll 的目录。'
}
$env:LIBCLANG_PATH = (Resolve-Path -LiteralPath $LibClangPath).Path
$NdkClang = Join-Path $env:ANDROID_NDK_HOME 'toolchains\llvm\prebuilt\windows-x86_64\bin\clang.exe'
if (-not (Test-Path -LiteralPath $NdkClang)) {
    throw "NDK 不完整，未找到 clang.exe：$NdkClang"
}
$env:CLANG_PATH = $NdkClang

& cargo ndk --version *> $null
if ($LASTEXITCODE -ne 0) {
    throw '未安装 cargo-ndk。请先执行 cargo install cargo-ndk --locked。'
}

$InstalledTargets = & rustup target list --installed --toolchain $RustToolchain
if ($InstalledTargets -notcontains $RustTarget) {
    if ($SkipRustTargetInstall) {
        throw "Rust $RustToolchain 缺少目标 $RustTarget。"
    }
    & rustup target add $RustTarget --toolchain $RustToolchain
    if ($LASTEXITCODE -ne 0) {
        throw "为 Rust $RustToolchain 安装目标 $RustTarget 失败。"
    }
}

$FfiPath = Join-Path $Source 'easytier-contrib\easytier-ffi'
$JniPath = Join-Path $Source 'easytier-contrib\easytier-android-jni'
$ReleaseDir = Join-Path $Source "target\$RustTarget\release"
foreach ($Path in @($FfiPath, $JniPath)) {
    if (-not (Test-Path -LiteralPath (Join-Path $Path 'Cargo.toml'))) {
        throw "EasyTier v2.6.4 Android 源码不完整：$Path"
    }
}

Push-Location $FfiPath
try {
    & cargo ndk -t $AndroidAbi build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'easytier-ffi Android 构建失败。' }
}
finally {
    Pop-Location
}

Push-Location $JniPath
$PreviousTargetRustFlags = $env:CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS
try {
    $FfiLinkFlags = "-Lnative=$($ReleaseDir.Replace('\', '/')) -ldylib=easytier_ffi"
    $env:CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS = if ($PreviousTargetRustFlags) {
        "$PreviousTargetRustFlags $FfiLinkFlags"
    } else { $FfiLinkFlags }
    & cargo ndk -t $AndroidAbi build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'easytier-android-jni 构建失败。' }
}
finally {
    $env:CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS = $PreviousTargetRustFlags
    Pop-Location
}

$Libraries = @('libeasytier_ffi.so', 'libeasytier_android_jni.so')
New-Item -ItemType Directory -Force -Path $Destination | Out-Null
foreach ($Library in $Libraries) {
    $SourceLibrary = Join-Path $ReleaseDir $Library
    if (-not (Test-Path -LiteralPath $SourceLibrary)) {
        throw "构建完成但未找到 $SourceLibrary。"
    }
    Copy-Item -LiteralPath $SourceLibrary -Destination (Join-Path $Destination $Library) -Force
}

$Libraries | ForEach-Object {
    Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $Destination $_)
} | Format-Table -AutoSize

Write-Host "EasyTier v2.6.4 Android 运行库已暂存到 $Destination"
