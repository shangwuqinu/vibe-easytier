package com.vibeeasytier.a14.storage;

import com.vibeeasytier.a14.model.Profile;
import com.vibeeasytier.a14.model.ProfileValidator;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.Collections;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public final class ProfileCatalog {
    private static final int MAX_PROFILES = 32;
    private final List<Profile> profiles;
    private final String activeInstanceName;

    public ProfileCatalog(List<Profile> profiles, String activeInstanceName) {
        this.profiles = Collections.unmodifiableList(new ArrayList<>(profiles));
        this.activeInstanceName = activeInstanceName == null ? "" : activeInstanceName;
        validate();
    }

    public static ProfileCatalog single(Profile profile) {
        return new ProfileCatalog(List.of(profile), profile.instanceName());
    }

    public List<Profile> profiles() { return profiles; }
    public String activeInstanceName() { return activeInstanceName; }

    public Profile active() {
        for (Profile profile : profiles) {
            if (profile.instanceName().equals(activeInstanceName)) {
                return profile;
            }
        }
        if (profiles.isEmpty()) {
            throw new IllegalStateException("尚未保存私有网络档案");
        }
        return profiles.get(0);
    }

    public ProfileCatalog upsert(String previousInstanceName, Profile profile) {
        ProfileValidator.validate(profile);
        List<Profile> updated = new ArrayList<>();
        boolean replaced = false;
        for (Profile existing : profiles) {
            boolean sameOriginal = previousInstanceName != null
                    && existing.instanceName().equals(previousInstanceName);
            boolean sameCurrent = existing.instanceName().equals(profile.instanceName());
            if (sameOriginal || sameCurrent) {
                if (!replaced) {
                    updated.add(profile);
                    replaced = true;
                }
            } else {
                updated.add(existing);
            }
        }
        if (!replaced) {
            updated.add(profile);
        }
        return new ProfileCatalog(updated, profile.instanceName());
    }

    public ProfileCatalog select(String instanceName) {
        if (profiles.stream().noneMatch(profile -> profile.instanceName().equals(instanceName))) {
            throw new IllegalArgumentException("所选档案不存在");
        }
        return new ProfileCatalog(profiles, instanceName);
    }

    public ProfileCatalog delete(String instanceName) {
        List<Profile> updated = profiles.stream()
                .filter(profile -> !profile.instanceName().equals(instanceName))
                .toList();
        String next = activeInstanceName;
        if (instanceName.equals(activeInstanceName)) {
            next = updated.isEmpty() ? "" : updated.get(0).instanceName();
        }
        return new ProfileCatalog(updated, next);
    }

    public JSONObject toJson() throws Exception {
        JSONArray values = new JSONArray();
        for (Profile profile : profiles) {
            values.put(profileToJson(profile));
        }
        return new JSONObject()
                .put("schemaVersion", 2)
                .put("activeInstanceName", activeInstanceName)
                .put("profiles", values);
    }

    public static ProfileCatalog fromJson(JSONObject json) throws Exception {
        JSONArray values = json.optJSONArray("profiles");
        if (values == null) {
            Profile migrated = profileFromJson(json);
            return single(migrated);
        }
        List<Profile> profiles = new ArrayList<>();
        for (int index = 0; index < values.length(); index++) {
            JSONObject value = values.optJSONObject(index);
            if (value == null) {
                throw new IllegalArgumentException("加密档案集合格式无效");
            }
            profiles.add(profileFromJson(value));
        }
        return new ProfileCatalog(profiles, json.optString("activeInstanceName", ""));
    }

    private void validate() {
        if (profiles.size() > MAX_PROFILES) {
            throw new IllegalArgumentException("私有网络档案最多 32 个");
        }
        java.util.HashSet<String> instances = new java.util.HashSet<>();
        for (Profile profile : profiles) {
            ProfileValidator.validate(profile);
            if (!instances.add(profile.instanceName())) {
                throw new IllegalArgumentException("实例名称不能重复");
            }
        }
        if (!profiles.isEmpty() && profiles.stream()
                .noneMatch(profile -> profile.instanceName().equals(activeInstanceName))) {
            throw new IllegalArgumentException("活动档案不存在");
        }
    }

    private static JSONObject profileToJson(Profile profile) throws Exception {
        JSONObject flags = new JSONObject();
        for (Map.Entry<String, Object> entry : profile.flags().entrySet()) {
            flags.put(entry.getKey(), entry.getValue());
        }
        return new JSONObject()
                .put("profileName", profile.profileName())
                .put("instanceName", profile.instanceName())
                .put("hostname", profile.hostname())
                .put("networkName", profile.networkName())
                .put("networkSecret", profile.networkSecret())
                .put("ipv4Cidr", profile.ipv4Cidr())
                .put("peers", new JSONArray(profile.peers()))
                .put("flags", flags);
    }

    private static Profile profileFromJson(JSONObject json) throws Exception {
        List<String> peers = new ArrayList<>();
        JSONArray peerArray = json.getJSONArray("peers");
        for (int index = 0; index < peerArray.length(); index++) {
            peers.add(peerArray.getString(index));
        }
        Map<String, Object> flags = new LinkedHashMap<>(Profile.defaultFlags());
        JSONObject flagObject = json.optJSONObject("flags");
        if (flagObject != null) {
            Iterator<String> keys = flagObject.keys();
            while (keys.hasNext()) {
                String key = keys.next();
                Object value = flagObject.get(key);
                if (value instanceof Integer integer) {
                    value = integer.longValue();
                }
                flags.put(key, value);
            }
        }
        return new Profile(
                json.getString("profileName"),
                json.getString("instanceName"),
                json.getString("hostname"),
                json.getString("networkName"),
                json.getString("networkSecret"),
                json.getString("ipv4Cidr"),
                peers,
                flags);
    }
}
