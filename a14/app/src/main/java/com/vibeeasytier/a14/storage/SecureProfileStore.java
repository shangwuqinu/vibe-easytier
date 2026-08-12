package com.vibeeasytier.a14.storage;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.AtomicFile;

import com.vibeeasytier.a14.model.Profile;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.util.ArrayList;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.Iterator;
import java.util.List;
import java.util.Map;

import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

public final class SecureProfileStore {
    private static final String KEY_ALIAS = "vibe-easytier-profile-v1";
    private static final String TRANSFORMATION = "AES/GCM/NoPadding";
    private final AtomicFile file;

    public SecureProfileStore(Context context) {
        file = new AtomicFile(new File(context.getFilesDir(), "profile.enc"));
    }

    public boolean exists() {
        return file.getBaseFile().isFile();
    }

    public synchronized void save(Profile profile) throws Exception {
        byte[] plain = toJson(profile).toString().getBytes(StandardCharsets.UTF_8);
        Cipher cipher = Cipher.getInstance(TRANSFORMATION);
        cipher.init(Cipher.ENCRYPT_MODE, getOrCreateKey());
        JSONObject envelope = new JSONObject()
                .put("version", 1)
                .put("iv", Base64.getEncoder().encodeToString(cipher.getIV()))
                .put("ciphertext", Base64.getEncoder().encodeToString(cipher.doFinal(plain)));

        FileOutputStream output = null;
        try {
            output = file.startWrite();
            output.write(envelope.toString().getBytes(StandardCharsets.UTF_8));
            file.finishWrite(output);
        } catch (Exception error) {
            if (output != null) {
                file.failWrite(output);
            }
            throw error;
        }
    }

    public synchronized Profile load() throws Exception {
        byte[] envelopeBytes;
        try (var input = file.openRead(); var output = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[4096];
            int read;
            while ((read = input.read(buffer)) != -1) {
                output.write(buffer, 0, read);
            }
            envelopeBytes = output.toByteArray();
        }
        JSONObject envelope = new JSONObject(new String(envelopeBytes, StandardCharsets.UTF_8));
        if (envelope.getInt("version") != 1) {
            throw new IllegalStateException("不支持的档案加密版本");
        }
        Cipher cipher = Cipher.getInstance(TRANSFORMATION);
        cipher.init(
                Cipher.DECRYPT_MODE,
                getOrCreateKey(),
                new GCMParameterSpec(128, Base64.getDecoder().decode(envelope.getString("iv"))));
        byte[] plain = cipher.doFinal(Base64.getDecoder().decode(envelope.getString("ciphertext")));
        return fromJson(new JSONObject(new String(plain, StandardCharsets.UTF_8)));
    }

    private SecretKey getOrCreateKey() throws Exception {
        KeyStore keyStore = KeyStore.getInstance("AndroidKeyStore");
        keyStore.load(null);
        KeyStore.Entry existing = keyStore.getEntry(KEY_ALIAS, null);
        if (existing instanceof KeyStore.SecretKeyEntry secretKeyEntry) {
            return secretKeyEntry.getSecretKey();
        }
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build());
        return generator.generateKey();
    }

    private static JSONObject toJson(Profile profile) throws Exception {
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

    private static Profile fromJson(JSONObject json) throws Exception {
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
