package com.vibeeasytier.a14.core;

public final class Ipv4Cidr {
    private final String address;
    private final String networkAddress;
    private final int prefix;

    private Ipv4Cidr(String address, String networkAddress, int prefix) {
        this.address = address;
        this.networkAddress = networkAddress;
        this.prefix = prefix;
    }

    public static Ipv4Cidr parse(String cidr) {
        String[] parts = cidr.split("/", -1);
        if (parts.length != 2) {
            throw new IllegalArgumentException("虚拟 IPv4 格式无效");
        }
        int prefix = Integer.parseInt(parts[1]);
        if (prefix < 0 || prefix > 32) {
            throw new IllegalArgumentException("虚拟 IPv4 前缀无效");
        }
        String[] octets = parts[0].split("\\.", -1);
        if (octets.length != 4) {
            throw new IllegalArgumentException("虚拟 IPv4 地址无效");
        }
        long address = 0;
        for (String octet : octets) {
            int value = Integer.parseInt(octet);
            if (value < 0 || value > 255) {
                throw new IllegalArgumentException("虚拟 IPv4 地址无效");
            }
            address = (address << 8) | value;
        }
        long mask = prefix == 0 ? 0 : (0xffffffffL << (32 - prefix)) & 0xffffffffL;
        return new Ipv4Cidr(parts[0], format(address & mask), prefix);
    }

    private static String format(long value) {
        return ((value >>> 24) & 255) + "." + ((value >>> 16) & 255) + "."
                + ((value >>> 8) & 255) + "." + (value & 255);
    }

    public String address() { return address; }
    public String networkAddress() { return networkAddress; }
    public int prefix() { return prefix; }
}

