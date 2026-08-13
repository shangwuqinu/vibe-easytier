package com.vibeeasytier.a14.model;

import java.util.List;

public final class EasyTierFlagSpec {
    public enum Kind { BOOLEAN, TEXT, NUMBER, SELECT, RATE }

    public record Option(String label, Object value) {}

    public record Field(
            String key,
            Kind kind,
            String label,
            String description,
            List<Option> options,
            Object lockedValue) {
        public boolean locked() { return lockedValue != null; }
    }

    public record Section(String title, String description, List<Field> fields) {}

    private static Field toggle(String key, String label, String description) {
        return new Field(key, Kind.BOOLEAN, label, description, List.of(), null);
    }

    private static Field locked(String key, String label, String description, boolean value) {
        return new Field(key, Kind.BOOLEAN, label, description, List.of(), value);
    }

    private static Field text(String key, String label, String description) {
        return new Field(key, Kind.TEXT, label, description, List.of(), null);
    }

    private static Field number(String key, String label, String description) {
        return new Field(key, Kind.NUMBER, label, description, List.of(), null);
    }

    private static Field rate(String key, String label, String description) {
        return new Field(key, Kind.RATE, label, description, List.of(), null);
    }

    private static Field select(String key, String label, String description, Option... options) {
        return new Field(key, Kind.SELECT, label, description, List.of(options), null);
    }

    public static final List<Section> SECTIONS = List.of(
            new Section("基础与虚拟网卡", "加密、虚拟网卡、线程与数据处理方式。", List.of(
                    select("default_protocol", "默认传输协议", "Bootstrap 未显式写协议时使用。",
                            new Option("TCP", "tcp"), new Option("UDP", "udp"),
                            new Option("WireGuard", "wg"), new Option("QUIC", "quic"),
                            new Option("WebSocket", "ws"), new Option("WebSocket TLS", "wss"),
                            new Option("FakeTCP", "faketcp")),
                    text("dev_name", "虚拟网卡名称", "留空时由 EasyTier 管理名称；Android 使用系统 VPN 接口。"),
                    locked("enable_encryption", "启用加密", "私有网络安全约束，Android 端固定启用。", true),
                    select("encryption_algorithm", "加密算法", "所有节点应使用兼容的加密算法。",
                            new Option("使用 Core 默认", ""), new Option("AES-GCM", "aes-gcm"),
                            new Option("AES-256-GCM", "aes-256-gcm"), new Option("ChaCha20", "chacha20"),
                            new Option("XOR（不建议）", "xor")),
                    toggle("enable_ipv6", "启用 IPv6", "允许 Core 使用 IPv6 底层网络与监听器。"),
                    number("mtu", "MTU", "Android VPN 接口单包大小，范围 576-9000。"),
                    locked("no_tun", "不创建虚拟网卡", "Android 必须通过 VpnService 提供 TUN，固定关闭。", false),
                    toggle("use_smoltcp", "使用 smoltcp", "为代理数据使用 smoltcp 协议栈。"),
                    toggle("multi_thread", "多线程运行", "使用多线程运行时处理 Core 工作负载。"),
                    number("multi_thread_count", "工作线程数", "多线程运行时使用的线程数量，范围 2-128。"),
                    toggle("bind_device", "绑定物理网卡", "将连接套接字绑定到物理网卡，帮助避免路由冲突。"),
                    select("data_compress_algo", "数据压缩", "选择节点间数据压缩方式。",
                            new Option("不压缩", 1L), new Option("Zstandard", 2L))
            )),
            new Section("路由与中继", "转发策略、流量限制和私有网络边界。", List.of(
                    toggle("latency_first", "延迟优先", "优先选择延迟更低的路径。"),
                    locked("enable_exit_node", "允许作为出口节点", "本客户端只提供私有虚拟局域网，固定关闭。", false),
                    toggle("proxy_forward_by_system", "通过系统转发子网代理", "使用系统内核转发子网代理数据包。"),
                    text("relay_network_whitelist", "中继网络白名单", "网络名称以空格分隔，* 表示全部。"),
                    toggle("relay_all_peer_rpc", "转发所有节点 RPC", "不受白名单限制地转发节点控制 RPC。"),
                    locked("private_mode", "私有网络模式", "只允许具有相同网络密钥的节点接入，固定启用。", true),
                    rate("foreign_relay_bps_limit", "外部网络中继上限", "单位 B/s；留空使用 Core 的无限制默认值。"),
                    rate("instance_recv_bps_limit", "实例接收上限", "单位 B/s；留空使用 Core 的无限制默认值。"),
                    toggle("disable_relay_data", "禁止中继数据", "不转发中继数据流，但保留控制连接。")
            )),
            new Section("P2P 与 NAT", "节点直连、打洞和自动端口映射行为。", List.of(
                    toggle("disable_p2p", "禁用自动 P2P", "不主动建立普通 P2P 连接。"),
                    toggle("p2p_only", "仅使用 P2P", "只与已建立 P2P 连接的节点通信。"),
                    toggle("lazy_p2p", "按需建立 P2P", "有实际流量时才尝试建立 P2P。"),
                    toggle("need_p2p", "声明需要 P2P", "通知其他节点主动连接本节点。"),
                    toggle("disable_udp_hole_punching", "禁用 UDP 打洞", "关闭 UDP NAT 穿透。"),
                    toggle("disable_tcp_hole_punching", "禁用 TCP 打洞", "关闭 TCP NAT 穿透。"),
                    toggle("disable_sym_hole_punching", "禁用对称 NAT 打洞", "关闭对称 NAT 的 UDP 打洞。"),
                    toggle("disable_upnp", "禁用 UPnP/NAT-PMP", "关闭自动端口映射。")
            )),
            new Section("KCP 与 QUIC", "TCP 流代理、接入控制和中继策略。", List.of(
                    toggle("enable_kcp_proxy", "启用 KCP 代理", "将 TCP 流通过 KCP 代理。"),
                    toggle("disable_kcp_input", "拒绝 KCP 入站", "不允许其他节点通过 KCP 访问本节点。"),
                    toggle("disable_relay_kcp", "禁止中继 KCP", "不为其他节点转发 KCP 数据包。"),
                    toggle("enable_relay_foreign_network_kcp", "中继外部网络 KCP", "允许转发其他网络的 KCP 数据包。"),
                    toggle("enable_quic_proxy", "启用 QUIC 代理", "将 TCP 流通过 QUIC 代理。"),
                    toggle("disable_quic_input", "拒绝 QUIC 入站", "不允许其他节点通过 QUIC 访问本节点。"),
                    toggle("disable_relay_quic", "禁止中继 QUIC", "不为其他节点转发 QUIC 数据包。"),
                    toggle("enable_relay_foreign_network_quic", "中继外部网络 QUIC", "允许转发其他网络的 QUIC 数据包。"),
                    new Field("quic_listen_port", Kind.NUMBER, "QUIC 监听端口（已废弃）",
                            "仅兼容旧配置，保持 4294967295 自动值。", List.of(), 4294967295L)
            )),
            new Section("DNS 与平台", "Magic DNS 与平台专用行为。", List.of(
                    toggle("accept_dns", "接受 Magic DNS", "允许 EasyTier 提供虚拟网络域名解析。"),
                    text("tld_dns_zone", "Magic DNS 顶级域", "仅在接受 Magic DNS 时生效。"),
                    locked("enable_udp_broadcast_relay", "转发本机 UDP 广播",
                            "此项仅支持 Windows，Android 固定关闭。", false)
            ))
    );

    private EasyTierFlagSpec() {}

    public static int fieldCount() {
        return SECTIONS.stream().mapToInt(section -> section.fields().size()).sum();
    }
}
