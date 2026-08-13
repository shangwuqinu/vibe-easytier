package com.vibeeasytier.a14.core;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

public final class PeerSnapshot {
    private final String id;
    private final String hostname;
    private final String virtualIp;
    private final List<String> protocols;
    private final boolean relay;
    private final long latencyMs;
    private final String version;
    private final long sentBytes;
    private final long receivedBytes;

    public PeerSnapshot(
            String id,
            String hostname,
            String virtualIp,
            List<String> protocols,
            boolean relay,
            long latencyMs,
            String version,
            long sentBytes,
            long receivedBytes) {
        this.id = clean(id);
        this.hostname = clean(hostname);
        this.virtualIp = clean(virtualIp);
        this.protocols = Collections.unmodifiableList(new ArrayList<>(protocols));
        this.relay = relay;
        this.latencyMs = Math.max(0, latencyMs);
        this.version = clean(version);
        this.sentBytes = Math.max(0, sentBytes);
        this.receivedBytes = Math.max(0, receivedBytes);
    }

    public String id() { return id; }
    public String hostname() { return hostname; }
    public String virtualIp() { return virtualIp; }
    public List<String> protocols() { return protocols; }
    public boolean relay() { return relay; }
    public long latencyMs() { return latencyMs; }
    public String version() { return version; }
    public long sentBytes() { return sentBytes; }
    public long receivedBytes() { return receivedBytes; }

    public String displayName() {
        if (!hostname.isBlank()) {
            return hostname;
        }
        return id.isBlank() ? "未知节点" : "节点 " + id;
    }

    public String toTransportJson() {
        try {
            JSONObject json = new JSONObject()
                    .put("id", id)
                    .put("hostname", hostname)
                    .put("virtualIp", virtualIp)
                    .put("protocols", new JSONArray(protocols))
                    .put("relay", relay)
                    .put("latencyMs", latencyMs)
                    .put("version", version)
                    .put("sentBytes", sentBytes)
                    .put("receivedBytes", receivedBytes);
            return json.toString();
        } catch (JSONException error) {
            throw new IllegalStateException("节点状态序列化失败", error);
        }
    }

    public static PeerSnapshot fromTransportJson(String source) throws JSONException {
        JSONObject json = new JSONObject(source);
        JSONArray values = json.optJSONArray("protocols");
        List<String> protocols = new ArrayList<>();
        if (values != null) {
            for (int index = 0; index < values.length(); index++) {
                String value = values.optString(index, "").trim();
                if (!value.isEmpty()) {
                    protocols.add(value);
                }
            }
        }
        return new PeerSnapshot(
                json.optString("id", ""),
                json.optString("hostname", ""),
                json.optString("virtualIp", ""),
                protocols,
                json.optBoolean("relay", false),
                json.optLong("latencyMs", 0),
                json.optString("version", ""),
                json.optLong("sentBytes", 0),
                json.optLong("receivedBytes", 0));
    }

    private static String clean(String value) {
        return value == null ? "" : value.trim();
    }
}
