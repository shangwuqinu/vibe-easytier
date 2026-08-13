package com.vibeeasytier.a14.storage;

import com.vibeeasytier.a14.model.Profile;

import org.junit.Test;

import java.util.List;

import static org.junit.Assert.assertEquals;

public class ProfileCatalogTest {
    @Test
    public void migratesSingleProfileAndPreservesActiveSelection() throws Exception {
        Profile first = profile("one", "第一个");
        Profile second = profile("two", "第二个");

        ProfileCatalog catalog = ProfileCatalog.single(first)
                .upsert(null, second)
                .select("one");
        ProfileCatalog restored = ProfileCatalog.fromJson(catalog.toJson());

        assertEquals(2, restored.profiles().size());
        assertEquals("one", restored.active().instanceName());
        assertEquals("s3cret", restored.active().networkSecret());
    }

    @Test
    public void renameReplacesOriginalInsteadOfCreatingDuplicate() {
        ProfileCatalog catalog = ProfileCatalog.single(profile("one", "旧档案"))
                .upsert("one", profile("renamed", "新档案"));

        assertEquals(1, catalog.profiles().size());
        assertEquals("renamed", catalog.active().instanceName());
    }

    private static Profile profile(String instance, String name) {
        return new Profile(
                name, instance, "Pixel", "private", "s3cret", "100.76.1.2/24",
                List.of("tcp://seed.example.com:11010"), Profile.defaultFlags());
    }
}
