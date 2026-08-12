package com.vibeeasytier.a14.core;

import org.junit.Test;

import java.util.concurrent.TimeUnit;

import static org.junit.Assert.assertEquals;

public class RetryPolicyTest {
    @Test
    public void exponentialRetryIsCappedAtFiveMinutes() {
        assertEquals(1000, RetryPolicy.delayMs(0, 0));
        assertEquals(32000, RetryPolicy.delayMs(5, 0));
        assertEquals(TimeUnit.MINUTES.toMillis(5), RetryPolicy.delayMs(100, Long.MAX_VALUE));
    }
}

