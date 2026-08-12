package com.vibeeasytier.a14.storage;

import android.content.Context;
import android.content.SharedPreferences;

import org.json.JSONArray;

import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Date;
import java.util.List;
import java.util.Locale;

public final class LogStore {
    private static final int MAX_LINES = 200;
    private static final String FILE = "vibe_logs";
    private static final String KEY = "lines";
    private final SharedPreferences preferences;

    public LogStore(Context context) {
        preferences = context.getSharedPreferences(FILE, Context.MODE_PRIVATE);
    }

    public synchronized void append(String message) {
        List<String> lines = read();
        String timestamp = new SimpleDateFormat("MM-dd HH:mm:ss", Locale.CHINA).format(new Date());
        lines.add(timestamp + "  " + sanitize(message));
        if (lines.size() > MAX_LINES) {
            lines = new ArrayList<>(lines.subList(lines.size() - MAX_LINES, lines.size()));
        }
        preferences.edit().putString(KEY, new JSONArray(lines).toString()).apply();
    }

    public synchronized List<String> read() {
        List<String> result = new ArrayList<>();
        JSONArray array;
        try {
            array = new JSONArray(preferences.getString(KEY, "[]"));
            for (int index = 0; index < array.length(); index++) {
                result.add(array.getString(index));
            }
        } catch (Exception ignored) {
            preferences.edit().remove(KEY).apply();
        }
        return result;
    }

    public void clear() {
        preferences.edit().remove(KEY).apply();
    }

    private static String sanitize(String value) {
        return value.replace('\n', ' ').replace('\r', ' ').trim();
    }
}

