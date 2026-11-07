package sh.hoi.core

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import java.io.File
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.hoi_ffi.KeyStore

/**
 * Keeps the node's secret keys encrypted with an AES-256-GCM key that lives in the Android
 * Keystore (hardware-backed where the phone has it) and never leaves it. The ciphertext sits in
 * `noBackupFilesDir`: it can't be decrypted anywhere else, so it stays out of backups too.
 *
 * The core calls this from its own threads; access is serialized here.
 */
class KeystoreKeyStore(
    context: Context,
    private val alias: String = "sh.hoi.keys",
) : KeyStore {
    private val dir = File(context.noBackupFilesDir, "hoi-keys").apply { mkdirs() }
    private val lock = Any()

    override fun get(key: String): ByteArray? {
        synchronized(lock) {
            return runCatching {
                val blob = AtomicFile(file(key)).readFully()
                if (blob.size <= IV_BYTES) return null
                val cipher = Cipher.getInstance(TRANSFORMATION)
                cipher.init(Cipher.DECRYPT_MODE, secretKey(), GCMParameterSpec(TAG_BITS, blob, 0, IV_BYTES))
                cipher.updateAAD(key.toByteArray())
                cipher.doFinal(blob, IV_BYTES, blob.size - IV_BYTES)
            }.getOrNull()
        }
    }

    override fun set(key: String, value: ByteArray?) {
        synchronized(lock) {
            val target = AtomicFile(file(key))
            if (value == null) {
                target.delete()
                return
            }
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.ENCRYPT_MODE, secretKey())
            cipher.updateAAD(key.toByteArray())
            val sealed = cipher.iv + cipher.doFinal(value)
            // AtomicFile syncs data before committing the rename; readback alone does not
            // make the only database key durable before plaintext migration is committed.
            val stream = target.startWrite()
            try {
                stream.write(sealed)
                stream.fd.sync()
                target.finishWrite(stream)
            } catch (failure: Throwable) {
                target.failWrite(stream)
                throw failure
            }
        }
    }

    private fun file(key: String) = File(dir, key.replace(Regex("[^A-Za-z0-9._-]"), "_") + ".bin")

    private fun secretKey(): SecretKey {
        val store = java.security.KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    private companion object {
        const val ANDROID_KEYSTORE = "AndroidKeyStore"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
        const val TAG_BITS = 128
    }
}
