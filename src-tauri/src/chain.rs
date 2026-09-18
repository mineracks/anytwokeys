//! The chain, through an Electrum server (coins, history, broadcast) and an esplora-style
//! explorer (fee levels, links). Both default to ours; both are the user's to override.
use bitcoin::{Network, ScriptBuf, Transaction, Txid};
use electrum_client::{Client, ConfigBuilder, ElectrumApi};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Servers {
    pub electrum: String,   // e.g. ssl://electrum.mineracks.com:50002
    pub explorer: String,   // e.g. https://mempool.mineracks.com
}

pub fn defaults(network: Network) -> Servers {
    match network {
        Network::Bitcoin => Servers { electrum: "ssl://electrum.mineracks.com:50002".into(), explorer: "https://mempool.mineracks.com".into() },
        _ => Servers { electrum: "tcp://100.94.4.10:50001".into(), explorer: "https://signet.mineracks.com".into() },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Coin {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub height: u32,   // 0 = unconfirmed
}

#[derive(Debug, Clone, Serialize)]
pub struct Fees { pub slow: f64, pub medium: f64, pub fast: f64 }

pub fn client(servers: &Servers) -> Result<Client, String> {
    let cfg = ConfigBuilder::new().timeout(Some(20)).validate_domain(true).build();
    Client::from_config(&servers.electrum, cfg).map_err(|e| format!("electrum {}: {e}", servers.electrum))
}

pub fn coins(servers: &Servers, spk: &ScriptBuf) -> Result<Vec<Coin>, String> {
    let c = client(servers)?;
    let list = c.script_list_unspent(spk.as_script()).map_err(|e| format!("listunspent: {e}"))?;
    Ok(list.into_iter().map(|u| Coin { txid: u.tx_hash.to_string(), vout: u.tx_pos as u32, value: u.value, height: u.height as u32 }).collect())
}

pub fn height(servers: &Servers) -> Result<u32, String> {
    let c = client(servers)?;
    let h = c.block_headers_subscribe().map_err(|e| format!("headers: {e}"))?;
    Ok(h.height as u32)
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryItem { pub txid: String, pub height: i32 }

pub fn history(servers: &Servers, spk: &ScriptBuf) -> Result<Vec<HistoryItem>, String> {
    let c = client(servers)?;
    let h = c.script_get_history(spk.as_script()).map_err(|e| format!("history: {e}"))?;
    Ok(h.into_iter().map(|i| HistoryItem { txid: i.tx_hash.to_string(), height: i.height }).collect())
}

pub fn broadcast(servers: &Servers, tx: &Transaction) -> Result<Txid, String> {
    let c = client(servers)?;
    c.transaction_broadcast(tx).map_err(|e| format!("broadcast refused: {e}"))
}

pub fn fees(servers: &Servers) -> Result<Fees, String> {
    let url = format!("{}/api/v1/fees/recommended", servers.explorer.trim_end_matches('/'));
    let http = reqwest::blocking::Client::builder().user_agent("anytwokeys/0.1").timeout(std::time::Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    let v: serde_json::Value = http.get(&url).send().map_err(|e| format!("fees: {e}"))?.json().map_err(|e| format!("fees: {e}"))?;
    let g = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(1.0);
    Ok(Fees { slow: g("hourFee").max(1.0), medium: g("halfHourFee").max(1.0), fast: g("fastestFee").max(1.0) })
}

pub fn tx_link(servers: &Servers, txid: &str) -> String {
    format!("{}/tx/{txid}", servers.explorer.trim_end_matches('/'))
}
