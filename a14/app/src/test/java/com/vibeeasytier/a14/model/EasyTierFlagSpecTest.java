package com.vibeeasytier.a14.model;

import org.junit.Test;

import java.util.HashSet;
import java.util.Set;

import static org.junit.Assert.assertEquals;

public class EasyTierFlagSpecTest {
    @Test
    public void exposesEveryPinnedCoreFlagExactlyOnce() {
        Set<String> keys = new HashSet<>();
        EasyTierFlagSpec.SECTIONS.forEach(section ->
                section.fields().forEach(field -> keys.add(field.key())));

        assertEquals(41, EasyTierFlagSpec.fieldCount());
        assertEquals(41, keys.size());
        assertEquals(com.vibeeasytier.a14.config.TomlProfileCodec.FLAG_KEYS, keys);
    }
}
