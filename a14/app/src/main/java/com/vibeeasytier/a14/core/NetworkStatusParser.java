package com.vibeeasytier.a14.core;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.Iterator;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;

public final class NetworkStatusParser {
    private NetworkStatusParser() {}

    public static NetworkSnapshot parse(String json, String instanceName) {
        if (json == null || json.isBlank()) {
            return new NetworkSnapshot(false, 0, "尚未获取到 Core 状态", List.of());
        }
        try {
            JSONObject map = new JSONObject(json).optJSONObject("map");
            JSONObject instance = map == null ? null : map.optJSONObject(instanceName);
            if (instance == null) {
                return new NetworkSnapshot(false, 0, "Core 尚未报告网络实例", List.of());
            }
            boolean running = instance.optBoolean("running", false);
            String error = instance.optString("error_msg", "");
            JSONArray routes = firstArray(instance, "routes", "route_list");
            List<String> nodes = new ArrayList<>();
            if (routes != null) {
                for (int index = 0; index < routes.length(); index++) {
                    JSONObject route = routes.optJSONObject(index);
                    if (route == null) {
                        continue;
                    }
                    String node = firstString(route, "hostname", "peer_id", "id", "ipv4_addr", "ipv4");
                    Set<String> protocols = new LinkedHashSet<>();
                    collectProtocols(route, protocols, 0);
                    if (node.isBlank()) {
                        node = "节点 " + (index + 1);
                    }
                    if (!protocols.isEmpty()) {
                        node += "  " + String.join(" / ", protocols);
                    }
                    nodes.add(node);
                }
            }
            return new NetworkSnapshot(running, nodes.size(), error, nodes);
        } catch (Exception error) {
            return new NetworkSnapshot(false, 0, "Core 状态格式无法识别", List.of());
        }
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
                        || normalizedKey.equals("scheme") || normalizedKey.equals("uri"))) {
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
        for (String protocol : List.of("tcp", "udp", "wg", "ws", "wss", "quic", "kcp")) {
            if (lower.equals(protocol) || lower.startsWith(protocol + "://")
                    || lower.contains("\"" + protocol + "\"")) {
                result.add(protocol.toUpperCase(Locale.ROOT));
            }
        }
    }
}
