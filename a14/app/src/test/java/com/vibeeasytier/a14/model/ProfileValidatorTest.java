package com.vibeeasytier.a14.model;

import org.junit.Test;

import java.util.List;

import static org.junit.Assert.assertThrows;

public class ProfileValidatorTest {
    @Test
    public void acceptsWireGuardAndMultipleTransports() {
        Profile profile = new Profile(
                "测试", "vibe-a14", "Pixel", "private", "secret", "100.76.1.2/24",
                List.of("tcp://seed.example.com:11010", "udp://seed.example.com:11011", "wg://seed.example.com:11012"),
                Profile.defaultFlags());

        ProfileValidator.validate(profile);
    }

    @Test
    public void rejectsBootstrapWithoutPort() {
        Profile profile = new Profile(
                "测试", "vibe-a14", "Pixel", "private", "secret", "100.76.1.2/24",
                List.of("tcp://seed.example.com"), Profile.defaultFlags());

        assertThrows(IllegalArgumentException.class, () -> ProfileValidator.validate(profile));
    }
}

