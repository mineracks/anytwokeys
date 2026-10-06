// The wire to a YubiKey on an Android phone, and the two things the phone itself must do for the app:
//
//   a key in the USB-C port   listed while it is there, opened as a smart card, bytes out and bytes back. Yubico's
//                             library carries them (CCID over USB); what they MEAN is written once, in Rust.
//   the store key's slot      the 32-byte key that opens the app's sealed files, kept wrapped under a key that
//                             lives in the phone's hardware keystore and never leaves it.
//   "is someone entitled here" fingerprint, face, or the phone's own PIN, through the system's prompt.
package com.anytwokeys.keylink

import android.app.Activity
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.yubico.yubikit.android.YubiKitManager
import com.yubico.yubikit.android.transport.usb.UsbConfiguration
import com.yubico.yubikit.android.transport.usb.UsbYubiKeyDevice
import com.yubico.yubikit.core.smartcard.SmartCardConnection
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

@InvokeArg
class OpenArgs { lateinit var reader: String }

@InvokeArg
class TransmitArgs { var id: Int = 0; lateinit var apdu: String }

@InvokeArg
class CloseArgs { var id: Int = 0 }

@InvokeArg
class SlotArgs { lateinit var account: String; var key: String? = null }

@InvokeArg
class PresenceArgs { lateinit var reason: String }

@InvokeArg
class SecureArgs { var on: Boolean = false }

private fun unhex(s: String): ByteArray = ByteArray(s.length / 2) { s.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
private fun hex(b: ByteArray): String = b.joinToString("") { "%02x".format(it) }

@TauriPlugin
class KeylinkPlugin(private val activity: Activity) : Plugin(activity) {
  private val work = Executors.newSingleThreadExecutor()
  private val yubikit = YubiKitManager(activity)
  private val plugged = ConcurrentHashMap<String, UsbYubiKeyDevice>()
  private val open = ConcurrentHashMap<Int, SmartCardConnection>()
  private var next = 1

  // ---- a key in the port ----
  override fun load(webView: android.webkit.WebView) {
    super.load(webView)
    // every key plugged in from now on (and any already there) is told to us once the person allows the app to use it
    yubikit.startUsbDiscovery(UsbConfiguration()) { device ->
      val name = "Yubico YubiKey " + device.usbDevice.deviceName
      plugged[name] = device
      device.setOnClosed { plugged.remove(name) }
    }
  }

  @Command
  fun readers(invoke: Invoke) {
    val names = JSArray()
    for (n in plugged.keys.sorted()) names.put(n)
    val out = JSObject(); out.put("readers", names); out.put("nfc", false)
    invoke.resolve(out)
  }

  @Command
  fun open(invoke: Invoke) {
    val args = invoke.parseArgs(OpenArgs::class.java)
    work.execute {
      val out = JSObject()
      val device = plugged[args.reader]
      // gone since it was listed, or nothing a smart-card connection can be made to: not a key failing, not a key
      if (device == null || !device.supportsConnection(SmartCardConnection::class.java)) { out.put("id", org.json.JSONObject.NULL); invoke.resolve(out); return@execute }
      try {
        val conn = device.openConnection(SmartCardConnection::class.java)
        val id = synchronized(this) { next++ }
        open[id] = conn
        out.put("id", id); invoke.resolve(out)
      } catch (e: Exception) { invoke.reject(e.message ?: "the key in the port would not open") }
    }
  }

  @Command
  fun transmit(invoke: Invoke) {
    val args = invoke.parseArgs(TransmitArgs::class.java)
    work.execute {
      val conn = open[args.id]
      if (conn == null) { invoke.reject("the key is no longer plugged in"); return@execute }
      try { val out = JSObject(); out.put("data", hex(conn.sendAndReceive(unhex(args.apdu)))); invoke.resolve(out) }
      catch (e: Exception) { invoke.reject(e.message ?: "the key in the port did not answer") }
    }
  }

  @Command
  fun close(invoke: Invoke) {
    val args = invoke.parseArgs(CloseArgs::class.java)
    work.execute { try { open.remove(args.id)?.close() } catch (_: Exception) {}; invoke.resolve(JSObject()) }
  }

  // ---- the store key's slot ----
  private fun slotFile(account: String): File {
    val dir = File(activity.filesDir, "keyslots"); dir.mkdirs()
    return File(dir, hex(MessageDigest.getInstance("SHA-256").digest(account.toByteArray())).substring(0, 32) + ".bin")
  }

  // one wrapping key, made in the phone's keystore (its secure chip where there is one) and never readable outside it
  private fun wrapper(): SecretKey {
    val ks = KeyStore.getInstance("AndroidKeyStore"); ks.load(null)
    (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
    val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
    gen.init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
      .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setKeySize(256).build())
    return gen.generateKey()
  }

  @Command
  fun slotGet(invoke: Invoke) {
    val args = invoke.parseArgs(SlotArgs::class.java)
    work.execute {
      try {
        val f = slotFile(args.account); val out = JSObject()
        if (!f.exists()) { out.put("key", org.json.JSONObject.NULL); invoke.resolve(out); return@execute }
        val blob = f.readBytes()
        val c = Cipher.getInstance("AES/GCM/NoPadding"); c.init(Cipher.DECRYPT_MODE, wrapper(), GCMParameterSpec(128, blob, 0, 12))
        out.put("key", hex(c.doFinal(blob, 12, blob.size - 12))); invoke.resolve(out)
      } catch (e: Exception) { invoke.reject("the phone's keystore would not give up the key for Any Two Keys: " + (e.message ?: e.javaClass.simpleName)) }
    }
  }

  @Command
  fun slotSet(invoke: Invoke) {
    val args = invoke.parseArgs(SlotArgs::class.java)
    work.execute {
      try {
        val c = Cipher.getInstance("AES/GCM/NoPadding"); c.init(Cipher.ENCRYPT_MODE, wrapper())
        val ct = c.doFinal(unhex(args.key ?: ""))
        val f = slotFile(args.account); val tmp = File(f.path + ".tmp"); tmp.writeBytes(c.iv + ct)
        if (!tmp.renameTo(f)) throw java.io.IOException("could not write the slot")
        invoke.resolve(JSObject())
      } catch (e: Exception) { invoke.reject("the phone's keystore would not store the key: " + (e.message ?: e.javaClass.simpleName)) }
    }
  }

  @Command
  fun slotForget(invoke: Invoke) {
    val args = invoke.parseArgs(SlotArgs::class.java)
    slotFile(args.account).delete(); invoke.resolve(JSObject())
  }

  // ---- is someone entitled here ----
  @Command
  fun presence(invoke: Invoke) {
    val args = invoke.parseArgs(PresenceArgs::class.java)
    val allowed = BiometricManager.Authenticators.BIOMETRIC_WEAK or BiometricManager.Authenticators.DEVICE_CREDENTIAL
    val fa = activity as? FragmentActivity
    if (fa == null) { invoke.reject("this phone cannot ask for a fingerprint or its PIN"); return }
    if (BiometricManager.from(activity).canAuthenticate(allowed) != BiometricManager.BIOMETRIC_SUCCESS) { invoke.reject("this phone has no screen lock set, so it cannot confirm who is holding it"); return }
    activity.runOnUiThread {
      val prompt = BiometricPrompt(fa, ContextCompat.getMainExecutor(activity), object : BiometricPrompt.AuthenticationCallback() {
        override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) { invoke.resolve(JSObject()) }
        override fun onAuthenticationError(code: Int, message: CharSequence) { invoke.reject(message.toString()) }
      })
      prompt.authenticate(BiometricPrompt.PromptInfo.Builder().setTitle("Any Two Keys").setSubtitle(args.reason).setAllowedAuthenticators(allowed).build())
    }
  }

  // ---- the window kept out of screenshots, recordings and the recents preview ----
  @Command
  fun secure(invoke: Invoke) {
    val args = invoke.parseArgs(SecureArgs::class.java)
    activity.runOnUiThread {
      if (args.on) activity.window.addFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE)
      else activity.window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE)
      invoke.resolve(JSObject())
    }
  }

  companion object { const val ALIAS = "anytwokeys-store-key-wrapper" }
}
