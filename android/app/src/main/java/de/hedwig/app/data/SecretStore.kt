package de.hedwig.app.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.core.content.edit
import kotlinx.serialization.json.Json
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

class SecretStore(context: Context) {
    private val preferences = context.getSharedPreferences("cloud-account", Context.MODE_PRIVATE)
    private val json = Json { ignoreUnknownKeys = true }

    fun save(server: String, session: Session) {
        val plaintext = json.encodeToString(Session.serializer(), session).encodeToByteArray()
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val encrypted = cipher.doFinal(plaintext)
        preferences.edit {
            putString("server", normalizeServer(server))
            putString("session", encode(cipher.iv) + "." + encode(encrypted))
        }
    }

    fun load(): CloudAccount? {
        val server = preferences.getString("server", null) ?: return null
        val stored = preferences.getString("session", null) ?: return null
        return runCatching {
            val (iv, encrypted) = stored.split('.', limit = 2).map(::decode)
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, iv))
            val session = json.decodeFromString(
                Session.serializer(),
                cipher.doFinal(encrypted).decodeToString(),
            )
            CloudAccount(server, session)
        }.getOrNull()
    }

    fun clear() = preferences.edit { clear() }

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(
                KeyGenParameterSpec.Builder(
                    KEY_ALIAS,
                    KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
                ).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .build(),
            )
            generateKey()
        }
    }

    companion object {
        private const val KEY_ALIAS = "hedwig-device-session-v1"
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
        private fun encode(value: ByteArray) = Base64.encodeToString(value, Base64.NO_WRAP)
        private fun decode(value: String) = Base64.decode(value, Base64.NO_WRAP)
        fun normalizeServer(value: String) = value.trim().trimEnd('/')
    }
}
