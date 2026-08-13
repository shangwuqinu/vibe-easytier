package com.vibeeasytier.a14.storage;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.AtomicFile;

import com.vibeeasytier.a14.model.Profile;

import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.util.Base64;
import java.util.List;

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
        save(profile.instanceName(), profile);
    }

    public synchronized void save(String previousInstanceName, Profile profile) throws Exception {
        ProfileCatalog catalog = exists()
                ? loadCatalog().upsert(previousInstanceName, profile)
                : ProfileCatalog.single(profile);
        saveCatalog(catalog);
    }

    public synchronized List<Profile> list() throws Exception {
        return exists() ? loadCatalog().profiles() : List.of();
    }

    public synchronized void select(String instanceName) throws Exception {
        saveCatalog(loadCatalog().select(instanceName));
    }

    public synchronized void delete(String instanceName) throws Exception {
        ProfileCatalog updated = loadCatalog().delete(instanceName);
        if (updated.profiles().isEmpty()) {
            file.delete();
            return;
        }
        saveCatalog(updated);
    }

    private void saveCatalog(ProfileCatalog catalog) throws Exception {
        byte[] plain = catalog.toJson().toString().getBytes(StandardCharsets.UTF_8);
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
        return loadCatalog().active();
    }

    private ProfileCatalog loadCatalog() throws Exception {
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
        return ProfileCatalog.fromJson(new JSONObject(new String(plain, StandardCharsets.UTF_8)));
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
}
