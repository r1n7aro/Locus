package com.locus.remote

import android.app.Activity
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

@InvokeArg
class CryptArgs {
    var alias: String = ""
    var data: String = ""
}

/**
 * Seals device tokens with AES-256-GCM keys that never leave the
 * AndroidKeyStore (one key per paired desktop). Called from Rust only; no
 * page is granted this plugin's commands.
 */
@TauriPlugin
class SecureStorePlugin(activity: Activity) : Plugin(activity) {
    private val keyStore: KeyStore = KeyStore.getInstance(PROVIDER).apply { load(null) }

    private fun key(alias: String, create: Boolean): SecretKey? {
        (keyStore.getEntry(alias, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        if (!create) return null
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
        generator.init(
            KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setRandomizedEncryptionRequired(true)
                .build()
        )
        return generator.generateKey()
    }

    private fun result(data: String): JSObject {
        val ret = JSObject()
        ret.put("data", data)
        return ret
    }

    @Command
    fun seal(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(CryptArgs::class.java)
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.ENCRYPT_MODE, key(args.alias, true))
            val plaintext = Base64.decode(args.data, Base64.NO_WRAP)
            val sealed = cipher.iv + cipher.doFinal(plaintext)
            plaintext.fill(0)
            invoke.resolve(result(Base64.encodeToString(sealed, Base64.NO_WRAP)))
        } catch (error: Exception) {
            invoke.reject(error.message ?: "seal failed")
        }
    }

    @Command
    fun open(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(CryptArgs::class.java)
            val key = key(args.alias, false) ?: throw IllegalStateException("key not found")
            val sealed = Base64.decode(args.data, Base64.NO_WRAP)
            require(sealed.size > IV_BYTES) { "sealed data too short" }
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BITS, sealed, 0, IV_BYTES))
            val plaintext = cipher.doFinal(sealed, IV_BYTES, sealed.size - IV_BYTES)
            val encoded = Base64.encodeToString(plaintext, Base64.NO_WRAP)
            plaintext.fill(0)
            invoke.resolve(result(encoded))
        } catch (error: Exception) {
            invoke.reject(error.message ?: "open failed")
        }
    }

    @Command
    fun forget(invoke: Invoke) {
        try {
            val alias = invoke.parseArgs(CryptArgs::class.java).alias
            if (keyStore.containsAlias(alias)) keyStore.deleteEntry(alias)
            invoke.resolve()
        } catch (error: Exception) {
            invoke.reject(error.message ?: "forget failed")
        }
    }

    private companion object {
        const val PROVIDER = "AndroidKeyStore"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
        const val TAG_BITS = 128
    }
}
