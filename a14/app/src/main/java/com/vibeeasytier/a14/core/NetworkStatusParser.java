package com.vibeeasytier.a14.core;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

public final class NetworkStatusParser {
    private NetworkStatusParser() {}

    public static NetworkSnapshot parse(String json, String instanceName) {
        if (json == null || json.isBlank()) {
            return empty("尚未获取到 Core 状态");
        }
        try {
            JSONObject map = new JSONObject(json).optJSONObject("map");
            JSONObject instance = map == null ? null : map.optJSONObject(instanceName);
            if (instance == null) {
                return empty("Core 尚未报告网络实例");
            }
            boolean running = instance.optBoolean("running", false);
            String error = instance.optString("error_msg", "");
            JSONArray routes = firstArray(instance, "routes", "route_list");
            JSONArray pairs = instance.optJSONArray("peer_route_pairs");
            JSONArray rawPeers = instance.optJSONArray("peers");

            Map<String, JSONObject> peerById = indexPeers(rawPeers);
            List<PeerSnapshot> peers = new ArrayList<>();
            Set<String> seen = new LinkedHashSet<>();
            long sent = 0;
            long received = 0;

            if (pairs != null) {
                for (int index = 0; index < pairs.length(); index++) {
                    JSONObject pair = pairs.optJSONObject(index);
                    if (pair == null) {
                        continue;
                    }
                    JSONObject route = pair.optJSONObject("route");
                    JSONObject peer = pair.optJSONObject("peer");
                    if (route == null) {
                        continue;
                    }
                    PeerSnapshot snapshot = parsePeer(route, peer);
                    if (seen.add(snapshot.id())) {
                        peers.add(snapshot);
                        sent += snapshot.sentBytes();
                        received += snapshot.receivedBytes();
                    }
                }
            }
            if (routes != null) {
                for (int index = 0; index < routes.length(); index++) {
                    JSONObject route = routes.optJSONObject(index);
                    if (route == null) {
                        continue;
                    }
                    String id = peerId(route);
                    if (seen.contains(id)) {
                        continue;
                    }
                    PeerSnapshot snapshot = parsePeer(route, peerById.get(id));
                    seen.add(snapshot.id());
                    peers.add(snapshot);
                    sent += snapshot.sentBytes();
                    received += snapshot.receivedBytes();
                }
            }

            int routeCount = routes == null ? peers.size() : routes.length();
            return new NetworkSnapshot(
                    running, peers.size(), routeCount, sent, received, error, peers);
        } catch (Exception ignored) {
            return empty("Core 状态格式无法识别");
        }
    }

    private static NetworkSnapshot empty(String error) {
        return new NetworkSnapshot(false, 0, 0, 0, 0, error, List.of());
    }

    private static Map<String, JSONObject> indexPeers(JSONArray peers) {
        Map<String, JSONObject> result = new LinkedHashMap<>();
        if (peers == null) {
            return result;
        }
        for (int index = 0; index < peers.length(); index++) {
            JSONObject peer = peers.optJSONObject(index);
            if (peer != null) {
                result.put(peerId(peer), peer);
            }
        }
        return result;
    }

    private static PeerSnapshot parsePeer(JSONObject route, JSONObject peer) {
        String id = peerId(route);
        String hostname = firstString(route, "hostname", "name");
        String virtualIp = ipv4(route.opt("ipv4_addr"));
        String version = firstString(route, "version");
        Set<String> protocols = new LinkedHashSet<>();
        long sent = 0;
        long received = 0;
        long latencyMs = positiveLong(route, "path_latency", "path_latency_latency_first");

        JSONArray connections = peer == null ? null : peer.optJSONArray("conns");
        if (connections != null) {
            long bestLatencyUs = Long.MAX_VALUE;
            for (int index = 0; index < connections.length(); index++) {
                JSONObject connection = connections.optJSONObject(index);
                if (connection == null || connection.optBoolean("is_closed", false)) {
                    continue;
                }
                collectProtocols(connection, protocols, 0);
                JSONObject stats = connection.optJSONObject("stats");
                if (stats != null) {
                    received += unsignedLong(stats, "rx_bytes");
                    sent += unsignedLong(stats, "tx_bytes");
                    long latencyUs = unsignedLong(stats, "latency_us");
                    if (latencyUs > 0) {
                        bestLatencyUs = Math.min(bestLatencyUs, latencyUs);
                    }
                }
            }
            if (bestLatencyUs != Long.MAX_VALUE) {
                latencyMs = Math.max(1, bestLatencyUs / 1_000);
            }
        }
        if (protocols.isEmpty()) {
            collectProtocols(route, protocols, 0);
        }
        boolean relay = peer == null || connections == null || connections.length() == 0;
        return new PeerSnapshot(
                id, hostname, virtualIp, new ArrayList<>(protocols), relay,
                latencyMs, version, sent, received);
    }

    private static String peerId(JSONObject object) {
        Object value = object == null ? null : object.opt("peer_id");
        return value == null || value == JSONObject.NULL ? "" : String.valueOf(value);
    }

    private static String ipv4(Object value) {
        if (value instanceof String text) {
            return text;
        }
        if (!(value instanceof JSONObject inet)) {
            return "";
        }
        Object address = inet.opt("address");
        long raw;
        if (address instanceof JSONObject object) {
            raw = unsignedLong(object, "addr");
        } else if (address instanceof Number number) {
            raw = number.longValue();
        } else {
            raw = unsignedLong(inet, "addr");
        }
        if (raw == 0) {
            return "";
        }
        return String.format(
                Locale.ROOT, "%d.%d.%d.%d",
                (raw >>> 24) & 0xff, (raw >>> 16) & 0xff, (raw >>> 8) & 0xff, raw & 0xff);
    }

    private static JSONArray firstArray(JSONObject object, String... keys) {
        for (String key : keys) {
            JSONArray value = object.optJSONArray(key);
            if (value != null) {
                return value;
            }
        }
        return null;
    }

    private static String firstString(JSONObject object, String... keys) {
        for (String key : keys) {
            Object value = object.opt(key);
            if (value instanceof String text && !text.isBlank()) {
                return text;
            }
        }
        return "";
    }

    private static long positiveLong(JSONObject object, String... keys) {
        for (String key : keys) {
            long value = unsignedLong(object, key);
            if (value > 0) {
                return value;
            }
        }
        return 0;
    }

    private static long unsignedLong(JSONObject object, String key) {
        Object value = object == null ? null : object.opt(key);
        if (value instanceof Number number) {
            return Math.max(0, number.longValue());
        }
        if (value instanceof String text) {
            try {
                return Math.max(0, Long.parseLong(text));
            } catch (NumberFormatException ignored) {
                return 0;
            }
        }
        return 0;
    }

    private static void collectProtocols(Object value, Set<String> result, int depth) {
        if (depth > 5 || value == null) {
            return;
        }
        if (value instanceof JSONObject object) {
            Iterator<String> keys = object.keys();
            while (keys.hasNext()) {
                String key = keys.next();
                Object child = object.opt(key);
                String normalizedKey = key.toLowerCase(Locale.ROOT);
                if (child instanceof String text
                        && (normalizedKey.contains("proto") || normalizedKey.contains("tunnel")
                        || normalizedKey.equals("scheme") || normalizedKey.equals("uri")
                        || normalizedKey.endsWith("addr"))) {
                    addProtocols(text, result);
                }
                collectProtocols(child, result, depth + 1);
            }
        } else if (value instanceof JSONArray array) {
            for (int index = 0; index < array.length(); index++) {
                collectProtocols(array.opt(index), result, depth + 1);
            }
        }
    }

    private static void addProtocols(String value, Set<String> result) {
        String lower = value.toLowerCase(Locale.ROOT);
        for (String protocol : List.of("tcp", "udp", "wg", "ws", "wss", "quic", "kcp", "faketcp")) {
            if (lower.equals(protocol) || lower.startsWith(protocol + "://")
                    || lower.contains("\"" + protocol + "\"")) {
                result.add(protocol.equals("wg") ? "WireGuard" : protocol.toUpperCase(Locale.ROOT));
            }
        }
    }
}
