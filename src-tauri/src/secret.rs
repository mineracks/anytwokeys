//! A signing key's secret made OUTSIDE the key, so that it can be written down, written to a spare card, or come
//! from the person's own randomness — the one thing a card-made key can never offer.
//!
//! The backup form is a standard BIP39 phrase of 12, 15, 18, 21 or 24 words (128 to 256 bits of entropy, with the
//! usual checksum), so any BIP39 tool can check the words. The card's secp256k1 secret is derived from the phrase's
//! ENTROPY by one tagged hash — `SHA-256("anytwokeys/card-key/v1" ‖ entropy)` — the same rule for every length, and
//! checked to be a valid scalar. The words are therefore not a wallet seed: no BIP32 path is derived, and they open
//! no other wallet. Sources:
//!   app   — 128 to 256 bits from the operating system's randomness (12 to 24 words)
//!   words — a phrase typed in (a backup, or a spare being made), or one word SHORT of a phrase drawn by hand
//!           (Entropia tablets: 1,024 two-sided pieces carrying the 2,048 words): the last word carries free bits
//!           and the checksum, and the valid endings are listed for the person to pick one
//!   dice  — 99 or more rolls of a six-sided die hashed once with SHA-256 into 256 bits of entropy (the scheme
//!           Coldcard documents, so a Coldcard shows the same 24 words for the same rolls)
//!   hex   — 32 or 64 hex characters of entropy from any other source
//! Nothing here stores anything.
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub type Secret = Zeroizing<[u8; 32]>;
pub type Entropy = Zeroizing<Vec<u8>>;
const TAG: &[u8] = b"anytwokeys/card-key/v1";
pub const LENGTHS: [usize; 5] = [12, 15, 18, 21, 24];

/// The scalar a phrase's entropy makes.
pub fn secret_of(entropy: &Entropy) -> Result<Secret, String> {
    let mut h = Sha256::new(); h.update(TAG); h.update(&entropy[..]);
    let b: [u8; 32] = h.finalize().into();
    SecretKey::from_slice(&b).map_err(|_| "that entropy makes no usable key (a one-in-2^128 chance): make it again".to_string())?;
    Ok(Zeroizing::new(b))
}

fn check_len(bytes: usize) -> Result<(), String> { if [16usize, 20, 24, 28, 32].contains(&bytes) { Ok(()) } else { Err(format!("{} bits of entropy is not a BIP39 length (128, 160, 192, 224 or 256)", bytes * 8)) } }

/// Any BIP39 length's worth of entropy from the operating system: 12 words = 16 bytes … 24 = 32.
pub fn from_app(words: usize) -> Result<Entropy, String> {
    if !LENGTHS.contains(&words) { return Err(format!("{words} words: make 12, 15, 18, 21 or 24")); }
    let bytes = words * 4 / 3;
    let r = crate::lockbox::random::<32>();
    Ok(Zeroizing::new(r[..bytes].to_vec()))
}

/// The words that carry this entropy.
pub fn words_of(entropy: &Entropy) -> String { bip39::Mnemonic::from_entropy(&entropy[..]).map(|m| m.to_string()).unwrap_or_default() }

fn normalise(words: &str) -> Vec<String> { words.split_whitespace().map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect() }

/// A phrase → its entropy. Any BIP39 length; the checksum must match.
pub fn from_words(words: &str) -> Result<Entropy, String> {
    let w = normalise(words);
    if !LENGTHS.contains(&w.len()) { return Err(format!("a phrase is 12, 15, 18, 21 or 24 words; that is {}", w.len())); }
    let m = bip39::Mnemonic::parse_normalized(&w.join(" ")).map_err(|_| "a word is not in the list, or the checksum does not match: check each word against the page".to_string())?;
    let (e, n) = m.to_entropy_array();
    check_len(n)?;
    Ok(Zeroizing::new(e[..n].to_vec()))
}

/// The words that can complete a phrase drawn one word short: every valid last word (8 for 23 drawn, 128 for 11).
pub fn completions(drawn: &str) -> Result<Vec<String>, String> {
    let w = normalise(drawn);
    let total = LENGTHS.iter().copied().find(|n| n - 1 == w.len()).ok_or_else(|| format!("{} drawn words complete to no phrase: draw 11, 14, 17, 20 or 23", w.len()))?;
    let list = bip39::Language::English.word_list();
    let mut bits = Vec::with_capacity(w.len() * 11);
    for word in &w {
        let i = list.iter().position(|x| *x == word.as_str()).ok_or_else(|| format!("\"{word}\" is not a word on the list"))?;
        for b in (0..11).rev() { bits.push((i >> b) & 1 == 1); }
    }
    let ent_bits = total * 11 * 32 / 33;             // 128 … 256
    let free = ent_bits - bits.len();                // 7 for 12 words, 3 for 24
    let mut out = Vec::new();
    for f in 0..(1u32 << free) {
        let mut all = bits.clone();
        for b in (0..free).rev() { all.push((f >> b) & 1 == 1); }
        let mut entropy = vec![0u8; ent_bits / 8];
        for (i, bit) in all.iter().enumerate() { if *bit { entropy[i / 8] |= 1 << (7 - i % 8); } }
        if let Ok(m) = bip39::Mnemonic::from_entropy(&entropy) { if let Some(last) = m.words().last() { out.push(last.to_string()); } }
    }
    if out.len() != 1 << free { return Err("could not complete those words".into()); }
    Ok(out)
}

/// 99 or more rolls of a six-sided die, written as digits 1–6, hashed once with SHA-256 — Coldcard's rule.
pub fn from_dice(rolls: &str) -> Result<Entropy, String> {
    let digits: String = rolls.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some(c) = digits.chars().find(|c| !('1'..='6').contains(c)) { return Err(format!("\"{c}\" is not a roll of a six-sided die (1 to 6)")); }
    if digits.len() < 99 { return Err(format!("{} rolls carry fewer than 256 bits; 99 or more are needed (that is {} to go)", digits.len(), 99 - digits.len())); }
    Ok(Zeroizing::new(Sha256::digest(digits.as_bytes()).to_vec()))
}

/// 32 or 64 hex characters of entropy (12 or 24 words).
pub fn from_hex(text: &str) -> Result<Entropy, String> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let v = hex::decode(&t).map_err(|_| "that is not hex".to_string())?;
    check_len(v.len()).map_err(|_| format!("entropy is 32, 40, 48, 56 or 64 hex characters (12 to 24 words); that is {}", t.len()))?;
    Ok(Zeroizing::new(v))
}

/// The compressed public key of a secret, hex — what the card must report back after an import.
pub fn pubkey_of(secret: &Secret) -> String {
    let sk = SecretKey::from_slice(&secret[..]).expect("checked");
    hex::encode(PublicKey::from_secret_key(&Secp256k1::signing_only(), &sk).serialize())
}

/// The uncompressed point (04 ‖ x ‖ y), which the card's import template carries beside the scalar.
pub fn point_of(secret: &Secret) -> [u8; 65] {
    let sk = SecretKey::from_slice(&secret[..]).expect("checked");
    PublicKey::from_secret_key(&Secp256k1::signing_only(), &sk).serialize_uncompressed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_bip39_length_round_trips_and_the_key_is_one_tagged_hash_of_its_entropy() {
        for n in LENGTHS {
            let bytes = n * 11 * 32 / 33 / 8;
            let e = Zeroizing::new((0..bytes as u8).map(|i| i.wrapping_mul(37).wrapping_add(n as u8)).collect::<Vec<u8>>());
            let w = words_of(&e); assert_eq!(w.split_whitespace().count(), n);
            assert_eq!(&from_words(&w).unwrap()[..], &e[..]);
            let mut h = Sha256::new(); h.update(TAG); h.update(&e[..]);
            assert_eq!(&secret_of(&e).unwrap()[..], &h.finalize()[..]);
        }
        // the BIP39 vector: 128 zero bits are "abandon ×11 about"
        assert_eq!(&from_words(&"abandon ".repeat(11).trim().to_string().clone()).unwrap_err()[..7], "a phras");
        assert_eq!(&from_words("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap()[..], &[0u8; 16][..]);
        assert!(from_words("abandon abandon abandon about").is_err());
        for (n, b) in [(12, 16), (15, 20), (18, 24), (21, 28), (24, 32)] { let e = from_app(n).unwrap(); assert_eq!(e.len(), b); assert_eq!(words_of(&e).split_whitespace().count(), n); } assert!(from_app(13).is_err());
        assert_eq!(from_hex(&"ab".repeat(16)).unwrap().len(), 16); assert_eq!(from_hex(&"ab".repeat(20)).unwrap().len(), 20); assert!(from_hex(&"ab".repeat(18)).is_err_and(|e| e.contains("12")));
        let s = secret_of(&from_hex("1e99423a4ed27608a15a2616a2b0e9e52ced330ac530edcc32c8ffc6a526aedd").unwrap()).unwrap();
        assert_eq!(pubkey_of(&s).len(), 66); assert_eq!(point_of(&s)[0], 4);
    }

    #[test]
    fn a_phrase_drawn_one_word_short_lists_every_valid_ending() {
        let e = from_app(24).unwrap(); let w = words_of(&e);
        let first23: Vec<&str> = w.split_whitespace().take(23).collect();
        let ends = completions(&first23.join(" ")).unwrap();
        assert_eq!(ends.len(), 8);
        assert!(ends.contains(&w.split_whitespace().last().unwrap().to_string()), "the real 24th word is among them");
        for x in &ends { assert!(from_words(&format!("{} {x}", first23.join(" "))).is_ok(), "{x}"); }
        let e = from_app(12).unwrap(); let w = words_of(&e);
        let first11: Vec<&str> = w.split_whitespace().take(11).collect();
        let ends = completions(&first11.join(" ")).unwrap();
        assert_eq!(ends.len(), 128, "11 drawn words leave 7 free bits");
        assert!(ends.contains(&w.split_whitespace().last().unwrap().to_string()));
        assert!(completions("abandon").is_err()); assert!(completions(&"zzzz ".repeat(23)).is_err());
    }

    #[test]
    fn dice_rolls_hash_the_way_coldcard_documents_so_the_words_match_a_coldcards() {
        let rolls = "1234561234561234561234561234561234561234561234561234561234561234561234561234561234561234561234561234"; // 100
        let e = from_dice(rolls).unwrap();
        assert_eq!(&e[..], &Sha256::digest(rolls.as_bytes())[..]);
        assert_eq!(words_of(&e).split_whitespace().count(), 24);
        assert_eq!(&from_dice(&rolls.chars().collect::<Vec<_>>().chunks(10).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ")).unwrap()[..], &e[..], "spaces are ignored");
        assert!(from_dice(&rolls[..98]).unwrap_err().contains("99"));
        assert!(from_dice("1234567").is_err());
    }
}
