# Vibe EasyTier Android 14

`a14` 是独立的 Android 14（API 34）arm64 客户端。它复用桌面版的私有
虚拟局域网边界，但不移植 Windows Service、DPAPI、命名管道或外部
`easytier-core.exe`。Android 端通过 `VpnService` 承载 TUN，并在应用进程内
调用固定为 EasyTier v2.6.4 的官方 JNI/FFI 组件。

## 已实现

- 概览、私网、节点、日志、设置五个可滚动页面，支持系统、浅色和深色主题。
- 多档案新建、切换、重命名和删除；只允许一个活动档案自动连接，旧版单档案
  会在首次读取时迁移到加密档案集合。
- 固定虚拟 IPv4/CIDR、加密私网、最多 8 个 Bootstrap 节点，以及 TCP、
  UDP、WireGuard、WS、WSS 多协议地址。
- Android Keystore AES-GCM 加密档案、原子写入、本地 TOML 导入/导出；保存前
  先交给内置 Core 校验，错误配置不会覆盖当前可用档案。
- 设置页展示 EasyTier Core 2.6.4 的全部 41 项 `[flags]`，使用中文名称和说明；
  加密、私有模式、TUN、出口节点和 Windows 广播项按 Android 安全边界锁定。
- 概览和节点页展示路由、收发字节、虚拟地址、活动协议、延迟和 Core 版本；
  日志页支持搜索和清空。
- 前台 `VpnService`、持久通知、开机广播、Always-on VPN 声明、网络恢复和
  最长 5 分钟的抖动指数退避；手动断开同时清除自动连接意图。
- Core 存活但无节点与 Core 停止分别展示；连续 10 分钟无节点时受控重启，
  最多每 15 分钟一次。

## 构建

环境要求：JDK 17、Android SDK Platform/Build Tools 34、NDK r26、Rust、
`cargo-ndk`、`protoc`（含标准 `.proto` 文件）和 LLVM `libclang.dll`。先准备
固定源码并构建官方 JNI 库；工具不在 `PATH` 时可分别传入 `-AndroidNdk`、
`-ProtocPath`、`-ProtocInclude` 与 `-LibClangPath`：

```powershell
git clone --branch v2.6.4 --depth 1 https://github.com/EasyTier/EasyTier.git C:\src\EasyTier-2.6.4
pwsh -NoProfile -File .\a14\scripts\Build-EasyTierNative.ps1 -EasyTierSource C:\src\EasyTier-2.6.4
```

随后构建和测试 APK：

```powershell
$env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk"
.\a14\gradlew.bat -p .\a14 testDebugUnitTest assembleDebug
```

APK 位于 `a14/app/build/outputs/apk/debug/app-debug.apk`。Debug 与 Release 构建
都会先检查两个 `.so`，缺少固定 Core 时直接停止，避免生成只能预览界面、
无法连接的 APK。原生库属于生成物且不会提交。

## 真机验收

1. 安装 arm64 APK，导入或创建档案并同意系统 VPN 授权。
2. 验证固定虚拟 IP 与另一 EasyTier v2.6.4 节点互通。
3. 在系统 VPN 设置中启用 Always-on VPN，重启且不打开应用，确认自动加入。
4. 验证断网恢复、切换 Wi-Fi/移动网络、Doze、强杀进程和升级覆盖安装。
5. 在至少两种 Android 14 厂商 ROM 上关闭与开启电池限制各跑一轮。

详细边界见 [FEASIBILITY.md](FEASIBILITY.md)。
与 Windows 当前功能的逐项校验见 [FEATURE_PARITY.md](FEATURE_PARITY.md)。
第三方运行库的发布义务见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
