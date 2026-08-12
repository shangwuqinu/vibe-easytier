package com.vibeeasytier.a14.core;

import org.junit.Test;

import static org.junit.Assert.assertEquals;

public class Ipv4CidrTest {
    @Test
    public void calculatesRouteNetwork() {
        Ipv4Cidr cidr = Ipv4Cidr.parse("100.76.1.2/24");
        assertEquals("100.76.1.2", cidr.address());
        assertEquals("100.76.1.0", cidr.networkAddress());
        assertEquals(24, cidr.prefix());
    }
}

