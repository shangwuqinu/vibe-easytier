package com.vibeeasytier.a14.core;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

public final class NetworkSnapshot {
    private final boolean running;
    private final int peerCount;
    private final int routeCount;
    private final long sentBytes;
    private final long receivedBytes;
    private final String error;
    private final List<PeerSnapshot> peers;

    public NetworkSnapshot(
            boolean running,
            int peerCount,
            int routeCount,
            long sentBytes,
            long receivedBytes,
            String error,
            List<PeerSnapshot> peers) {
        this.running = running;
        this.peerCount = peerCount;
        this.routeCount = routeCount;
        this.sentBytes = Math.max(0, sentBytes);
        this.receivedBytes = Math.max(0, receivedBytes);
        this.error = error == null ? "" : error;
        this.peers = Collections.unmodifiableList(new ArrayList<>(peers));
    }

    public boolean running() { return running; }
    public int peerCount() { return peerCount; }
    public int routeCount() { return routeCount; }
    public long sentBytes() { return sentBytes; }
    public long receivedBytes() { return receivedBytes; }
    public String error() { return error; }
    public List<PeerSnapshot> peers() { return peers; }
}
