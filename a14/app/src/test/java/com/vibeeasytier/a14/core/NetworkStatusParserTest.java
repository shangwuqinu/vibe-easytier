package com.vibeeasytier.a14.core;

import org.junit.Test;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

public class NetworkStatusParserTest {
    @Test
    public void extractsNodesAndMultipleActiveProtocols() {
        String json = """
                {"map":{"phone":{"running":true,"routes":[
                  {"hostname":"seed-a","tunnels":[
                    {"tunnel_proto":"tcp"},{"tunnel_proto":"wg"}
                  ]}
                ]}}}
                """;

        NetworkSnapshot snapshot = NetworkStatusParser.parse(json, "phone");

        assertTrue(snapshot.running());
        assertEquals(1, snapshot.peerCount());
        assertTrue(snapshot.nodes().get(0).contains("TCP / WG"));
    }
}

