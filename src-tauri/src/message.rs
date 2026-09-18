//! Signed messages. A card key signs in the classic "Bitcoin Signed Message" form (BIP 137 headers,
//! so any wallet can verify it against the key's addresses). A vault address signs in BIP 322 "full"
//! form: a virtual transaction the vault's script has to satisfy, which needs the same k touches as a spend.
use bitcoin::base64::{prelude::BASE64_STANDARD, Engine};
use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::psbt::Psbt;
use bitcoin::script::Builder;
use bitcoin::secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
use bitcoin::secp256k1::{Message, PublicKey, Secp256k1};
use bitcoin::sign_message::signed_msg_hash;
use bitcoin::{absolute, opcodes, transaction, Address, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness};
use serde::Serialize;
use std::str::FromStr;

/// What a single card key produces: the signature in both header families, and the addresses it proves.
#[derive(Debug, Clone, Serialize)]
pub struct KeySig { pub pubkey: String, pub p2pkh: String, pub p2wpkh: String, pub legacy: String, pub segwit: String }

pub fn legacy_digest(msg: &str) -> [u8; 32] { signed_msg_hash(msg).to_byte_array() }

/// Turn the card's (r, s) over the legacy digest into BIP 137 signatures for its compressed key.
pub fn key_signature(pubkey_hex: &str, network: Network, msg: &str, r: &[u8; 32], s: &[u8; 32]) -> Result<KeySig, String> {
    let secp = Secp256k1::new();
    let pk = PublicKey::from_str(pubkey_hex).map_err(|e| e.to_string())?;
    let m = Message::from_digest(legacy_digest(msg));
    let mut compact = [0u8; 64];
    compact[..32].copy_from_slice(r);
    compact[32..].copy_from_slice(s);
    let mut found = None;
    for id in 0..4 {
        let rs = RecoverableSignature::from_compact(&compact, RecoveryId::from_i32(id).unwrap()).map_err(|e| e.to_string())?;
        if secp.recover_ecdsa(&m, &rs).map(|p| p == pk).unwrap_or(false) { found = Some(id as u8); break; }
    }
    let id = found.ok_or("the signature does not recover to the card's key")?;
    let enc = |header: u8| { let mut raw = vec![header]; raw.extend_from_slice(&compact); BASE64_STANDARD.encode(raw) };
    let bpk = bitcoin::PublicKey::new(pk);
    let cpk = bitcoin::CompressedPublicKey(pk);
    Ok(KeySig {
        pubkey: pubkey_hex.to_string(),
        p2pkh: Address::p2pkh(bpk, network).to_string(),
        p2wpkh: Address::p2wpkh(&cpk, network).to_string(),
        legacy: enc(31 + id),
        segwit: enc(39 + id),
    })
}

fn tagged_hash(msg: &str) -> [u8; 32] {
    let tag = sha256::Hash::hash(b"BIP0322-signed-message");
    let mut e = sha256::Hash::engine();
    e.input(tag.as_byte_array());
    e.input(tag.as_byte_array());
    e.input(msg.as_bytes());
    sha256::Hash::from_engine(e).to_byte_array()
}

/// BIP 322 `to_spend`: a transaction nobody can mine whose single output is the address being proven.
pub fn to_spend(spk: &ScriptBuf, msg: &str) -> Transaction {
    let script_sig = Builder::new().push_int(0).push_slice(tagged_hash(msg)).into_script();
    Transaction {
        version: transaction::Version(0),
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn { previous_output: OutPoint { txid: Txid::all_zeros(), vout: 0xFFFF_FFFF }, script_sig, sequence: Sequence::ZERO, witness: Witness::new() }],
        output: vec![TxOut { value: Amount::ZERO, script_pubkey: spk.clone() }],
    }
}

/// BIP 322 `to_sign`, unsigned: spends `to_spend` into an OP_RETURN. Signing it is the proof.
pub fn to_sign(spk: &ScriptBuf, msg: &str) -> Transaction {
    let ts = to_spend(spk, msg);
    Transaction {
        version: transaction::Version(0),
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn { previous_output: OutPoint { txid: ts.compute_txid(), vout: 0 }, script_sig: ScriptBuf::new(), sequence: Sequence::ZERO, witness: Witness::new() }],
        output: vec![TxOut { value: Amount::ZERO, script_pubkey: Builder::new().push_opcode(opcodes::all::OP_RETURN).into_script() }],
    }
}

/// The PSBT the ordinary signing path can work on: one input, the proven address as its witness UTXO.
pub fn to_sign_psbt(spk: &ScriptBuf, witness_script: Option<ScriptBuf>, msg: &str) -> Result<Psbt, String> {
    let ts = to_spend(spk, msg);
    let mut psbt = Psbt::from_unsigned_tx(to_sign(spk, msg)).map_err(|e| e.to_string())?;
    psbt.inputs[0].witness_utxo = Some(ts.output[0].clone());
    psbt.inputs[0].witness_script = witness_script;
    Ok(psbt)
}

/// A finished `to_sign` becomes the signature: its input witness, serialised, base64.
pub fn encode_witness(tx: &Transaction) -> Result<String, String> {
    let w = tx.input.first().ok_or("no input")?.witness.clone();
    Ok(BASE64_STANDARD.encode(bitcoin::consensus::serialize(&w)))
}

#[derive(Debug, Clone, Serialize)]
pub struct Verified { pub ok: bool, pub kind: String, pub detail: String }

/// Check a signature against an address: 65-byte legacy/BIP 137 (recovered key must own the address),
/// or a BIP 322 witness (the address's script must be satisfied by it).
pub fn verify(address: &str, msg: &str, sig_b64: &str, network: Network) -> Result<Verified, String> {
    let addr = Address::from_str(address.trim()).map_err(|e| format!("address: {e}"))?.require_network(network).map_err(|_| "that address is for another network".to_string())?;
    let raw = BASE64_STANDARD.decode(sig_b64.trim()).map_err(|_| "the signature is not base64".to_string())?;
    if raw.len() == 65 && (27..=42).contains(&raw[0]) {
        let header = raw[0];
        let id = ((header - 27) & 3) as i32;
        let compressed = header >= 31;
        let secp = Secp256k1::new();
        let rs = RecoverableSignature::from_compact(&raw[1..], RecoveryId::from_i32(id).unwrap()).map_err(|e| e.to_string())?;
        let pk = secp.recover_ecdsa(&Message::from_digest(legacy_digest(msg)), &rs).map_err(|_| "the signature does not recover a key".to_string())?;
        let bpk = bitcoin::PublicKey { compressed, inner: pk };
        let mut owned = vec![Address::p2pkh(bpk, network)];
        if compressed {
            let cpk = bitcoin::CompressedPublicKey(pk);
            owned.push(Address::p2wpkh(&cpk, network));
            owned.push(Address::p2shwpkh(&cpk, network));
        }
        let ok = owned.iter().any(|a| *a == addr);
        return Ok(Verified { ok, kind: "legacy".into(), detail: if ok { format!("signed by key {}", hex::encode(bpk.to_bytes())) } else { format!("the signature belongs to {}", owned[0]) } });
    }
    // BIP 322: the witness must satisfy the address's script over to_sign
    let witness: Witness = bitcoin::consensus::deserialize(&raw).map_err(|_| "not a legacy signature and not a BIP 322 witness".to_string())?;
    let spk = addr.script_pubkey();
    let ts = to_spend(&spk, msg);
    let mut tx = to_sign(&spk, msg);
    tx.input[0].witness = witness.clone();
    let empty = ScriptBuf::new();
    let interp = miniscript::interpreter::Interpreter::from_txdata(&spk, &empty, &witness, Sequence::ZERO, absolute::LockTime::ZERO).map_err(|e| format!("cannot read the script: {e}"))?;
    let secp = Secp256k1::verification_only();
    let prevouts = bitcoin::sighash::Prevouts::All(std::slice::from_ref(&ts.output[0]));
    let mut sigs = 0usize;
    for el in interp.iter(&secp, &tx, 0, &prevouts) {
        match el {
            Ok(miniscript::interpreter::SatisfiedConstraint::PublicKey { .. }) => sigs += 1,
            Ok(_) => {}
            Err(e) => return Ok(Verified { ok: false, kind: "bip322".into(), detail: format!("the script is not satisfied: {e}") }),
        }
    }
    Ok(Verified { ok: true, kind: "bip322".into(), detail: format!("the address's script is satisfied by {sigs} signature{}", if sigs == 1 { "" } else { "s" }) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::SecretKey;

    #[test]
    fn key_signature_round_trips_and_verifies() {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[7u8; 32]).unwrap();
        let pk = PublicKey::from_secret_key(&secp, &sk);
        let msg = "hello from a card";
        let sig = secp.sign_ecdsa(&Message::from_digest(legacy_digest(msg)), &sk).serialize_compact();
        let (mut r, mut s) = ([0u8; 32], [0u8; 32]);
        r.copy_from_slice(&sig[..32]); s.copy_from_slice(&sig[32..]);
        let ks = key_signature(&hex::encode(pk.serialize()), Network::Signet, msg, &r, &s).unwrap();
        assert!(verify(&ks.p2pkh, msg, &ks.legacy, Network::Signet).unwrap().ok);
        assert!(verify(&ks.p2wpkh, msg, &ks.segwit, Network::Signet).unwrap().ok);
        assert!(verify(&ks.p2wpkh, msg, &ks.legacy, Network::Signet).unwrap().ok, "lenient: any of the key's addresses");
        assert!(!verify(&ks.p2pkh, "another message", &ks.legacy, Network::Signet).unwrap().ok);
    }

    #[test]
    fn bip322_p2wpkh_full_signature_verifies() {
        use bitcoin::sighash::{EcdsaSighashType, SighashCache};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[9u8; 32]).unwrap();
        let pk = PublicKey::from_secret_key(&secp, &sk);
        let cpk = bitcoin::CompressedPublicKey(pk);
        let addr = Address::p2wpkh(&cpk, Network::Signet);
        let spk = addr.script_pubkey();
        let msg = "prove this address";
        let ts = to_spend(&spk, msg);
        let mut tx = to_sign(&spk, msg);
        let h = SighashCache::new(&tx).p2wpkh_signature_hash(0, &spk, Amount::ZERO, EcdsaSighashType::All).unwrap();
        let sig = secp.sign_ecdsa(&Message::from_digest(h.to_byte_array()), &sk);
        let mut der = sig.serialize_der().to_vec(); der.push(1);
        tx.input[0].witness.push(der);
        tx.input[0].witness.push(pk.serialize());
        let _ = ts;
        let b64 = encode_witness(&tx).unwrap();
        assert!(verify(&addr.to_string(), msg, &b64, Network::Signet).unwrap().ok);
        assert!(!verify(&addr.to_string(), "other", &b64, Network::Signet).unwrap().ok);
    }
}
