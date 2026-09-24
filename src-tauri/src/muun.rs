//! Reading a Muun Emergency Kit, so a Muun wallet can be WATCHED here.
//!
//! Muun gives no export: no CSV, and until recently no way to get at an xpub. Its kit holds two keys — the owner's
//! and Muun's — each encrypted with the Recovery Code, and a set of output descriptors that name those keys only by
//! fingerprint. So the code is unavoidable: without it the kit says nothing about which addresses are yours.
//!
//! **Nothing that can spend is kept.** The keys are decrypted here, the account xpubs are taken from them, and the
//! private halves are dropped. What the vault stores is a descriptor — a watch-only wallet by construction.
//!
//! Every step below is Muun's own, read from `github.com/muun/recovery` (2026-09-24) rather than guessed, and each
//! is checked against their published vectors in the tests.

use bitcoin::base58;
use bitcoin::bip32::{ChainCode, Xpriv};
use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1};

/// One encrypted key as the kit prints it: base58 of
/// `version(1) | birthday(2, big endian) | ephemeral public key(33) | ciphertext(64) | salt(8, since 2019)`.
#[derive(Debug, Clone)]
pub struct KitKey {
    pub version: u8,
    /// days since Muun's own epoch, used to know how far back to look
    pub birthday: u16,
    pub eph: [u8; 33],
    pub ct: [u8; 64],
    pub salt: Option<[u8; 8]>,
}

const SALTED: usize = 147;    // the length of a modern encoded key
const UNSALTED: usize = 136;  // before the salt rode along in the first key

pub fn decode(s: &str) -> Result<KitKey, String> {
    let s: String = s.split_whitespace().collect();
    if s.len() != SALTED && s.len() != UNSALTED {
        return Err(format!("that is not one of the kit's two keys: they are {SALTED} characters long (or {UNSALTED} in a kit from before 2019), and this one is {}", s.len()));
    }
    let raw = base58::decode(&s).map_err(|_| "that key is not valid base58 — check it was copied whole".to_string())?;
    if raw.len() < 100 { return Err("that key is too short to hold a Muun key".into()); }
    let version = raw[0];
    if version != 2 { return Err(format!("this build reads version 2 keys; the kit's key says version {version}")); }
    let mut eph = [0u8; 33]; eph.copy_from_slice(&raw[3..36]);
    let mut ct = [0u8; 64]; ct.copy_from_slice(&raw[36..100]);
    let salt = if s.len() == SALTED && raw.len() >= 108 { let mut sl = [0u8; 8]; sl.copy_from_slice(&raw[100..108]); Some(sl) } else { None };
    Ok(KitKey { version, birthday: u16::from_be_bytes([raw[1], raw[2]]), eph, ct, salt })
}

/// Muun's `Scrypt256`: N=512, r=8, p=1, 32 bytes out.
pub fn scrypt256(data: &[u8], salt: &[u8]) -> Result<[u8; 32], String> {
    let params = scrypt::Params::new(9, 8, 1, 32).map_err(|e| e.to_string())?;   // log2(512) = 9
    let mut out = [0u8; 32];
    scrypt::scrypt(data, salt, &params, &mut out).map_err(|e| e.to_string())?;
    out_ok(out)
}
fn out_ok(o: [u8; 32]) -> Result<[u8; 32], String> { Ok(o) }

/// The alphabet a recovery code is written in: upper case without the letters that are read as something else.
pub const ALPHABET: &str = "ABCDEFHJKLMNPQRSTUVWXYZ2345789";
const ALPHABET_LEGACY: &str = "ABCDEFHJKMNPQRSTUVWXYZ2345789";

/// Which kind of recovery code this is. Version 2 codes begin with `L`; earlier ones do not.
pub fn code_version(code: &str) -> Result<u8, String> {
    let c = code.trim().to_uppercase();
    if c.len() != 39 { return Err(format!("a recovery code is 8 groups of 4, like LB2Q-48Z3-… ({} characters with the dashes); this one has {}", 39, c.len())); }
    let letters: String = c.chars().filter(|ch| *ch != '-').collect();
    if letters.len() != 32 { return Err("a recovery code is 8 groups of 4 characters, separated by dashes".into()); }
    let v2 = c.starts_with('L');
    let alphabet = if v2 { ALPHABET } else { ALPHABET_LEGACY };
    if let Some(bad) = letters.chars().find(|ch| !alphabet.contains(*ch)) {
        return Err(format!("‘{bad}’ is not a character a recovery code uses — they never contain G, I, O, 0, 1 or 6"));
    }
    Ok(if v2 { 2 } else { 1 })
}

/// The recovery code, turned into the key that unlocks the kit. A version 2 code is an HMAC of itself; an earlier
/// one is scrypt over the salt that rides in the key.
pub fn code_key(code: &str, salt: Option<[u8; 8]>) -> Result<[u8; 32], String> {
    use hmac::{Hmac, Mac};
    let c = code.trim().to_uppercase();
    match code_version(&c)? {
        2 => {
            let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(b"muun:rc").map_err(|e| e.to_string())?;
            mac.update(c.as_bytes());
            let mut out = [0u8; 32]; out.copy_from_slice(&mac.finalize().into_bytes()); Ok(out)
        }
        _ => {
            let salt = salt.ok_or("this kit's key carries no salt, which an older recovery code needs")?;
            scrypt256(c.as_bytes(), &salt)
        }
    }
}

/// Muun's own ECIES, as their code does it: ECDH to a shared point, its X coordinate as the AES key, and the
/// *last 16 bytes of the ephemeral public key* as the IV. AES-256-CBC, no padding — the plaintext is exactly the
/// 32-byte private key followed by its 32-byte chain code.
pub fn decrypt(k: &KitKey, unlock: [u8; 32], network: bitcoin::Network) -> Result<(Xpriv, u16), String> {
    use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
    let secp = Secp256k1::new();
    let eph = PublicKey::from_slice(&k.eph).map_err(|_| "the kit's key is damaged: its ephemeral public key is not a point on the curve".to_string())?;
    let scalar = Scalar::from_be_bytes(unlock).map_err(|_| "that recovery code does not give a usable key".to_string())?;
    let shared = eph.mul_tweak(&secp, &scalar).map_err(|_| "that recovery code does not unlock this key".to_string())?;
    let mut aes_key = [0u8; 32];
    aes_key.copy_from_slice(&shared.serialize_uncompressed()[1..33]);   // the X coordinate, and only that
    let iv = &k.eph[33 - 16..];
    let mut buf = k.ct;
    let plain = <cbc::Decryptor<aes::Aes256> as KeyIvInit>::new(&aes_key.into(), iv.into())
        .decrypt_padded_mut::<NoPadding>(&mut buf).map_err(|_| "that recovery code does not unlock this key".to_string())?
        .to_vec();
    if plain.len() != 64 { return Err("that key did not decrypt to a key and chain code".into()); }
    let xpriv = Xpriv {
        network: network.into(),
        depth: 0,
        parent_fingerprint: Default::default(),
        child_number: bitcoin::bip32::ChildNumber::from_normal_idx(0).map_err(|e| e.to_string())?,
        private_key: bitcoin::secp256k1::SecretKey::from_slice(&plain[..32]).map_err(|_| "that recovery code does not unlock this key".to_string())?,
        chain_code: ChainCode::from(<[u8; 32]>::try_from(&plain[32..]).map_err(|e| e.to_string())?),
    };
    Ok((xpriv, k.birthday))
}

// ---------- the addresses a Muun wallet has used ----------
//
// A Muun wallet is not one descriptor. Over the years it has paid to FIVE kinds of address from the same two keys,
// and one wallet's history can span all of them — so a reader that knows four would quietly show the wrong balance.
// Key order is fixed (the owner's, then Muun's) and never sorted.

/// The two keys sit at `m/schema:1'/recovery:1'`; an address is `<chain>/<index>` below that, unhardened.
pub fn derive(k: &bitcoin::bip32::Xpub, chain: u32, index: u32) -> Result<bitcoin::secp256k1::PublicKey, String> {
    use bitcoin::bip32::ChildNumber;
    let secp = Secp256k1::new();
    let path = [ChildNumber::from_normal_idx(chain).map_err(|e| e.to_string())?, ChildNumber::from_normal_idx(index).map_err(|e| e.to_string())?];
    Ok(k.derive_pub(&secp, &path).map_err(|e| e.to_string())?.public_key)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme { V1, V2, V3, V4, V5 }
pub const SCHEMES: [Scheme; 5] = [Scheme::V1, Scheme::V2, Scheme::V3, Scheme::V4, Scheme::V5];

/// `OP_2 <owner> <muun> OP_2 OP_CHECKMULTISIG` — the script behind v2, v3 and v4 alike.
fn multisig(user: &bitcoin::secp256k1::PublicKey, muun: &bitcoin::secp256k1::PublicKey) -> bitcoin::ScriptBuf {
    bitcoin::blockdata::script::Builder::new()
        .push_int(2)
        .push_key(&bitcoin::PublicKey::new(*user))
        .push_key(&bitcoin::PublicKey::new(*muun))
        .push_int(2)
        .push_opcode(bitcoin::opcodes::all::OP_CHECKMULTISIG)
        .into_script()
}

/// MuSig2 key aggregation **as Muun does it**, which is the older draft rather than the final BIP 327: the list hash
/// and each coefficient are taken over the keys' **x-only** (32-byte) form, where BIP 327 uses the 33-byte
/// compressed form. Implementing the published standard here would give a different key and therefore a wrong
/// address. Read from their vendored `musig/keyagg_impl.h` (2026-09-24).
pub fn key_agg(user: &bitcoin::secp256k1::PublicKey, muun: &bitcoin::secp256k1::PublicKey) -> Result<bitcoin::secp256k1::XOnlyPublicKey, String> {
    use bitcoin::secp256k1::{Parity, PublicKey as Pk, XOnlyPublicKey};
    use sha2::{Digest, Sha256};
    let secp = Secp256k1::new();
    let tagged = |tag: &str, msg: &[u8]| -> [u8; 32] {
        let t = Sha256::digest(tag.as_bytes());
        let mut h = Sha256::new(); h.update(t); h.update(t); h.update(msg);
        h.finalize().into()
    };
    let xs: Vec<XOnlyPublicKey> = [user, muun].iter().map(|k| k.x_only_public_key().0).collect();
    let mut list = Vec::new();
    for x in &xs { list.extend_from_slice(&x.serialize()); }
    let l = tagged("KeyAgg list", &list);
    // the coefficient of the SECOND DISTINCT key is 1; every other key's is a hash
    let second = xs.iter().find(|x| **x != xs[0]).copied();
    let mut sum: Option<Pk> = None;
    for x in &xs {
        let point = Pk::from_x_only_public_key(*x, Parity::Even);
        let term = if Some(*x) == second { point } else {
            let mut msg = l.to_vec(); msg.extend_from_slice(&x.serialize());
            let c = Scalar::from_be_bytes(tagged("KeyAgg coefficient", &msg)).map_err(|_| "a key aggregation coefficient fell outside the curve order".to_string())?;
            point.mul_tweak(&secp, &c).map_err(|e| e.to_string())?
        };
        sum = Some(match sum { None => term, Some(acc) => acc.combine(&term).map_err(|e| e.to_string())? });
    }
    Ok(sum.ok_or("no keys to aggregate")?.x_only_public_key().0)
}

/// One address of one scheme. For v5 the MuSig2 aggregate is the taproot INTERNAL key, tweaked the ordinary
/// key-path way (`TapTweak` over itself, no script tree).
pub fn address_at(user: &bitcoin::bip32::Xpub, muun: &bitcoin::bip32::Xpub, scheme: Scheme, chain: u32, index: u32, network: bitcoin::Network) -> Result<bitcoin::Address, String> {
    use bitcoin::{Address, ScriptBuf};
    let u = derive(user, chain, index)?;
    let m = derive(muun, chain, index)?;
    Ok(match scheme {
        Scheme::V1 => Address::p2pkh(bitcoin::PublicKey::new(u), network),
        Scheme::V2 => Address::p2sh(&multisig(&u, &m), network).map_err(|e| e.to_string())?,
        Scheme::V3 => Address::p2sh(&ScriptBuf::new_p2wsh(&multisig(&u, &m).wscript_hash()), network).map_err(|e| e.to_string())?,
        Scheme::V4 => Address::p2wsh(&multisig(&u, &m), network),
        Scheme::V5 => Address::p2tr(&Secp256k1::new(), key_agg(&u, &m)?, None, network),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Muun's own vector (libwallet/challenge_keys_test.go, TestChallengeKeyCryptoV2): a real encrypted key, the
    /// password that unlocks it, and the master key it must come back as.
    #[test]
    fn muuns_own_encrypted_key_decrypts_to_the_key_muun_says_it_should() {
        const ENCRYPTED: &str = "4LbSKwcepbbx4dPetoxvTWszb6mLyJHFhumzmdPRVprbn8XZBvFa6Ffarm6R3WGKutFzdxxJgQDdSHuYdjhDp1EZfSNbj12gXMND1AgmNijSxEua3LwVURU3nzWsvV5b1AsWEjJca24CaFY6T3C";
        const EXPECTED: &str = "tprv8ZgxMBicQKsPcxg1GFGZgL5zALjPwijrYNUqTi2s9JsVqDLzbpX55U9JH2PKAQKExtpdTyboZmV2ytaqr9pAHuxE1hX8k9bQgZAjq25E6P7";
        let k = decode(ENCRYPTED).unwrap();
        assert_eq!(k.birthday, 376, "the birthday Muun's test states");
        let unlock = scrypt256(b"a very good password", &k.salt.expect("this key is salted")).unwrap();
        let (xpriv, birthday) = decrypt(&k, unlock, bitcoin::Network::Regtest).unwrap();
        assert_eq!(xpriv.to_string(), EXPECTED);
        assert_eq!(birthday, 376);
    }
    #[test]
    fn a_wrong_code_is_refused_rather_than_giving_a_key_to_nowhere() {
        let k = decode("4LbSKwcepbbx4dPetoxvTWszb6mLyJHFhumzmdPRVprbn8XZBvFa6Ffarm6R3WGKutFzdxxJgQDdSHuYdjhDp1EZfSNbj12gXMND1AgmNijSxEua3LwVURU3nzWsvV5b1AsWEjJca24CaFY6T3C").unwrap();
        let wrong = scrypt256(b"not the password", &k.salt.unwrap()).unwrap();
        let other = decrypt(&k, wrong, bitcoin::Network::Regtest).map(|(x, _)| x.to_string());
        assert!(other.as_deref() != Ok("tprv8ZgxMBicQKsPcxg1GFGZgL5zALjPwijrYNUqTi2s9JsVqDLzbpX55U9JH2PKAQKExtpdTyboZmV2ytaqr9pAHuxE1hX8k9bQgZAjq25E6P7"), "a wrong code must not give the right key");
    }
    fn xpub(s: &str) -> bitcoin::bip32::Xpub {
        use std::str::FromStr;
        match bitcoin::bip32::Xpub::from_str(s) {
            Ok(x) => x,
            Err(_) => bitcoin::bip32::Xpub::from_priv(&Secp256k1::new(), &bitcoin::bip32::Xpriv::from_str(s).unwrap()),
        }
    }
    /// Muun's own address vectors (libwallet/addresses/v{2,3,4,5}_test.go), on regtest, at the paths they state.
    /// Each is a different scheme over the same pair of keys, and a wallet's history can hold all of them.
    #[test]
    fn every_kind_of_muun_address_matches_the_address_muun_computes() {
        let net = bitcoin::Network::Regtest;
        let cases: [(Scheme, &str, &str, u32, u32, &str); 4] = [
            (Scheme::V2, "tprv8dfM4H5fYJirMai5Er3LguicgUAyxmcSQbFub5ens16amX1e1HAFiW4SXnFVw9nu9FedFQqTPGTTjPEmgfvvXMKww3UcRpFbbC4DFjbCcTb",
             "tpubDBYMnFoxYLdMBZThTk4uARTe4kGPeEYWdKcaEzaUxt1cesetnxtTqmAxVkzDRou51emWytommyLWcF91SdF5KecA6Ja8oHK1FF7d5U2hMxX", 1, 0, "2NDeWrsJEwvxwVnvtWzPjhDC5B2LYkFuX2s"),
            (Scheme::V3, "tpubDAN21T1DFREQQS4FvpUktKRBzXXsj5ddenAa5u198hLXvErFFR4Lj8bt8xMG3xnZr6u8mx1vrFW9RwCDXQwQuYRCLq1j9Nr2VJUrENzteQH",
             "tpubDAsVhzq6otpasovieofhiaY38bSFGyJaBGvrJjBv9whhSnftUXfMTMVrq4BbTXT5A9b78CqqbPuM2j1ZGWdiggd7JHUTZAHh8GXDTt4Pkj9", 1, 0, "2MswEXmCLaHQq6pUTtnUVF8wVArfYSqUec5"),
            (Scheme::V4, "tpubDBf5wCeqg3KrLJiXaveDzD5JtFJ1ss9NVvFMx4RYS73SjwPEEawcAQ7V1B5DGM4gunWDeYNrnkc49sUaf7mS1wUKiJJQD6WEctExUQoLvrg",
             "tpubDB22PFkUaHoB7sgxh7exCivV5rAevVSzbB8WkFCCdbHq39r8xnYexiot4NGbi8PM6E1ySVeaHsoDeMYb6EMndpFrzVmuX8iQNExzwNpU61B", 1, 2, "bcrt1qrs3vk4dzv70syck2qdz3g06tgckq4pftenuk5p77st9glnskpvtqe2tvvk"),
            // taproot, whose output key is the MuSig2 aggregate of the pair — the one that would be wrong under BIP 327
            (Scheme::V5, "tpubDBf5wCeqg3KrLJiXaveDzD5JtFJ1ss9NVvFMx4RYS73SjwPEEawcAQ7V1B5DGM4gunWDeYNrnkc49sUaf7mS1wUKiJJQD6WEctExUQoLvrg",
             "tpubDB22PFkUaHoB7sgxh7exCivV5rAevVSzbB8WkFCCdbHq39r8xnYexiot4NGbi8PM6E1ySVeaHsoDeMYb6EMndpFrzVmuX8iQNExzwNpU61B", 1, 17, "bcrt1pvqngr85tm8hmsv2hjyrejlpsy7u65f7vke8mmrxnyuj3aj3xsapqvh8yrf"),
        ];
        for (scheme, user, muun, chain, index, want) in cases {
            let got = address_at(&xpub(user), &xpub(muun), scheme, chain, index, net).unwrap();
            assert_eq!(got.to_string(), want, "{scheme:?} at {chain}/{index}");
        }
    }
    /// The script behind v2 is the one Muun prints in its own test, key order and all.
    #[test]
    fn the_multisig_script_is_muuns_own_bytes() {
        let (u, m) = (xpub("tprv8dfM4H5fYJirMai5Er3LguicgUAyxmcSQbFub5ens16amX1e1HAFiW4SXnFVw9nu9FedFQqTPGTTjPEmgfvvXMKww3UcRpFbbC4DFjbCcTb"),
                      xpub("tpubDBYMnFoxYLdMBZThTk4uARTe4kGPeEYWdKcaEzaUxt1cesetnxtTqmAxVkzDRou51emWytommyLWcF91SdF5KecA6Ja8oHK1FF7d5U2hMxX"));
        let script = multisig(&derive(&u, 1, 0).unwrap(), &derive(&m, 1, 0).unwrap());
        assert_eq!(&bitcoin::consensus::encode::serialize_hex(&script)[2..],
                   "5221029fa5af7a34c142c1ce348b360abeb7de01df25b1d50129e58a67a6b846c9303b21025714f6b3670d4a38f5e2d6e8f239c9fc072543ce33dca54fcb4f4886a5cb87a652ae");
    }
    #[test]
    fn a_recovery_code_is_read_the_way_muun_writes_it() {
        assert_eq!(code_version("LB2Q-48Z3-25JR-S5JB-5SUS-HXHJ-RCMM-8YUA").unwrap(), 2);
        assert_eq!(code_version("R52Q-48Z3-25JR-S5JB-5SUS-HXHJ-RCMM-8YUA").unwrap(), 1);
        assert!(code_version("LB2Q-48Z3-25JR-S5JB-5SUS-HXHJ-RCMM").is_err(), "too short");
        assert!(code_version("LO2Q-48Z3-25JR-S5JB-5SUS-HXHJ-RCMM-8YUA").unwrap_err().contains('O'), "O is not in the alphabet");
    }
}
