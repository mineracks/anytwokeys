//! The chain cache: what the chain has told this app that can never change, kept so it is asked for once.
//!
//! A confirmed transaction is fixed by its id, so once fetched it is kept here and every later screen reads it from
//! disk instead of the network. SQLite, encrypted whole-file by SQLCipher (pages, indexes and all) under a key derived
//! from the store key: the file opens only while the app is unlocked, and closes when it locks.
//!
//! It is a CACHE. Everything in it can be fetched from the chain again, so a file that will not open under the key —
//! damaged, or from another key — is set aside and a fresh one started: never trusted, never fatal. Every transaction
//! read back is hashed and must match the id it was asked for.
use bitcoin::{consensus, Transaction, Txid};
use hkdf::Hkdf;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static DB: Mutex<Option<Connection>> = Mutex::new(None);
const SCHEMA: i64 = 1;

fn path() -> PathBuf { crate::lockbox::dir().join("chain.db") }

fn db_key(store_key: &[u8; 32]) -> [u8; 32] {
    let mut k = [0u8; 32];
    Hkdf::<Sha256>::new(Some(b"anytwokeys chain cache"), store_key).expand(b"sqlcipher key v1", &mut k).expect("32 bytes is a valid HKDF-SHA256 length");
    k
}

/// Open (or create) the cache at `p` under `key`. A file that will not open under it is moved aside and a new one begun.
pub fn open_at(p: &Path, key: &[u8; 32]) -> Result<Connection, String> {
    let attempt = |p: &Path| -> rusqlite::Result<Connection> {
        let c = Connection::open(p)?;
        c.execute_batch(&format!("PRAGMA key = \"x'{}'\";", hex::encode(key)))?;
        // the first read is what proves the key: SQLCipher says "file is not a database" otherwise
        c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))?;
        c.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;
            CREATE TABLE IF NOT EXISTS tx (net TEXT NOT NULL, txid TEXT NOT NULL, raw BLOB NOT NULL, PRIMARY KEY (net, txid)) WITHOUT ROWID;
            CREATE TABLE IF NOT EXISTS transfer_cache (id INTEGER PRIMARY KEY, body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS spk (net TEXT NOT NULL, spk BLOB NOT NULL, status TEXT NOT NULL, history TEXT NOT NULL, coins TEXT, PRIMARY KEY (net, spk)) WITHOUT ROWID;")?;
        let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v == 0 { c.execute_batch(&format!("PRAGMA user_version = {SCHEMA};"))?; }
        Ok(c)
    };
    match attempt(p) {
        Ok(c) => Ok(c),
        Err(_) if p.exists() => {
            // a cache that cannot be read is an empty cache: kept aside for a look, not deleted, and not read again
            let aside = p.with_extension(format!("unreadable-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)));
            let _ = std::fs::rename(p, &aside);
            for ext in ["db-wal", "db-shm"] { let _ = std::fs::remove_file(p.with_extension(ext)); }
            attempt(p).map_err(|e| format!("the chain cache could not be started: {e}"))
        }
        Err(e) => Err(format!("the chain cache could not be started: {e}")),
    }
}

/// Run `f` on the cache. `None` when there is none to be had (locked, or it could not open): callers then simply ask
/// the network, as they would for anything not yet kept.
fn with<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
    let mut g = DB.lock().ok()?;
    if g.is_none() {
        let key = crate::lockbox::key().ok()?;
        *g = open_at(&path(), &db_key(&key)).ok();
    }
    f(g.as_ref()?).ok()
}

/// What a script's history and coins were when its status was last this: Electrum's status is a hash of the whole
/// history, so the same status means the same answer, and a refresh in which nothing moved fetches nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct Known { pub status: String, pub history: Vec<crate::chain::HistoryItem>, pub coins: Option<Vec<crate::chain::Coin>> }

pub fn spk_get(net: &str, spks: &[bitcoin::ScriptBuf]) -> Vec<Option<Known>> {
    with(|c| {
        let mut st = c.prepare_cached("SELECT status, history, coins FROM spk WHERE net = ?1 AND spk = ?2")?;
        spks.iter().map(|s| {
            let row: Option<(String, String, Option<String>)> = st.query_row(params![net, s.as_bytes()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
            Ok(row.and_then(|(status, h, c)| Some(Known { status, history: serde_json::from_str(&h).ok()?, coins: c.and_then(|c| serde_json::from_str(&c).ok()) })))
        }).collect::<rusqlite::Result<Vec<_>>>()
    }).unwrap_or_else(|| vec![None; spks.len()])
}

pub fn spk_put(net: &str, spks: &[bitcoin::ScriptBuf], known: &[Known]) {
    let _ = with(|c| {
        let tx = c.unchecked_transaction()?;
        { let mut st = tx.prepare_cached("INSERT OR REPLACE INTO spk (net, spk, status, history, coins) VALUES (?1, ?2, ?3, ?4, ?5)")?;
          for (s, k) in spks.iter().zip(known) {
              st.execute(params![net, s.as_bytes(), k.status, serde_json::to_string(&k.history).unwrap_or_default(), k.coins.as_ref().map(|c| serde_json::to_string(c).unwrap_or_default())])?;
          } }
        tx.commit()
    });
}

/// Close it: the app locked, and the key has left memory.
pub fn close() { if let Ok(mut g) = DB.lock() { *g = None; } }

fn tx_get(c: &Connection, net: &str, id: &Txid) -> rusqlite::Result<Option<Transaction>> {
    let raw: Option<Vec<u8>> = c.query_row("SELECT raw FROM tx WHERE net = ?1 AND txid = ?2", params![net, id.to_string()], |r| r.get(0)).optional()?;
    // what comes back must BE the transaction asked for; anything else is ignored and fetched again
    Ok(raw.and_then(|b| consensus::deserialize::<Transaction>(&b).ok()).filter(|t| t.compute_txid() == *id))
}

/// Kept transactions for these ids, in the same order; `None` where one is not kept.
pub fn txs(net: &str, ids: &[Txid]) -> Vec<Option<Transaction>> {
    with(|c| ids.iter().map(|id| tx_get(c, net, id)).collect::<rusqlite::Result<Vec<_>>>()).unwrap_or_else(|| vec![None; ids.len()])
}

/// Keep confirmed transactions. (Only confirmed ones: until then, a transaction's witness can still differ.)
pub fn keep_txs(net: &str, txs: &[&Transaction]) {
    let _ = with(|c| {
        let tx = c.unchecked_transaction()?;
        { let mut st = tx.prepare_cached("INSERT OR IGNORE INTO tx (net, txid, raw) VALUES (?1, ?2, ?3)")?;
          for t in txs { st.execute(params![net, t.compute_txid().to_string(), consensus::serialize(*t)])?; } }
        tx.commit()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tmp(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("a2k-db-{name}-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d.join("chain.db") }
    fn sample() -> Transaction {
        let raw = include_bytes!("../../test/fixtures/sample.psbt");
        bitcoin::psbt::Psbt::deserialize(raw).unwrap().unsigned_tx
    }
    #[test]
    fn a_scripts_status_history_and_coins_are_kept_and_come_back_by_status() {
        let p = tmp("spk"); let c = open_at(&p, &db_key(&[3; 32])).unwrap();
        let spk = bitcoin::ScriptBuf::from(vec![0x00u8, 0x14, 7, 7, 7]);
        let put = |c: &Connection, k: &Known| {
            let mut st = c.prepare("INSERT OR REPLACE INTO spk (net, spk, status, history, coins) VALUES (?1, ?2, ?3, ?4, ?5)").unwrap();
            st.execute(params!["signet", spk.as_bytes(), k.status, serde_json::to_string(&k.history).unwrap(), k.coins.as_ref().map(|x| serde_json::to_string(x).unwrap())]).unwrap();
        };
        let get = |c: &Connection| -> Option<Known> {
            let row: Option<(String, String, Option<String>)> = c.query_row("SELECT status, history, coins FROM spk WHERE net = 'signet' AND spk = ?1", params![spk.as_bytes()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().unwrap();
            row.map(|(status, h, co)| Known { status, history: serde_json::from_str(&h).unwrap(), coins: co.map(|x| serde_json::from_str(&x).unwrap()) })
        };
        let k = Known { status: "ab".into(), history: vec![crate::chain::HistoryItem { txid: "t1".into(), height: 5 }], coins: None };
        put(&c, &k); assert_eq!(get(&c), Some(k.clone()), "history kept without coins");
        let k2 = Known { coins: Some(vec![crate::chain::Coin { txid: "t1".into(), vout: 0, value: 9, height: 5 }]), ..k };
        put(&c, &k2); assert_eq!(get(&c), Some(k2), "coins kept beside it, replacing the row");
    }

    #[test]
    fn a_kept_transaction_comes_back_only_under_its_key_and_the_file_holds_nothing_readable() {
        let p = tmp("seal"); let (k1, k2) = (db_key(&[1; 32]), db_key(&[2; 32]));
        let t = sample(); let id = t.compute_txid();
        { let c = open_at(&p, &k1).unwrap(); c.execute("INSERT INTO tx (net, txid, raw) VALUES ('signet', ?1, ?2)", params![id.to_string(), consensus::serialize(&t)]).unwrap(); c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap(); }
        let bytes = std::fs::read(&p).unwrap();
        assert!(!bytes.starts_with(b"SQLite format 3"), "an encrypted file, not a plain SQLite one");
        let hex_id = id.to_string();
        assert!(!bytes.windows(hex_id.len()).any(|w| w == hex_id.as_bytes()), "the txid is not on disk in the clear");
        assert!(!bytes.windows(8).any(|w| w == &consensus::serialize(&t)[..8]), "nor the transaction's bytes");
        let c = open_at(&p, &k1).unwrap();
        assert_eq!(tx_get(&c, "signet", &id).unwrap().unwrap(), t, "the same key reads it back");
        drop(c);
        // another key cannot read it: the file is set aside and an empty cache begun — nothing is lost that the chain lacks
        let c = open_at(&p, &k2).unwrap();
        assert!(tx_get(&c, "signet", &id).unwrap().is_none());
        assert!(std::fs::read_dir(p.parent().unwrap()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains("unreadable")), "the unreadable file is kept aside, not deleted");
    }
    #[test]
    fn a_row_that_is_not_the_transaction_asked_for_is_ignored() {
        let p = tmp("mismatch"); let k = db_key(&[3; 32]);
        let c = open_at(&p, &k).unwrap();
        let t = sample(); let wrong: Txid = "aa".repeat(32).parse().unwrap();
        c.execute("INSERT INTO tx (net, txid, raw) VALUES ('signet', ?1, ?2)", params![wrong.to_string(), consensus::serialize(&t)]).unwrap();
        assert!(tx_get(&c, "signet", &wrong).unwrap().is_none(), "bytes under the wrong id are not handed out as that transaction");
    }
}

// Derived history evidence stays in the encrypted cache, never in the user-answer store.
pub fn transfer_cache_get() -> Option<String> { with(|c| c.query_row("SELECT body FROM transfer_cache WHERE id=1", [], |r| r.get(0)).optional()).flatten() }
pub fn transfer_cache_put(body: &str) { let _=with(|c| c.execute("INSERT OR REPLACE INTO transfer_cache (id,body) VALUES (1,?1)", [body])); }
