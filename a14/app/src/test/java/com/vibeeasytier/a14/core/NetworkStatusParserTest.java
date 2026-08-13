package com.vibeeasytier.a14.core;

import org.junit.Test;

import java.util.List;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

public class NetworkStatusParserTest {
    @Test
    public void extractsRoutesTrafficAndMultipleActiveProtocols() {
        String json = """
                {"map":{"phone":{"running":true,
                  "routes":[{"peer_id":42,"hostname":"seed-a","version":"2.6.4",
                    "ipv4_addr":{"address":{"addr":1682702594},"network_length":24},"path_latency":18}],
                  "peers":[{"peer_id":42,"conns":[
                    {"tunnel":{"tunnel_type":"tcp","remote_addr":{"url":"tcp://seed:11010"}},
                     "stats":{"rx_bytes":"2048","tx_bytes":"1024","latency_us":"17000"}},
                    {"tunnel":{"tunnel_type":"wg","remote_addr":{"url":"wg://seed:11012"}},
                     "stats":{"rx_bytes":"4096","tx_bytes":"3072","latency_us":"19000"}}
                  ]}]
                }}}
                """;

        NetworkSnapshot snapshot = NetworkStatusParser.parse(json, "phone");

        assertTrue(snapshot.running());
        assertEquals(1, snapshot.peerCount());
        assertEquals(1, snapshot.routeCount());
        assertEquals(4096, snapshot.sentBytes());
        assertEquals(6144, snapshot.receivedBytes());
        assertEquals(List.of("TCP", "WireGuard"), snapshot.peers().get(0).protocols());
        assertEquals(17, snapshot.peers().get(0).latencyMs());
        assertEquals("100.76.1.2", snapshot.peers().get(0).virtualIp());
    }
}
