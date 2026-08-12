package com.vibeeasytier.a14.core;

import java.util.concurrent.TimeUnit;

public final class RetryPolicy {
    public static final long MAX_DELAY_MS = TimeUnit.MINUTES.toMillis(5);

    private RetryPolicy() {}

    public static long delayMs(int attempt, long jitter) {
        int boundedAttempt = Math.max(0, Math.min(attempt, 8));
        long base = Math.min(MAX_DELAY_MS, 1000L << boundedAttempt);
        long boundedJitter = Math.max(0, Math.min(jitter, Math.max(0, base / 5 - 1)));
        return Math.min(MAX_DELAY_MS, base + boundedJitter);
    }
}

