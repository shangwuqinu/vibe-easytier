# Windows 与 Android 14 功能对照

本表以当前 Windows 桌面版为基准。`等价` 表示用户目标一致，但使用 Android
平台机制实现；`受限` 表示不能在当前单 APK、非 root 架构下安全复用。

| 能力 | Android 14 状态 | 实现 |
| --- | --- | --- |
| 开机启动并自动连接 | 等价 | BootReceiver + 前台 VpnService；稳定路径为系统 Always-on VPN |
| 断网、睡眠和 Core 故障恢复 | 已对齐 | 网络回调、健康检查、抖动指数退避和无节点受控重启 |
| 多档案与单活动档案 | 已对齐 | 加密档案集合支持新建、切换、重命名和删除；旧单档案自动迁移 |
| 配置保存安全性 | 已对齐 | 先由内置 Core 校验，再以 Keystore AES-GCM 和 AtomicFile 更新 |
| 本地 TOML 导入/导出 | 已对齐 | 系统文件选择器，41 项 `[flags]` 白名单 |
| Core 2.6.4 全部 flags | 已对齐 | 设置页展示 41 项中文名称和说明 |
| 路由、收发流量与节点详情 | 已对齐 | 解析 JNI 运行状态，展示路由、字节、协议、延迟和版本 |
| 日志检索与清空 | 已对齐 | 设备内日志支持实时筛选和清空 |
| 系统/浅色/深色主题 | 已对齐 | 主题偏好持久化 |
| iperf3 节点测速 | 受限 | 未开放，原因见下文 |

Android 必须把运行 EasyTier Core 的应用 UID 排除在自身 VPN 之外，否则 Core
的 Bootstrap 底层连接会再次进入 TUN 形成路由环路。iperf3 子进程继承同一
UID，也会被排除，因而不能通过虚拟地址测速。仅把 Android 可执行文件打进
APK 会产生看似可点、实际绕过隧道的错误结果。要与 Windows 的 iperf3 语义
真正一致，需要独立 UID 的受信任伴随应用，或 EasyTier JNI 增加逐套接字保护
接口后调整 VPN 包含策略；当前版本不做伪测速降级。

另外，Windows Service、DPAPI、命名管道、托盘和 NSIS 属于平台实现，不在
Android 上逐项复制；对应边界分别由 VpnService、Android Keystore、进程内
JNI、系统通知和 APK 安装机制承担。
