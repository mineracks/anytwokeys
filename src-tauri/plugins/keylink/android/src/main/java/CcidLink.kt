// A smart card in the USB-C port that is not a YubiKey: a Token2 key, or a card in a reader. Android shows it as a
// raw USB device with a CCID interface (class 0x0B), and nothing on the phone speaks CCID for us: Yubico's library
// only lists Yubico's vendor id. So this file speaks it. It is the same handful of messages every PC/SC daemon
// sends: power the card on (and keep its ATR), carry one command APDU and bring back the reply, power it off.
//
// The keys and cards this is for are APDU-level readers (short and extended APDUs alike), so every XfrBlock carries
// one whole command and the reader does any card-side chaining itself. What the bytes MEAN is written once, in Rust.
package com.anytwokeys.keylink

import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbDeviceConnection
import android.hardware.usb.UsbEndpoint
import android.hardware.usb.UsbInterface
import android.hardware.usb.UsbManager
import java.io.ByteArrayOutputStream
import java.io.IOException

internal class CcidLink(private val manager: UsbManager, val device: UsbDevice) : Link {
  private val iface: UsbInterface = ccidInterface(device) ?: throw IOException("no smart-card interface on this device")
  private var conn: UsbDeviceConnection? = null
  private lateinit var bulkIn: UsbEndpoint
  private lateinit var bulkOut: UsbEndpoint
  private var seq = 0
  var atr: ByteArray = ByteArray(0); private set

  fun open() {
    val c = manager.openDevice(device) ?: throw IOException("the phone would not open the key in the port")
    if (!c.claimInterface(iface, true)) { c.close(); throw IOException("the key in the port is held by something else") }
    var inEp: UsbEndpoint? = null; var outEp: UsbEndpoint? = null
    for (i in 0 until iface.endpointCount) {
      val ep = iface.getEndpoint(i)
      if (ep.type != UsbConstants.USB_ENDPOINT_XFER_BULK) continue
      if (ep.direction == UsbConstants.USB_DIR_IN) inEp = ep else outEp = ep
    }
    if (inEp == null || outEp == null) { c.releaseInterface(iface); c.close(); throw IOException("the key in the port has no bulk pipes") }
    bulkIn = inEp; bulkOut = outEp; conn = c
    try { atr = exchange(PC_TO_RDR_ICC_POWER_ON, byteArrayOf(0, 0, 0), ByteArray(0)) }
    catch (e: IOException) { release(); throw e }
  }

  override fun send(apdu: ByteArray): ByteArray = exchange(PC_TO_RDR_XFR_BLOCK, byteArrayOf(0, 0, 0), apdu)

  override fun close() {
    // the card is powered down on a best effort only: if the key has already been pulled there is no one to tell
    try { exchange(PC_TO_RDR_ICC_POWER_OFF, byteArrayOf(0, 0, 0), ByteArray(0)) } catch (_: Exception) {}
    release()
  }

  private fun release() {
    val c = conn ?: return
    conn = null
    try { c.releaseInterface(iface) } catch (_: Exception) {}
    try { c.close() } catch (_: Exception) {}
  }

  // one command message out, its reply back: data block(s) joined, with time extensions waited out and chained
  // replies followed until the reader says it has given everything
  private fun exchange(type: Int, specific: ByteArray, data: ByteArray): ByteArray {
    var s = write(type, specific, data)
    val out = ByteArrayOutputStream()
    while (true) {
      val msg = read(s)
      val status = msg[7].toInt() and 0xFF; val error = msg[8].toInt() and 0xFF; val chain = msg[9].toInt() and 0xFF
      if ((status and 0x40) != 0) throw IOException(describe(error, status))
      if ((msg[0].toInt() and 0xFF) != RDR_TO_PC_DATA_BLOCK) {
        // power-off answers with a slot status, which carries no data; anything else here is not a reply we expect
        if (type == PC_TO_RDR_ICC_POWER_OFF) return ByteArray(0)
        throw IOException("the key in the port answered with a message of type 0x%02x".format(msg[0].toInt() and 0xFF))
      }
      out.write(msg, 10, msg.size - 10)
      when (chain) {
        0x00, 0x02 -> return out.toByteArray()
        // the reply continues in another block: ask for it with an empty XfrBlock whose level parameter says so
        0x01, 0x03 -> s = write(PC_TO_RDR_XFR_BLOCK, byteArrayOf(0, 0x10, 0), ByteArray(0))
        else -> throw IOException("the key in the port expects a chained command, which this app does not send")
      }
    }
  }

  // sends one message, returns its sequence number
  private fun write(type: Int, specific: ByteArray, data: ByteArray): Int {
    val c = conn ?: throw IOException("the key is no longer plugged in")
    val s = seq; seq = (seq + 1) and 0xFF
    val msg = ByteArray(10 + data.size)
    msg[0] = type.toByte()
    msg[1] = (data.size and 0xFF).toByte(); msg[2] = ((data.size shr 8) and 0xFF).toByte()
    msg[3] = ((data.size shr 16) and 0xFF).toByte(); msg[4] = ((data.size shr 24) and 0xFF).toByte()
    msg[5] = 0; msg[6] = s.toByte()
    System.arraycopy(specific, 0, msg, 7, 3)
    System.arraycopy(data, 0, msg, 10, data.size)
    // a bulk transfer is capped at 16 KiB on older Android; the reader sees one stream either way
    var off = 0
    while (off < msg.size) {
      val n = minOf(CHUNK, msg.size - off)
      val chunk = if (off == 0 && n == msg.size) msg else msg.copyOfRange(off, off + n)
      val sent = c.bulkTransfer(bulkOut, chunk, n, TIMEOUT_MS)
      if (sent != n) throw IOException("the key in the port did not take the command")
      off += n
    }
    return s
  }

  // reads one whole message for the given sequence number: stale replies are skipped, and a reader still working
  // on the card (a time extension) is waited for again rather than given up on
  private fun read(wanted: Int): ByteArray {
    val c = conn ?: throw IOException("the key is no longer plugged in")
    val buf = ByteArray(CHUNK)
    var tries = 0
    while (tries++ < 64) {
      val msg = ByteArrayOutputStream()
      var need = 10
      var empties = 0
      while (msg.size() < need) {
        val n = c.bulkTransfer(bulkIn, buf, buf.size, TIMEOUT_MS)
        if (n < 0) throw IOException("the key in the port did not answer")
        if (n == 0) { if (++empties > 8) throw IOException("the key in the port sent nothing"); continue }
        msg.write(buf, 0, n)
        if (need == 10 && msg.size() >= 10) {
          val h = msg.toByteArray()
          val len = (h[1].toInt() and 0xFF) or ((h[2].toInt() and 0xFF) shl 8) or ((h[3].toInt() and 0xFF) shl 16) or ((h[4].toInt() and 0xFF) shl 24)
          if (len < 0 || len > 65544) throw IOException("the key in the port announced an impossible reply length")
          need = 10 + len
        }
      }
      val m = msg.toByteArray()
      if ((m[6].toInt() and 0xFF) != wanted) continue
      val status = m[7].toInt() and 0xFF
      if ((status and 0xC0) == 0x80) continue
      return m
    }
    throw IOException("the key in the port kept asking for more time")
  }

  private fun describe(error: Int, status: Int): String = when {
    (status and 0x03) == 0x02 || error == 0xFB -> "no card is in the reader"
    error == 0xFE -> "the card did not answer"
    error == 0xE0 -> "the reader is busy"
    error == 0xEF -> "the PIN entry was cancelled"
    error == 0xF0 -> "the PIN entry timed out"
    error == 0xF6 || error == 0xF5 -> "the card speaks a protocol this reader does not"
    error == 0xF8 || error == 0xF7 -> "the card's answer to reset was malformed"
    error == 0x00 -> "the reader does not support that command"
    error in 1..9 -> "the reader rejected byte $error of the command"
    else -> "the reader reported error 0x%02x".format(error)
  }

  companion object {
    const val PC_TO_RDR_ICC_POWER_ON = 0x62
    const val PC_TO_RDR_ICC_POWER_OFF = 0x63
    const val PC_TO_RDR_XFR_BLOCK = 0x6F
    const val RDR_TO_PC_DATA_BLOCK = 0x80
    const val CHUNK = 16384
    const val TIMEOUT_MS = 10_000

    // the first interface of smart-card class, if the device has one
    fun ccidInterface(device: UsbDevice): UsbInterface? {
      for (i in 0 until device.interfaceCount) {
        val f = device.getInterface(i)
        if (f.interfaceClass == UsbConstants.USB_CLASS_CSCID) return f
      }
      return null
    }
  }
}
