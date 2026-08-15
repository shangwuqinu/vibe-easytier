# Vibe EasyTier

[![Build and Release](https://github.com/shangwuqinu/vibe-easytier/actions/workflows/verify-windows.yml/badge.svg?branch=master)](https://github.com/shangwuqinu/vibe-easytier/actions/workflows/verify-windows.yml)
[![Latest Release](https://img.shields.io/github/v/release/shangwuqinu/vibe-easytier?label=%E4%B8%8B%E8%BD%BD%E6%9C%80%E6%96%B0%E7%89%88)](https://github.com/shangwuqinu/vibe-easytier/releases/latest)

Vibe EasyTier 是一个面向私有虚拟局域网的 EasyTier 客户端，提供 Windows x64
桌面版和 Android 14 arm64 版。产品专注于稳定完成两件事：**设备启动后自动运行**，
以及**持续连接指定的私有网络**。

内置运行时固定为 EasyTier Core 2.6.4。客户端不尝试覆盖 EasyTier 的所有场景，
而是把档案管理、连接恢复、节点观测和安全存储组合成一套日常可用的运维界面。

## 核心功能

### 开机启动与自动连接

- Windows 安装程序注册延迟自动启动的 `VibeEasyTierService`，无需等待用户登录即可
  启动 Core 并连接活动私网。
- 自动连接保存的是长期用户意图。服务启动、网络恢复和睡眠唤醒都会重新尝试连接，
  失败后使用带抖动的指数退避，最长等待 5 分钟。
- Core 异常退出或 RPC 无响应时会受控重启；Core 正常但长期无远端节点时采用限频恢复，
  避免频繁重启影响网络。
- 手动断开会同时关闭自动连接，后台不会擅自重新拉起连接。
- 关闭 Windows 窗口后应用进入托盘，后台服务继续维持私网。

### 私有网络档案

- 管理多个私网档案，并指定唯一的活动档案用于自动连接。
- 配置档案名称、设备名称、网络名称、网络密钥和固定虚拟 IPv4/CIDR。
- Windows 设备名称留空时自动使用本机计算机名称。
- 每个档案最多添加 8 个 Bootstrap 地址，支持 `tcp://`、`udp://`、`wg://`、
  `ws://` 和 `wss://`。
- 同一 Bootstrap 节点可以配置多个协议地址，Core 会尝试并维护可用传输；节点页
  会同时展示实际建立连接的一个或多个协议。
- 从本地 TOML 文件导入或导出档案。导入只接受允许的私网字段，并在替换现有配置前
  交给内置 Core 校验；错误配置不会破坏当前可用连接。

> 导出的 TOML 包含私网密钥，请将文件存放在可信位置。

### 状态、节点与日志

| 页面 | 主要功能 |
| --- | --- |
| 概览 | 查看开机服务健康度、自动连接状态、活动档案、路由数、收发流量、节点数、最近成功时间和重试时间 |
| 私有网络 | 新建、编辑、选择、删除、导入和导出档案，并执行连接或断开 |
| 节点 | 查看节点名称、虚拟地址、状态、角色、活动协议、延迟和 Core 版本；Windows 支持 iperf3 上传/下载测速 |
| 日志 | 查看带级别标记的完整日志并进行搜索，保留 Core 多行日志的整体结构，并支持清空 |
| 设置 | 切换系统、浅色或深色主题，控制自动连接，并配置 EasyTier 2.6.4 的全部 41 项 `[flags]` |

### 配置与数据安全

- Windows 使用 DPAPI 加密档案和连接意图，活动 TOML 仅写入受 ACL 保护的服务目录。
- 桌面端通过受 ACL 保护的本地命名管道管理服务，不直接访问 Core 管理端口；Core RPC
  仅监听 `127.0.0.1`。
- 档案导出由原生层完成，包含密钥的完整 TOML 不进入 WebView 状态。
- Android 使用 Android Keystore AES-GCM 加密档案，并通过 `VpnService` 承载 TUN。

## 平台支持

| 能力 | Windows x64 | Android 14 arm64 |
| --- | --- | --- |
| 开机自动连接 | Windows Service，可在登录前运行 | `VpnService` + 开机广播；建议在系统中启用“始终开启的 VPN” |
| 多档案与单活动档案 | 支持 | 支持 |
| TOML 导入/导出 | 支持 | 支持 |
| 路由、流量和节点协议 | 支持 | 支持 |
| 41 项 Core flags | 支持，中文说明 | 支持，Android 不适用项会锁定 |
| 节点间 iperf3 测速 | 支持，内置 iperf3 3.21 | 暂不支持，避免测速流量绕过 VPN |

Android 厂商对后台进程和开机广播的限制并不一致。要获得最稳定的无人值守连接，
请授予 VPN 权限、启用系统“始终开启的 VPN”，并按设备需要关闭电池优化。

## 快速使用

1. 从 [Releases](https://github.com/shangwuqinu/vibe-easytier/releases/latest)
   下载 Windows x64 安装程序或 Android 14 arm64 APK。
2. 创建私网档案，填写网络名称、网络密钥、固定虚拟 IP 和至少一个 Bootstrap 地址。
3. 保存档案。配置会先由内置 Core 校验，失败原因会直接显示在界面中。
4. 选择活动档案，开启“自动连接”，然后连接私网。
5. 在“概览”和“节点”中确认已出现远端节点、活动协议、路由和流量。

建议为生产私网部署至少两台长期在线的 Bootstrap/中继节点，并为每个客户端分配
不冲突的固定虚拟 IP。

## 功能边界

Vibe EasyTier 聚焦加密私有虚拟局域网，不提供 WireGuard Portal、子网代理、
端口转发、SOCKS、DNS Portal 和配置服务器等独立管理入口。

`wg://host:port` 是受支持的 EasyTier 节点传输协议，不等同于 WireGuard
`vpn_portal` 服务端功能。

## 下载与文档

- [下载最新版本](https://github.com/shangwuqinu/vibe-easytier/releases/latest)
- [Android 14 使用与构建说明](a14/README.md)
- [Windows 与 Android 功能对照](a14/FEATURE_PARITY.md)
- [Android 14 可行性与平台限制](a14/FEASIBILITY.md)
- [贡献者指南](AGENTS.md)

推送到 `master` 或 `main` 后，GitHub Actions 会构建并验证 Windows 安装程序和
Android APK；两个平台均成功后，工作流自动创建 tag 和 GitHub Release。
