package com.vibeeasytier.a14.config;

import com.vibeeasytier.a14.model.Profile;
import com.vibeeasytier.a14.model.ProfileValidator;

import org.tomlj.Toml;
import org.tomlj.TomlArray;
import org.tomlj.TomlParseResult;
import org.tomlj.TomlTable;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;

public final class TomlProfileCodec {
    public static final Set<String> FLAG_KEYS = Set.of(
            "default_protocol", "dev_name", "enable_encryption", "enable_ipv6", "mtu",
            "latency_first", "enable_exit_node", "no_tun", "use_smoltcp",
            "relay_network_whitelist", "disable_p2p", "relay_all_peer_rpc",
            "disable_udp_hole_punching", "multi_thread", "data_compress_algo", "bind_device",
            "enable_kcp_proxy", "disable_kcp_input", "disable_relay_kcp",
            "proxy_forward_by_system", "accept_dns", "private_mode", "enable_quic_proxy",
            "disable_quic_input", "disable_relay_quic", "quic_listen_port",
            "foreign_relay_bps_limit", "multi_thread_count",
            "enable_relay_foreign_network_kcp", "enable_relay_foreign_network_quic",
            "encryption_algorithm", "disable_sym_hole_punching", "tld_dns_zone", "p2p_only",
            "disable_tcp_hole_punching", "lazy_p2p", "need_p2p", "instance_recv_bps_limit",
            "disable_upnp", "disable_relay_data", "enable_udp_broadcast_relay");
    private static final Set<String> ROOT_KEYS = Set.of(
            "instance_name", "hostname", "ipv4", "peer", "network_identity", "flags");

    private TomlProfileCodec() {}

    public static Profile parse(String source, String fallbackHostname) {
        TomlParseResult root = Toml.parse(source);
        if (root.hasErrors()) {
            throw new IllegalArgumentException("TOML 格式错误，请检查文件语法");
        }
        rejectUnknown(root.keySet(), ROOT_KEYS, "根配置");
        String instance = requiredString(root, "instance_name");
        String hostname = root.getString("hostname");
        if (hostname == null || hostname.trim().isEmpty()) {
            hostname = fallbackHostname;
        }
        TomlTable identity = requiredTable(root, "network_identity");
        rejectUnknown(identity.keySet(), Set.of("network_name", "network_secret"), "network_identity");

        List<String> peers = new ArrayList<>();
        TomlArray peerArray = root.getArray("peer");
        if (peerArray != null) {
            for (int index = 0; index < peerArray.size(); index++) {
                TomlTable peer = peerArray.getTable(index);
                rejectUnknown(peer.keySet(), Set.of("uri"), "peer[" + index + "]");
                peers.add(requiredString(peer, "uri"));
            }
        }

        Map<String, Object> flags = new LinkedHashMap<>(Profile.defaultFlags());
        TomlTable importedFlags = root.getTable("flags");
        if (importedFlags != null) {
            rejectUnknown(importedFlags.keySet(), FLAG_KEYS, "flags");
            for (String key : importedFlags.keySet()) {
                Object value = importedFlags.get(key);
                if (!(value instanceof Boolean || value instanceof Long || value instanceof String)) {
                    throw new IllegalArgumentException("flags." + key + " 的类型无效");
                }
                flags.put(key, value);
            }
        }

        Profile profile = new Profile(
                instance,
                instance,
                hostname,
                requiredString(identity, "network_name"),
                requiredString(identity, "network_secret"),
                requiredString(root, "ipv4"),
                peers,
                flags);
        ProfileValidator.validate(profile);
        return profile;
    }

    public static String render(Profile profile) {
        ProfileValidator.validate(profile);
        StringBuilder output = new StringBuilder(2048);
        appendString(output, "instance_name", profile.instanceName());
        appendString(output, "hostname", profile.hostname());
        appendString(output, "ipv4", profile.ipv4Cidr());
        output.append('\n');
        output.append("[network_identity]\n");
        appendString(output, "network_name", profile.networkName());
        appendString(output, "network_secret", profile.networkSecret());
        for (String peer : profile.peers()) {
            output.append("\n[[peer]]\n");
            appendString(output, "uri", peer);
        }
        output.append("\n[flags]\n");
        for (Map.Entry<String, Object> entry : profile.flags().entrySet()) {
            if (!FLAG_KEYS.contains(entry.getKey())) {
                continue;
            }
            Object value = entry.getValue();
            if (value instanceof String text) {
                appendString(output, entry.getKey(), text);
            } else if (value instanceof Boolean || value instanceof Number) {
                output.append(entry.getKey()).append(" = ").append(value).append('\n');
            }
        }
        return output.toString();
    }

    private static void appendString(StringBuilder output, String key, String value) {
        output.append(key).append(" = \"").append(escape(value)).append("\"\n");
    }

    private static String escape(String value) {
        return value.replace("\\", "\\\\").replace("\"", "\\\"")
                .replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t");
    }

    private static String requiredString(TomlTable table, String key) {
        String value = table.getString(key);
        if (value == null) {
            throw new IllegalArgumentException(key + " 必须是字符串");
        }
        return value;
    }

    private static TomlTable requiredTable(TomlTable table, String key) {
        TomlTable value = table.getTable(key);
        if (value == null) {
            throw new IllegalArgumentException("缺少 [" + key + "]");
        }
        return value;
    }

    private static void rejectUnknown(Set<String> actual, Set<String> allowed, String scope) {
        Set<String> unknown = new TreeSet<>(actual);
        unknown.removeAll(allowed);
        if (!unknown.isEmpty()) {
            throw new IllegalArgumentException(scope + " 包含不支持的字段：" + unknown.iterator().next());
        }
    }
}
