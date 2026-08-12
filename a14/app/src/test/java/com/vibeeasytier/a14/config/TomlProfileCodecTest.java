package com.vibeeasytier.a14.config;

import com.vibeeasytier.a14.model.Profile;

import org.junit.Test;

import java.util.List;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

public class TomlProfileCodecTest {
    @Test
    public void roundTripKeepsPrivateNetworkAndWireGuardPeer() {
        Profile input = new Profile(
                "手机私网", "phone", "Pixel", "private", "s3cret", "100.76.1.8/24",
                List.of("wg://seed.example.com:11012"), Profile.defaultFlags());

        Profile parsed = TomlProfileCodec.parse(TomlProfileCodec.render(input), "fallback");

        assertEquals("private", parsed.networkName());
        assertEquals("s3cret", parsed.networkSecret());
        assertEquals(List.of("wg://seed.example.com:11012"), parsed.peers());
        assertTrue((Boolean) parsed.flags().get("enable_encryption"));
    }

    @Test
    public void importRejectsPortalSurface() {
        String toml = """
                instance_name = "phone"
                hostname = "Pixel"
                ipv4 = "100.76.1.8/24"
                vpn_portal = "wg://0.0.0.0:11013/24"
                [network_identity]
                network_name = "private"
                network_secret = "secret"
                [[peer]]
                uri = "tcp://seed.example.com:11010"
                """;

        assertThrows(IllegalArgumentException.class, () -> TomlProfileCodec.parse(toml, "Pixel"));
    }

    @Test
    public void importRejectsWrongFlagType() {
        String toml = """
                instance_name = "phone"
                hostname = "Pixel"
                ipv4 = "100.76.1.8/24"
                [network_identity]
                network_name = "private"
                network_secret = "secret"
                [[peer]]
                uri = "tcp://seed.example.com:11010"
                [flags]
                enable_encryption = true
                private_mode = true
                mtu = "1300"
                """;

        assertThrows(IllegalArgumentException.class, () -> TomlProfileCodec.parse(toml, "Pixel"));
    }
}
