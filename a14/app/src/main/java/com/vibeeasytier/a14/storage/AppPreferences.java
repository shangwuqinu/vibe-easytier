package com.vibeeasytier.a14.storage;

import android.content.Context;
import android.content.SharedPreferences;

public final class AppPreferences {
    private static final String FILE = "vibe_runtime";
    private static final String AUTO_CONNECT = "auto_connect";
    private static final String STATE = "state";
    private static final String DETAIL = "detail";
    private static final String PEER_COUNT = "peer_count";
    private static final String LAST_SUCCESS = "last_success";
    private static final String RETRY_AT = "retry_at";
    private static final String ALWAYS_ON = "always_on";
    private final SharedPreferences preferences;

    public AppPreferences(Context context) {
        preferences = context.getSharedPreferences(FILE, Context.MODE_PRIVATE);
    }

    public boolean autoConnect() { return preferences.getBoolean(AUTO_CONNECT, false); }
    public void setAutoConnect(boolean value) { preferences.edit().putBoolean(AUTO_CONNECT, value).apply(); }
    public boolean alwaysOn() { return preferences.getBoolean(ALWAYS_ON, false); }
    public void setAlwaysOn(boolean value) { preferences.edit().putBoolean(ALWAYS_ON, value).apply(); }

    public Status readStatus() {
        return new Status(
                preferences.getString(STATE, "DISCONNECTED"),
                preferences.getString(DETAIL, "尚未连接"),
                preferences.getInt(PEER_COUNT, 0),
                preferences.getLong(LAST_SUCCESS, 0),
                preferences.getLong(RETRY_AT, 0));
    }

    public void writeStatus(String state, String detail, int peerCount, long lastSuccess, long retryAt) {
        preferences.edit()
                .putString(STATE, state)
                .putString(DETAIL, detail)
                .putInt(PEER_COUNT, peerCount)
                .putLong(LAST_SUCCESS, lastSuccess)
                .putLong(RETRY_AT, retryAt)
                .apply();
    }

    public record Status(String state, String detail, int peerCount, long lastSuccess, long retryAt) {}
}
