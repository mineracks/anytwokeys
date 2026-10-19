// The wire to a key on an Android phone, and the two things the phone itself must do for the app:
//
//   a key in the USB-C port   listed while it is there, opened as a smart card, bytes out and bytes back. A YubiKey
//                             is carried by Yubico's library (CCID over USB). Any other maker's key or card reader
//                             (a Token2 key, say) is carried by this plugin's own CCID link, because Yubico's
//                             library only looks for Yubico's vendor id. What the bytes MEAN is written once, in Rust.
//   a key held to the phone   NFC. There is no "present": reader mode is switched on when a key is wanted, the window
//                             shows its own prompt (Android has no system sheet), and the first ISO-DEP tag to arrive
//                             is the key. The session outlives a `close` by a moment, so a read and then a sign ride
//                             one tap, as on the iPhone.
//   the store key's slot      the 32-byte key that opens the app's sealed files, kept wrapped under a key that
//                             lives in the phone's hardware keystore and never leaves it.
//   "is someone entitled here" fingerprint, face, or the phone's own PIN, through the system's prompt.
package com.anytwokeys.keylink

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.nfc.NfcAdapter
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import androidx.appcompat.app.AppCompatActivity
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
import com.yubico.yubikit.android.transport.nfc.NfcConfiguration
import com.yubico.yubikit.android.transport.nfc.NfcNotAvailable
import com.yubico.yubikit.android.transport.nfc.NfcYubiKeyDevice
import com.yubico.yubikit.android.transport.usb.UsbConfiguration
import com.yubico.yubikit.android.transport.usb.UsbYubiKeyDevice
import com.yubico.yubikit.core.smartcard.SmartCardConnection
import java.io.File
import java.io.IOException
import java.security.KeyStore
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

@InvokeArg
class OpenArgs { lateinit var reader: String; var prompt: String? = null }

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

// one open key in the port, whichever library carries its bytes
internal interface Link {
  fun send(apdu: ByteArray): ByteArray
  fun close()
}

private class YubikitLink(private val conn: SmartCardConnection) : Link {
  override fun send(apdu: ByteArray): ByteArray = conn.sendAndReceive(apdu)
  override fun close() = conn.close()
}

@TauriPlugin
class KeylinkPlugin(private val activity: Activity) : Plugin(activity) {
  // one thread for everything said to a key, so no two commands ever interleave on the wire; it also keeps the
  // NFC linger and the wait-for-a-key clock
  private val work = Executors.newSingleThreadScheduledExecutor()
  private val yubikit = YubiKitManager(activity)
  private val usb = activity.getSystemService(Context.USB_SERVICE) as UsbManager
  // YubiKeys, by Yubico's library; other makers' keys and readers, by name, once the person has let the app use them
  private val plugged = ConcurrentHashMap<String, UsbYubiKeyDevice>()
  private val ccidPlugged = ConcurrentHashMap<String, UsbDevice>()
  private val asked = HashSet<String>()
  private val links = ConcurrentHashMap<Int, Link>()
  private var next = 1
  // NFC: one key in the field at a time, shared by every id handed out while it is there. Touched only on `work`.
  private var nfcOn = false
  private var nfcConn: SmartCardConnection? = null
  private val nfcIds = HashSet<Int>()
  private var nfcWaiting: Invoke? = null
  private var nfcLinger: ScheduledFuture<*>? = null
  private var nfcClock: ScheduledFuture<*>? = null

  // ---- a key in the port ----
  override fun load(webView: android.webkit.WebView) {
    super.load(webView)
    // every YubiKey plugged in from now on (and any already there) is told to us once the person allows the app to use it
    yubikit.startUsbDiscovery(UsbConfiguration()) { device ->
      val name = "Yubico YubiKey " + device.usbDevice.deviceName
      plugged[name] = device
      device.setOnClosed { plugged.remove(name) }
    }
    // every other maker's smart card: the system tells us of arrivals and departures, and of the person's answer
    // when we ask to use one. Arrivals and departures are system broadcasts, which need no export flag; our own
    // permission answer is not, and is kept to this app.
    val plugs = IntentFilter(UsbManager.ACTION_USB_DEVICE_ATTACHED); plugs.addAction(UsbManager.ACTION_USB_DEVICE_DETACHED)
    activity.registerReceiver(usbEvents, plugs)
    ContextCompat.registerReceiver(activity, usbEvents, IntentFilter(USB_PERMISSION), ContextCompat.RECEIVER_NOT_EXPORTED)
    work.execute { for (d in usb.deviceList.values) consider(d) }
  }

  override fun onDestroy(activity: AppCompatActivity) {
    try { activity.unregisterReceiver(usbEvents) } catch (_: Exception) {}
    try { activity.unregisterReceiver(usbEvents) } catch (_: Exception) {}
    work.execute { endNfc() }
    super.onDestroy(activity)
  }

  private val usbEvents = object : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
      @Suppress("DEPRECATION")
      val device = intent.getParcelableExtra<UsbDevice>(UsbManager.EXTRA_DEVICE) ?: return
      when (intent.action) {
        UsbManager.ACTION_USB_DEVICE_ATTACHED -> work.execute { consider(device) }
        UsbManager.ACTION_USB_DEVICE_DETACHED -> work.execute { forget(device) }
        USB_PERMISSION -> {
          val granted = intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)
          work.execute { if (granted && usb.hasPermission(device)) ccidPlugged[nameOf(device)] = device }
        }
      }
    }
  }

  // a smart card that is not a YubiKey: list it if we may use it, else ask once per plugging-in
  private fun consider(device: UsbDevice) {
    if (device.vendorId == YUBICO || CcidLink.ccidInterface(device) == null) return
    if (usb.hasPermission(device)) { ccidPlugged[nameOf(device)] = device; return }
    if (!asked.add(device.deviceName)) return
    val flags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
    val intent = PendingIntent.getBroadcast(activity, 0, Intent(USB_PERMISSION).setPackage(activity.packageName), flags)
    try { usb.requestPermission(device, intent) } catch (_: Exception) { asked.remove(device.deviceName) }
  }

  private fun forget(device: UsbDevice) {
    asked.remove(device.deviceName)
    ccidPlugged.entries.removeAll { it.value.deviceName == device.deviceName }
    for ((id, link) in links) if (link is CcidLink && link.device.deviceName == device.deviceName) { links.remove(id); try { link.close() } catch (_: Exception) {} }
  }

  // "Token2 Token2 PIN+ Release 3" rather than "/dev/bus/usb/001/004": the strings need permission to read, which
  // we have by the time this is called, but a phone may still refuse them
  private fun nameOf(device: UsbDevice): String {
    val name = try { listOfNotNull(device.manufacturerName, device.productName).joinToString(" ").trim() } catch (_: Exception) { "" }
    return if (name.isEmpty()) device.deviceName else name
  }

  @Command
  fun readers(invoke: Invoke) {
    val names = JSArray()
    for (n in (plugged.keys + ccidPlugged.keys).sorted()) names.put(n)
    val nfc = try { NfcAdapter.getDefaultAdapter(activity)?.isEnabled == true } catch (_: Exception) { false }
    val out = JSObject(); out.put("readers", names); out.put("nfc", nfc)
    invoke.resolve(out)
  }

  @Command
  fun open(invoke: Invoke) {
    val args = invoke.parseArgs(OpenArgs::class.java)
    if (args.reader == NFC_NAME) { work.execute { openNfc(invoke) }; return }
    work.execute {
      val out = JSObject()
      val yubi = plugged[args.reader]
      val other = ccidPlugged[args.reader]
      try {
        val link: Link = when {
          // gone since it was listed, or nothing a smart-card connection can be made to: not a key failing, not a key
          yubi != null && yubi.supportsConnection(SmartCardConnection::class.java) -> YubikitLink(yubi.openConnection(SmartCardConnection::class.java))
          other != null -> CcidLink(usb, other).also { it.open() }
          else -> { out.put("id", org.json.JSONObject.NULL); invoke.resolve(out); return@execute }
        }
        val id = next++
        links[id] = link
        out.put("id", id); invoke.resolve(out)
      } catch (e: Exception) { invoke.reject(e.message ?: "the key in the port would not open") }
    }
  }

  @Command
  fun transmit(invoke: Invoke) {
    val args = invoke.parseArgs(TransmitArgs::class.java)
    work.execute {
      val out = JSObject()
      val link = links[args.id]
      if (link != null) {
        try { out.put("data", hex(link.send(unhex(args.apdu)))); invoke.resolve(out) }
        catch (e: Exception) { invoke.reject(e.message ?: "the key in the port did not answer") }
        return@execute
      }
      if (!nfcIds.contains(args.id)) { invoke.reject("the key is no longer plugged in"); return@execute }
      val conn = nfcConn
      if (conn == null) { invoke.reject("the key is no longer against the phone"); return@execute }
      try { out.put("data", hex(conn.sendAndReceive(unhex(args.apdu)))); invoke.resolve(out) }
      catch (e: IOException) {
        // the tag left the field mid-sentence. The ids stay (their owner will close them); reader mode stays on, so a
        // key brought back is picked up by the next `open`
        try { conn.close() } catch (_: Exception) {}
        nfcConn = null
        invoke.reject("the key moved away from the phone: " + (e.message ?: e.javaClass.simpleName))
      } catch (e: Exception) { invoke.reject(e.message ?: "the key against the phone did not answer") }
    }
  }

  @Command
  fun close(invoke: Invoke) {
    val args = invoke.parseArgs(CloseArgs::class.java)
    work.execute {
      links.remove(args.id)?.let { try { it.close() } catch (_: Exception) {} }
      if (nfcIds.remove(args.id) && nfcIds.isEmpty()) {
        // wait a moment before switching NFC off: the next thing asked of this key rides the same tap
        nfcLinger?.cancel(false)
        nfcLinger = work.schedule(Runnable { if (nfcIds.isEmpty() && nfcWaiting == null) endNfc() }, LINGER_MS, TimeUnit.MILLISECONDS)
      }
      invoke.resolve(JSObject())
    }
  }

  // ---- a key held to the phone ----
  private fun openNfc(invoke: Invoke) {
    nfcLinger?.cancel(false); nfcLinger = null
    // a key still in the field from a moment ago: carry on with it
    nfcConn?.let { val id = next++; nfcIds.add(id); val out = JSObject(); out.put("id", id); invoke.resolve(out); return }
    val adapter = NfcAdapter.getDefaultAdapter(activity)
    if (adapter == null) { invoke.reject("this phone cannot read a key by NFC"); return }
    if (!adapter.isEnabled) { invoke.reject("NFC is switched off on this phone"); return }
    if (nfcWaiting != null) { invoke.reject("the phone is already waiting for a key"); return }
    nfcWaiting = invoke
    if (!nfcOn) {
      nfcOn = true
      // reader mode: the phone stops offering itself as a card and listens for one instead. The NDEF check is
      // skipped (a key has none, and the check costs a round trip); the timeout is per command, since a card signing
      // or making a key can take a while
      val config = NfcConfiguration().skipNdefCheck(true).handleUnavailableNfc(false).timeout(NFC_COMMAND_MS)
      activity.runOnUiThread {
        try { yubikit.startNfcDiscovery(config, activity) { device -> work.execute { arrived(device) } } }
        catch (e: NfcNotAvailable) {
          work.execute {
            nfcOn = false
            val w = nfcWaiting ?: return@execute
            nfcWaiting = null; nfcClock?.cancel(false); nfcClock = null
            w.reject(if (e.isDisabled) "NFC is switched off on this phone" else "this phone cannot read a key by NFC")
          }
        }
      }
    }
    nfcClock?.cancel(false)
    nfcClock = work.schedule(Runnable {
      val w = nfcWaiting ?: return@Runnable
      nfcWaiting = null
      if (nfcIds.isEmpty()) endNfc()
      w.reject("no key was held to the phone in time")
    }, WAIT_S, TimeUnit.SECONDS)
  }

  // a tag has come into the field. Yubico's manager hands over any ISO-DEP tag, whoever made it.
  private fun arrived(device: NfcYubiKeyDevice) {
    if (!nfcOn) return
    // it may have left again before we could speak; reader mode will tell us when it is back
    val conn = try { device.openConnection(SmartCardConnection::class.java) } catch (_: IOException) { return }
    // a key taken away and brought back while ids are still live: the old connection is dead, carry on with the new
    nfcConn?.let { try { it.close() } catch (_: Exception) {} }
    nfcConn = conn
    val w = nfcWaiting ?: return
    nfcWaiting = null; nfcClock?.cancel(false); nfcClock = null
    val id = next++; nfcIds.add(id)
    val out = JSObject(); out.put("id", id); w.resolve(out)
  }

  private fun endNfc() {
    nfcLinger?.cancel(false); nfcLinger = null
    nfcClock?.cancel(false); nfcClock = null
    nfcConn?.let { try { it.close() } catch (_: Exception) {} }
    nfcConn = null; nfcIds.clear()
    nfcWaiting?.let { nfcWaiting = null; it.reject("cancelled") }
    if (nfcOn) { nfcOn = false; activity.runOnUiThread { try { yubikit.stopNfcDiscovery(activity) } catch (_: Exception) {} } }
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

  companion object {
    const val ALIAS = "anytwokeys-store-key-wrapper"
    const val NFC_NAME = "NFC"
    const val YUBICO = 0x1050
    const val USB_PERMISSION = "com.anytwokeys.keylink.USB_PERMISSION"
    const val LINGER_MS = 1200L
    const val WAIT_S = 60L
    const val NFC_COMMAND_MS = 30_000
  }
}
