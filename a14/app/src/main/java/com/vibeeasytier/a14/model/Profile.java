package com.vibeeasytier.a14.model;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public final class Profile {
    private final String profileName;
    private final String instanceName;
    private final String hostname;
    private final String networkName;
    private final String networkSecret;
    private final String ipv4Cidr;
    private final List<String> peers;
    private final Map<String, Object> flags;

    public Profile(
            String profileName,
            String instanceName,
            String hostname,
            String networkName,
            String networkSecret,
            String ipv4Cidr,
            List<String> peers,
            Map<String, Object> flags) {
        this.profileName = profileName == null ? "" : profileName.trim();
        this.instanceName = instanceName == null ? "" : instanceName.trim();
        this.hostname = hostname == null ? "" : hostname.trim();
        this.networkName = networkName == null ? "" : networkName.trim();
        this.networkSecret = networkSecret == null ? "" : networkSecret;
        this.ipv4Cidr = ipv4Cidr == null ? "" : ipv4Cidr.trim();
        this.peers = Collections.unmodifiableList(new ArrayList<>(peers));
        this.flags = Collections.unmodifiableMap(new LinkedHashMap<>(flags));
    }

    public static Profile empty(String defaultHostname) {
        return new Profile(
                "Android 私网",
                "vibe-a14",
                defaultHostname,
                "",
                "",
                "100.76.1.2/24",
                List.of(),
                defaultFlags());
    }

    public static Map<String, Object> defaultFlags() {
        Map<String, Object> values = new LinkedHashMap<>();
        values.put("default_protocol", "tcp");
        values.put("dev_name", "");
        values.put("enable_encryption", true);
        values.put("enable_ipv6", true);
        values.put("mtu", 1380L);
        values.put("latency_first", false);
        values.put("enable_exit_node", false);
        values.put("no_tun", false);
        values.put("use_smoltcp", false);
        values.put("relay_network_whitelist", "*");
        values.put("disable_p2p", false);
        values.put("relay_all_peer_rpc", false);
        values.put("disable_udp_hole_punching", false);
        values.put("multi_thread", true);
        values.put("data_compress_algo", 1L);
        values.put("bind_device", true);
        values.put("enable_kcp_proxy", false);
        values.put("disable_kcp_input", false);
        values.put("disable_relay_kcp", false);
        values.put("proxy_forward_by_system", false);
        values.put("accept_dns", false);
        values.put("private_mode", true);
        values.put("enable_quic_proxy", false);
        values.put("disable_quic_input", false);
        values.put("disable_relay_quic", false);
        values.put("quic_listen_port", 4294967295L);
        values.put("multi_thread_count", 2L);
        values.put("enable_relay_foreign_network_kcp", false);
        values.put("enable_relay_foreign_network_quic", false);
        values.put("encryption_algorithm", "aes-gcm");
        values.put("disable_sym_hole_punching", false);
        values.put("tld_dns_zone", "et.net.");
        values.put("p2p_only", false);
        values.put("disable_tcp_hole_punching", false);
        values.put("lazy_p2p", false);
        values.put("need_p2p", false);
        values.put("disable_upnp", false);
        values.put("disable_relay_data", false);
        values.put("enable_udp_broadcast_relay", false);
        return values;
    }

    public String profileName() { return profileName; }
    public String instanceName() { return instanceName; }
    public String hostname() { return hostname; }
    public String networkName() { return networkName; }
    public String networkSecret() { return networkSecret; }
    public String ipv4Cidr() { return ipv4Cidr; }
    public List<String> peers() { return peers; }
    public Map<String, Object> flags() { return flags; }
}
