package com.vibeeasytier.a14.storage;

import android.content.Context;
import android.content.SharedPreferences;

public final class AppPreferences {
    private static final String FILE = "vibe_runtime";
    private static final String AUTO_CONNECT = "auto_connect";
    private static final String STATE = "state";
    private static final String DETAIL = "detail";
    private static final String PEER_COUNT = "peer_count";
    private static final String ROUTE_COUNT = "route_count";
    private static final String SENT_BYTES = "sent_bytes";
    private static final String RECEIVED_BYTES = "received_bytes";
    private static final String LAST_SUCCESS = "last_success";
    private static final String RETRY_AT = "retry_at";
    private static final String ALWAYS_ON = "always_on";
    private static final String THEME = "theme";
    private final SharedPreferences preferences;

    public AppPreferences(Context context) {
        preferences = context.getSharedPreferences(FILE, Context.MODE_PRIVATE);
    }

    public boolean autoConnect() { return preferences.getBoolean(AUTO_CONNECT, false); }
    public void setAutoConnect(boolean value) { preferences.edit().putBoolean(AUTO_CONNECT, value).apply(); }
    public boolean alwaysOn() { return preferences.getBoolean(ALWAYS_ON, false); }
    public void setAlwaysOn(boolean value) { preferences.edit().putBoolean(ALWAYS_ON, value).apply(); }
    public String theme() { return preferences.getString(THEME, "system"); }
    public void setTheme(String value) {
        if (!"light".equals(value) && !"dark".equals(value)) {
            value = "system";
        }
        preferences.edit().putString(THEME, value).apply();
    }

    public Status readStatus() {
        return new Status(
                preferences.getString(STATE, "DISCONNECTED"),
                preferences.getString(DETAIL, "尚未连接"),
                preferences.getInt(PEER_COUNT, 0),
                preferences.getInt(ROUTE_COUNT, 0),
                preferences.getLong(SENT_BYTES, 0),
                preferences.getLong(RECEIVED_BYTES, 0),
                preferences.getLong(LAST_SUCCESS, 0),
                preferences.getLong(RETRY_AT, 0));
    }

    public void writeStatus(
            String state,
            String detail,
            int peerCount,
            int routeCount,
            long sentBytes,
            long receivedBytes,
            long lastSuccess,
            long retryAt) {
        preferences.edit()
                .putString(STATE, state)
                .putString(DETAIL, detail)
                .putInt(PEER_COUNT, peerCount)
                .putInt(ROUTE_COUNT, routeCount)
                .putLong(SENT_BYTES, sentBytes)
                .putLong(RECEIVED_BYTES, receivedBytes)
                .putLong(LAST_SUCCESS, lastSuccess)
                .putLong(RETRY_AT, retryAt)
                .apply();
    }

    public record Status(
            String state,
            String detail,
            int peerCount,
            int routeCount,
            long sentBytes,
            long receivedBytes,
            long lastSuccess,
            long retryAt) {}
}
