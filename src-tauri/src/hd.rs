//! Rotating addresses for keys that cannot derive: multiplicative child keys.
//!
//! A YubiKey's OpenPGP key is one fixed scalar d. For address index i and a vault chain code cc,
//!   T_i  = HMAC-SHA512(cc, Q || i)[..32]  (as a scalar mod n, never zero)
//!   Q_i  = T_i · Q                        (the child public key; the card never learns T_i)
//! To sign digest z under Q_i, the card is handed z_c = z · T_i⁻¹ and answers (r, s_c); then
//!   s = T_i · s_c  (mod n, low-S)  and (r, s) verifies for z under Q_i, because
//!   s = k⁻¹(z/T + r·d)·T = k⁻¹(z + r·T·d) = k⁻¹(z + r·d_i).
//! Every address is fresh; watch-only needs only Q and cc. Idea credited to Astra (2026-09-18);
//! rerandomised-key ECDSA per Das, Faust and Loss. Known property: a signature under child i is a
//! valid signature under child j on z·T_j/T_i — an attacker cannot choose that message (it would
//! need a sighash preimage), and cannot even form it without cc.
use bitcoin::hashes::{hmac, sha512, Hash, HashEngine};
use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1};
use num_bigint::BigUint;
use num_traits::{One, Zero};

const N_HEX: &str = "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141";

fn n() -> BigUint { BigUint::parse_bytes(N_HEX.as_bytes(), 16).unwrap() }

fn to32(x: &BigUint) -> [u8; 32] {
    let b = x.to_bytes_be();
    let mut out = [0u8; 32];
    out[32 - b.len()..].copy_from_slice(&b);
    out
}

/// The tweak for card key `q` at address `index`.
pub fn child_scalar(cc: &[u8; 32], q: &PublicKey, index: u32) -> [u8; 32] {
    let mut eng = hmac::HmacEngine::<sha512::Hash>::new(cc);
    eng.input(&q.serialize());
    eng.input(&index.to_be_bytes());
    let h = hmac::Hmac::<sha512::Hash>::from_engine(eng);
    let t = BigUint::from_bytes_be(&h.as_byte_array()[..32]) % n();
    if t.is_zero() { return child_scalar(cc, q, index.wrapping_add(0x8000_0000)); }
    to32(&t)
}

pub fn child_pubkey(cc: &[u8; 32], q: &PublicKey, index: u32) -> PublicKey {
    let secp = Secp256k1::verification_only();
    let t = Scalar::from_be_bytes(child_scalar(cc, q, index)).expect("reduced mod n");
    q.mul_tweak(&secp, &t).expect("nonzero tweak")
}

/// z · T⁻¹ mod n — what the card is asked to sign.
pub fn card_digest(z: &[u8; 32], t: &[u8; 32]) -> [u8; 32] {
    let n = n();
    let z = BigUint::from_bytes_be(z) % &n;
    let t = BigUint::from_bytes_be(t);
    let inv = t.modpow(&(&n - BigUint::from(2u32)), &n);
    to32(&((z * inv) % &n))
}

/// T · s_c mod n, low-S.
pub fn lift_s(s_c: &[u8; 32], t: &[u8; 32]) -> [u8; 32] {
    let n = n();
    let mut s = (BigUint::from_bytes_be(s_c) * BigUint::from_bytes_be(t)) % &n;
    let half = (&n - BigUint::one()) / BigUint::from(2u32);
    if s > half { s = &n - s; }
    to32(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{ecdsa, Message, SecretKey};

    /// A software "card" that ECDSA-signs any digest with a fixed key stands in for the YubiKey.
    #[test]
    fn lifted_signature_verifies_under_child_only() {
        let secp = Secp256k1::new();
        let d = SecretKey::from_slice(&[7u8; 32]).unwrap();
        let q = d.public_key(&secp);
        let cc = [3u8; 32];
        for i in 0..20u32 {
            let t = child_scalar(&cc, &q, i);
            let qi = child_pubkey(&cc, &q, i);
            let z = [i as u8 + 1; 32];
            let zc = card_digest(&z, &t);
            let sig = secp.sign_ecdsa(&Message::from_digest(zc), &d);
            let c = sig.serialize_compact();
            let (r, sc) = (&c[..32], &c[32..]);
            let s = lift_s(sc.try_into().unwrap(), &t);
            let mut lifted = [0u8; 64];
            lifted[..32].copy_from_slice(r);
            lifted[32..].copy_from_slice(&s);
            let lifted = ecdsa::Signature::from_compact(&lifted).unwrap();
            assert!(secp.verify_ecdsa(&Message::from_digest(z), &lifted, &qi).is_ok(), "child {i}");
            assert!(secp.verify_ecdsa(&Message::from_digest(z), &lifted, &q).is_err(), "must not verify under the card key");
        }
    }
}
