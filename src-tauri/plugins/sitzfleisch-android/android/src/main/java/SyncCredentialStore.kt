package com.snooze26h.sitzfleisch.android

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import org.json.JSONObject
import org.json.JSONTokener
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** 登录会话的明文只在内存中出现，Preferences 中仅保存由 AndroidKeyStore 保护的密文。 */
internal object SyncCredentialStore {
  private const val PREFERENCES = "sitzfleisch_sync_credentials_v1"
  private const val ALIAS_PREFIX = "com.snooze26h.sitzfleisch.sync.v1."
  private const val MAX_SESSION_BYTES = 32 * 1024
  private const val MAX_STORED_CHARS = 48 * 1024
  private val SCOPE = Regex("[0-9a-f]{32}")

  fun validScope(scope: String): Boolean = SCOPE.matches(scope)

  fun validateSession(value: String) {
    require(value.isNotEmpty() && value.toByteArray(Charsets.UTF_8).size <= MAX_SESSION_BYTES)
    val parser = JSONTokener(value)
    require(parser.nextValue() is JSONObject && parser.nextClean() == '\u0000')
  }

  private fun keyStore(): KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

  private fun existingKey(store: KeyStore, scope: String): SecretKey? =
    (store.getEntry(ALIAS_PREFIX + scope, null) as? KeyStore.SecretKeyEntry)?.secretKey

  private fun newKey(scope: String): SecretKey =
    KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
      init(KeyGenParameterSpec.Builder(
        ALIAS_PREFIX + scope,
        KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
      )
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        .setRandomizedEncryptionRequired(true)
        // 同步需要在锁屏时刷新会话；无需反复要求指纹，密钥仍受应用 UID 与 Keystore 隔离。
        .setUserAuthenticationRequired(false)
        .build())
    }.generateKey()

  private fun aad(scope: String): ByteArray =
    "sitzfleisch-sync-session-v1:$scope".toByteArray(Charsets.UTF_8)

  @Synchronized
  fun load(context: Context, scope: String): String? {
    require(validScope(scope))
    val preferences = context.applicationContext.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    val stored = preferences.getString(scope, null) ?: return null
    require(stored.length <= MAX_STORED_CHARS)
    val envelope = JSONObject(stored)
    require(envelope.length() == 3 && envelope.optInt("version") == 1)
    val ivText = envelope.getString("iv")
    val ciphertextText = envelope.getString("ciphertext")
    require(ivText.length == 16 && ciphertextText.length <= MAX_STORED_CHARS)
    val iv = Base64.decode(ivText, Base64.NO_WRAP)
    val ciphertext = Base64.decode(ciphertextText, Base64.NO_WRAP)
    require(iv.size == 12 && ciphertext.size in 17..(MAX_SESSION_BYTES + 16))
    // 读密文时不能补建密钥；密钥缺失表示需要重新登录，不能悄悄覆盖旧会话。
    val key = existingKey(keyStore(), scope) ?: error("session key unavailable")
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, iv))
    cipher.updateAAD(aad(scope))
    val plaintext = cipher.doFinal(ciphertext)
    try {
      val value = plaintext.toString(Charsets.UTF_8)
      // Android 的 UTF-8 解码会替换坏字节，回编码相等检查避免接受损坏的明文。
      require(value.toByteArray(Charsets.UTF_8).contentEquals(plaintext))
      validateSession(value)
      return value
    } finally {
      plaintext.fill(0)
    }
  }

  @Synchronized
  fun store(context: Context, scope: String, value: String) {
    require(validScope(scope))
    validateSession(value)
    val preferences = context.applicationContext.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    val previous = preferences.getString(scope, null)
    val key = existingKey(keyStore(), scope) ?: newKey(scope)
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    // 由 Keystore 生成随机 IV；同一令牌多次保存也不能复用 GCM nonce。
    cipher.init(Cipher.ENCRYPT_MODE, key)
    cipher.updateAAD(aad(scope))
    val plaintext = value.toByteArray(Charsets.UTF_8)
    val encrypted = try { cipher.doFinal(plaintext) } finally { plaintext.fill(0) }
    require(cipher.iv.size == 12)
    val envelope = JSONObject()
      .put("version", 1)
      .put("iv", Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
      .put("ciphertext", Base64.encodeToString(encrypted, Base64.NO_WRAP))
      .toString()
    require(envelope.length <= MAX_STORED_CHARS)
    // commit 等落盘结果；失败时恢复 Preferences 的内存视图，原密钥与旧密文不先删除。
    if (!preferences.edit().putString(scope, envelope).commit()) {
      preferences.edit().apply {
        if (previous == null) remove(scope) else putString(scope, previous)
      }.commit()
      error("session persistence failed")
    }
  }

  @Synchronized
  fun delete(context: Context, scope: String) {
    require(validScope(scope))
    val preferences = context.applicationContext.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    val previous = preferences.getString(scope, null) ?: return
    if (!preferences.edit().remove(scope).commit()) {
      preferences.edit().putString(scope, previous).commit()
      error("session deletion failed")
    }
    // 加密钥不含账号或令牌，保留供下一次登录使用；退出只清当前 scope 的密文。
  }
}
