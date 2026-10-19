package org.forwiroh;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.AtomicFile;
import java.io.File;
import java.io.FileOutputStream;
import java.security.KeyStore;
import java.security.SecureRandom;
import java.util.Arrays;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

final class Identity {
    private static final String ALIAS = "forwiroh.identity.v1";
    private Identity() { }
    static synchronized byte[] load(Context context) throws Exception {
        AtomicFile file = new AtomicFile(new File(context.getNoBackupFilesDir(), "identity.enc"));
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        if (!store.containsAlias(ALIAS)) {
            // Never silently replace an existing identity when its wrapping key is lost.
            if (file.getBaseFile().exists()) throw new IllegalStateException("Identity key unavailable; clear app data to create a new identity");
            KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
            generator.init(new KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build());
            generator.generateKey();
        }
        SecretKey wrappingKey = (SecretKey) store.getKey(ALIAS, null);
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        if (file.getBaseFile().exists()) {
            byte[] stored = file.readFully();
            if (stored.length != 60) throw new IllegalStateException("Invalid saved identity");
            cipher.init(Cipher.DECRYPT_MODE, wrappingKey, new GCMParameterSpec(128, stored, 0, 12));
            return cipher.doFinal(stored, 12, stored.length - 12);
        }
        byte[] secret = new byte[32];
        new SecureRandom().nextBytes(secret);
        try {
            cipher.init(Cipher.ENCRYPT_MODE, wrappingKey);
            FileOutputStream output = file.startWrite();
            try {
                output.write(cipher.getIV());
                output.write(cipher.doFinal(secret));
                file.finishWrite(output);
            } catch (Exception e) { file.failWrite(output); throw e; }
            return secret;
        } catch (Exception e) { Arrays.fill(secret, (byte)0); throw e; }
    }
}
