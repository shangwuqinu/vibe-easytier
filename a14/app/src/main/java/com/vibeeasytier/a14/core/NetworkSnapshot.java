package com.vibeeasytier.a14.core;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

public final class NetworkSnapshot {
    private final boolean running;
    private final int peerCount;
    private final String error;
    private final List<String> nodes;

    public NetworkSnapshot(boolean running, int peerCount, String error, List<String> nodes) {
        this.running = running;
        this.peerCount = peerCount;
        this.error = error == null ? "" : error;
        this.nodes = Collections.unmodifiableList(new ArrayList<>(nodes));
    }

    public boolean running() { return running; }
    public int peerCount() { return peerCount; }
    public String error() { return error; }
    public List<String> nodes() { return nodes; }
}

