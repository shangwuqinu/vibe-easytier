# Android 14 可行性评估

## 结论

技术方案可行，建议首版限定 `arm64-v8a + API 34 + EasyTier v2.6.4`。EasyTier
官方仓库已支持 Android，并提供 Android JNI、TUN 文件描述符注入及移动端
构建路径，因此无需在 Java 层重新实现协议栈。移动端不能照搬 Windows 的
LocalSystem 服务模型：正确边界是应用内 Core 加 Android `VpnService`。

## 能力矩阵

| 能力 | 可行性 | 当前实现与边界 |
| --- | --- | --- |
| 固定虚拟 IPv4、加密私网 | 高 | 已实现，Core v2.6.4 原生层负责数据面 |
| TCP/UDP/WireGuard 多 Bootstrap | 高 | 已实现 URI 白名单与并行配置 |
| 后台自动恢复 | 中高 | 前台 VPN 服务、网络回调、退避与健康检查已实现 |
| 开机自动连接 | 中高 | BootReceiver 已实现；Always-on VPN 才是系统级稳定路径 |
| 完全无人值守首次连接 | 不可行 | Android 首次必须由用户确认 VPN 授权 |
| 厂商 ROM 全面保活 | 中 | 仍受电池策略和厂商后台管理影响，必须真机验收 |
| 桌面 41 项 flags 完整可视编辑 | 中 | TOML 可导入全部白名单；移动界面仅展示稳定常用项 |
| iperf3 节点测速 | 中 | 暂未打包 Android 原生 iperf3，不能复用 Windows EXE |

## 稳定性判断

Android 的 Always-on VPN 能在开机后由系统启动并维持 VPN 服务，也是满足
“开机自启”的首选验收模式。普通 BootReceiver 是补充路径，不应单独作为
稳定性承诺。自动连接意图、加密档案和重试状态均持久化；一次连接失败不会
删除档案。手动断开则关闭自动连接，避免后台违背用户操作。

Android 14 要求前台服务声明具体类型及对应权限。本实现使用 `specialUse`
并说明 VPN 隧道用途，同时保持不可取消的连接通知。发布到 Google Play 前
仍需通过该用途的政策审核；企业侧载不受商店审核流程影响。

## 尚需完成的发布门槛

- 用 NDK r26 构建并暂存固定提交的两个 `.so`，记录供应链哈希和 LGPL 通知。
- 两节点真实网络测试，以及重启、断网、Doze、进程回收、系统撤权测试。
- 补齐 Android 原生 iperf3 后再开放带宽测试，避免伪造或降级测速结果。
- 若要求所有设备都无需人工设置 Always-on VPN，需要 Device Owner/MDM 部署；
  普通消费级 APK 无权静默完成该系统设置。

## 参考

- [EasyTier v2.6.4 Android JNI](https://github.com/EasyTier/EasyTier/tree/v2.6.4/easytier-contrib/easytier-android-jni)
- [EasyTier v2.6.4 GUI 移动端工程](https://github.com/EasyTier/EasyTier/tree/v2.6.4/easytier-gui)
- [Android VPN 开发指南](https://developer.android.com/develop/connectivity/vpn)
- [Android 14 前台服务类型要求](https://developer.android.com/about/versions/14/changes/fgs-types-required)

