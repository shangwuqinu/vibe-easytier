package com.vibeeasytier.a14.model;

import java.net.URI;
import java.net.URISyntaxException;
import java.util.HashSet;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.regex.Pattern;

import com.vibeeasytier.a14.config.TomlProfileCodec;

public final class ProfileValidator {
    private static final Pattern IDENTIFIER = Pattern.compile("[A-Za-z0-9_-]{1,64}");
    private static final Set<String> PEER_SCHEMES = Set.of("tcp", "udp", "wg", "ws", "wss");
    private static final Set<String> STRING_FLAGS = Set.of(
            "default_protocol", "dev_name", "relay_network_whitelist", "encryption_algorithm", "tld_dns_zone");
    private static final Set<String> INTEGER_FLAGS = Set.of(
            "mtu", "data_compress_algo", "quic_listen_port", "foreign_relay_bps_limit",
            "multi_thread_count", "instance_recv_bps_limit");

    private ProfileValidator() {}

    public static void validate(Profile profile) {
        if (!IDENTIFIER.matcher(profile.instanceName()).matches()) {
            fail("实例名称仅允许 1-64 个字母、数字、连字符或下划线");
        }
        requireText(profile.profileName(), 96, "档案名称");
        requireText(profile.hostname(), 63, "设备名称");
        requireText(profile.networkName(), 128, "网络名称");
        requireText(profile.networkSecret(), 512, "网络密钥");
        validateCidr(profile.ipv4Cidr());
        if (profile.peers().isEmpty()) {
            fail("至少添加一个 Bootstrap 节点");
        }
        if (profile.peers().size() > 8) {
            fail("Bootstrap 节点最多 8 个");
        }
        Set<String> unique = new HashSet<>();
        for (String peer : profile.peers()) {
            validatePeer(peer);
            if (!unique.add(peer)) {
                fail("Bootstrap 节点不能重复");
            }
        }
        if (!Boolean.TRUE.equals(profile.flags().get("enable_encryption"))) {
            fail("Android 首版要求启用传输加密");
        }
        if (!Boolean.TRUE.equals(profile.flags().get("private_mode"))) {
            fail("Android 首版要求启用私有网络模式");
        }
        if (Boolean.TRUE.equals(profile.flags().get("no_tun"))) {
            fail("Android VPN 模式不能启用 no_tun");
        }
        if (Boolean.TRUE.equals(profile.flags().get("enable_exit_node"))) {
            fail("Android 首版不支持出口节点");
        }
        if (Boolean.TRUE.equals(profile.flags().get("enable_udp_broadcast_relay"))) {
            fail("Android 不支持 Windows UDP 广播转发");
        }
        validateFlags(profile.flags());
    }

    private static void validateFlags(Map<String, Object> flags) {
        for (Map.Entry<String, Object> entry : flags.entrySet()) {
            if (!TomlProfileCodec.FLAG_KEYS.contains(entry.getKey())) {
                fail("不支持 flags." + entry.getKey());
            }
            Object value = entry.getValue();
            if (STRING_FLAGS.contains(entry.getKey())) {
                if (!(value instanceof String)) {
                    fail("flags." + entry.getKey() + " 必须是字符串");
                }
            } else if (INTEGER_FLAGS.contains(entry.getKey())) {
                if (!(value instanceof Number)) {
                    fail("flags." + entry.getKey() + " 必须是整数");
                }
                long number = ((Number) value).longValue();
                if (number < 0) {
                    fail("flags." + entry.getKey() + " 不能为负数");
                }
            } else if (!(value instanceof Boolean)) {
                fail("flags." + entry.getKey() + " 必须是布尔值");
            }
        }
        long mtu = numberFlag(flags, "mtu");
        if (mtu < 576 || mtu > 9000) {
            fail("flags.mtu 必须在 576-9000 之间");
        }
        long threads = numberFlag(flags, "multi_thread_count");
        if (threads < 2 || threads > 128) {
            fail("flags.multi_thread_count 必须在 2-128 之间");
        }
        long compression = numberFlag(flags, "data_compress_algo");
        if (compression != 1 && compression != 2) {
            fail("flags.data_compress_algo 仅支持 1 或 2");
        }
        long quicPort = numberFlag(flags, "quic_listen_port");
        if (quicPort < 0 || quicPort > 4294967295L) {
            fail("flags.quic_listen_port 必须是 32 位无符号整数");
        }
        for (String rate : Set.of("foreign_relay_bps_limit", "instance_recv_bps_limit")) {
            Object value = flags.get(rate);
            if (value != null && (!(value instanceof Number) || ((Number) value).longValue() < 0)) {
                fail("flags." + rate + " 必须是非负整数");
            }
        }
        String algorithm = (String) flags.get("encryption_algorithm");
        if (!Set.of("", "xor", "aes-gcm", "aes-256-gcm", "chacha20").contains(algorithm)) {
            fail("flags.encryption_algorithm 的值不受支持");
        }
    }

    private static long numberFlag(Map<String, Object> flags, String key) {
        Object value = flags.get(key);
        if (!(value instanceof Number)) {
            fail("flags." + key + " 必须是整数");
        }
        return ((Number) value).longValue();
    }

    private static void requireText(String value, int max, String label) {
        if (value == null || value.trim().isEmpty()) {
            fail(label + "不能为空");
        }
        if (value.length() > max) {
            fail(label + "不能超过 " + max + " 个字符");
        }
        if (value.chars().anyMatch(Character::isISOControl)) {
            fail(label + "不能包含控制字符");
        }
    }

    private static void validateCidr(String cidr) {
        String[] parts = cidr.split("/", -1);
        if (parts.length != 2) {
            fail("虚拟 IPv4 必须使用 地址/前缀 格式");
        }
        String[] octets = parts[0].split("\\.", -1);
        if (octets.length != 4) {
            fail("虚拟 IPv4 地址无效");
        }
        for (String octet : octets) {
            try {
                int value = Integer.parseInt(octet);
                if (value < 0 || value > 255 || !Integer.toString(value).equals(octet)) {
                    fail("虚拟 IPv4 地址无效");
                }
            } catch (NumberFormatException error) {
                fail("虚拟 IPv4 地址无效");
            }
        }
        try {
            int prefix = Integer.parseInt(parts[1]);
            if (prefix < 0 || prefix > 32) {
                fail("虚拟 IPv4 前缀必须在 0-32 之间");
            }
        } catch (NumberFormatException error) {
            fail("虚拟 IPv4 前缀无效");
        }
    }

    private static void validatePeer(String value) {
        try {
            URI uri = new URI(value);
            String scheme = uri.getScheme() == null ? "" : uri.getScheme().toLowerCase(Locale.ROOT);
            if (!PEER_SCHEMES.contains(scheme)) {
                fail("Bootstrap 仅支持 tcp、udp、wg、ws 和 wss 协议");
            }
            if (uri.getHost() == null || uri.getPort() < 1 || uri.getPort() > 65535) {
                fail("Bootstrap 必须包含有效的主机和端口");
            }
            if (uri.getUserInfo() != null || uri.getQuery() != null || uri.getFragment() != null) {
                fail("Bootstrap 不能包含凭据、查询参数或片段");
            }
        } catch (URISyntaxException error) {
            fail("Bootstrap 地址格式无效");
        }
    }

    private static void fail(String message) {
        throw new IllegalArgumentException(message);
    }
}
