// The wire to a YubiKey on an iPhone. Two ways in, one shape out:
//
//   USB-C  the key is in the port. iOS shows it as a smart-card slot (CryptoTokenKit), much as PC/SC does on a Mac:
//          it can be listed, so the app notices a key being plugged in.
//   NFC    the key is held to the top of the phone. There is no "present": a session is opened when a key is wanted,
//          the system sheet asks for it, and everything said to the key happens before the sheet goes away.
//
// Every command here carries bytes and nothing else. What the bytes MEAN (select, verify, sign) is written once, in
// Rust, and is the same on every platform. This file never sees a PIN as anything but part of an opaque command.
import CoreNFC
import CryptoTokenKit
import Tauri
import UIKit
import WebKit

class OpenArgs: Decodable { let reader: String; let prompt: String? }
class TransmitArgs: Decodable { let id: UInt32; let apdu: String }
class CloseArgs: Decodable { let id: UInt32 }

private func bytes(_ hex: String) -> Data? {
  var out = Data(capacity: hex.count / 2); var i = hex.startIndex
  while i < hex.endIndex { guard let j = hex.index(i, offsetBy: 2, limitedBy: hex.endIndex), let b = UInt8(hex[i..<j], radix: 16) else { return nil }; out.append(b); i = j }
  return out
}
private func hex(_ d: Data) -> String { d.map { String(format: "%02x", $0) }.joined() }

class KeylinkPlugin: Plugin, NFCTagReaderSessionDelegate {
  static let nfcName = "NFC"
  private let q = DispatchQueue(label: "com.anytwokeys.keylink")
  private var next: UInt32 = 1
  // USB-C: one smart card per open session
  private var usb: [UInt32: TKSmartCard] = [:]
  // NFC: one session at a time. It outlives a `close` by a moment, so two operations one after the other (read which
  // key this is, then sign with it) are one tap and not two.
  private var nfcSession: NFCTagReaderSession?
  private var nfcTag: NFCISO7816Tag?
  private var nfcIds: Set<UInt32> = []
  private var nfcWaiting: Invoke?
  private var nfcLinger: DispatchWorkItem?

  // ---- listing ----
  @objc public func readers(_ invoke: Invoke) {
    // every slot the system shows: a YubiKey, a Token2 key, any OpenPGP card in a reader on the port. What answers
    // decides whether it is a key (cards.rs probes it); the name decides nothing.
    let names = TKSmartCardSlotManager.default?.slotNames ?? []
    invoke.resolve(["readers": names, "nfc": NFCTagReaderSession.readingAvailable])
  }

  // ---- opening ----
  @objc public func open(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(OpenArgs.self)
    if args.reader == KeylinkPlugin.nfcName { q.async { self.openNfc(invoke, prompt: args.prompt) }; return }
    guard let manager = TKSmartCardSlotManager.default else { invoke.reject("this iPhone will not let the app talk to a key in its port"); return }
    manager.getSlot(withName: args.reader) { slot in
      // gone since it was listed, or a slot with nothing a smart-card service can talk to: not a key failing, not a key
      guard let slot = slot, slot.state == .validCard, let card = slot.makeSmartCard() else { invoke.resolve(["id": NSNull()]); return }
      card.beginSession { ok, error in
        guard ok else { invoke.reject(error?.localizedDescription ?? "the key in the port would not open"); return }
        self.q.async { let id = self.next; self.next += 1; self.usb[id] = card; invoke.resolve(["id": id]) }
      }
    }
  }

  private func openNfc(_ invoke: Invoke, prompt: String?) {
    // a key still in the field from a moment ago: carry on with it
    if let tag = nfcTag, nfcSession != nil, tag.isAvailable {
      nfcLinger?.cancel(); nfcLinger = nil
      let id = next; next += 1; nfcIds.insert(id); invoke.resolve(["id": id]); return
    }
    guard NFCTagReaderSession.readingAvailable else { invoke.reject("this iPhone cannot read a key by NFC"); return }
    if nfcWaiting != nil { invoke.reject("the phone is already waiting for a key"); return }
    endNfc(nil)
    guard let session = NFCTagReaderSession(pollingOption: [.iso14443], delegate: self, queue: q) else { invoke.reject("the phone would not start looking for a key"); return }
    session.alertMessage = prompt ?? "Hold your key flat against the top of the iPhone and keep it there."
    nfcSession = session; nfcWaiting = invoke
    session.begin()
  }

  func tagReaderSessionDidBecomeActive(_ session: NFCTagReaderSession) {}

  func tagReaderSession(_ session: NFCTagReaderSession, didDetect tags: [NFCTag]) {
    guard session === nfcSession else { return }
    guard let first = tags.first, case let .iso7816(tag) = first else { session.restartPolling(); return }
    session.connect(to: first) { error in
      self.q.async {
        guard session === self.nfcSession else { return }
        if error != nil { session.restartPolling(); return }
        self.nfcTag = tag
        session.alertMessage = "Reading. Keep the key there."
        if let invoke = self.nfcWaiting { self.nfcWaiting = nil; let id = self.next; self.next += 1; self.nfcIds.insert(id); invoke.resolve(["id": id]) }
      }
    }
  }

  func tagReaderSession(_ session: NFCTagReaderSession, didInvalidateWithError error: Error) {
    guard session === nfcSession else { return }
    nfcSession = nil; nfcTag = nil; nfcIds.removeAll(); nfcLinger?.cancel(); nfcLinger = nil
    if let invoke = nfcWaiting {
      nfcWaiting = nil
      let code = (error as? NFCReaderError)?.code
      invoke.reject(code == .readerSessionInvalidationErrorUserCanceled ? "cancelled" : code == .readerSessionInvalidationErrorSessionTimeout ? "no key was held to the phone in time" : error.localizedDescription)
    }
  }

  private func endNfc(_ message: String?) {
    nfcLinger?.cancel(); nfcLinger = nil
    if let s = nfcSession { if let m = message { s.alertMessage = m }; s.invalidate() }
    nfcSession = nil; nfcTag = nil; nfcIds.removeAll()
  }

  // ---- talking ----
  @objc public func transmit(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(TransmitArgs.self)
    guard let cmd = bytes(args.apdu) else { invoke.reject("malformed command"); return }
    q.async {
      if let card = self.usb[args.id] {
        card.transmit(cmd) { reply, error in
          if let reply = reply { invoke.resolve(["data": hex(reply)]) } else { invoke.reject(error?.localizedDescription ?? "the key in the port did not answer") }
        }
        return
      }
      guard self.nfcIds.contains(args.id), let tag = self.nfcTag, let apdu = NFCISO7816APDU(data: cmd) else { invoke.reject("the key is no longer against the phone"); return }
      tag.sendCommand(apdu: apdu) { data, sw1, sw2, error in
        if let error = error { invoke.reject("the key moved away from the phone: \(error.localizedDescription)"); return }
        var reply = data; reply.append(sw1); reply.append(sw2)
        invoke.resolve(["data": hex(reply)])
      }
    }
  }

  // ---- closing ----
  @objc public func close(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(CloseArgs.self)
    q.async {
      if let card = self.usb.removeValue(forKey: args.id) { card.endSession() }
      else if self.nfcIds.remove(args.id) != nil, self.nfcIds.isEmpty {
        // wait a moment before letting the sheet go: the next thing asked of this key rides the same tap
        let work = DispatchWorkItem { if self.nfcIds.isEmpty { self.endNfc("Done. You can take the key away.") } }
        self.nfcLinger?.cancel(); self.nfcLinger = work
        self.q.asyncAfter(deadline: .now() + 1.2, execute: work)
      }
      invoke.resolve([:])
    }
  }
}

@_cdecl("init_plugin_keylink")
func initPlugin() -> Plugin {
  return KeylinkPlugin()
}
