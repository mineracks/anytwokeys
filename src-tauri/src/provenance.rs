//! Provenance: who can link a coin to its owner.
//!
//! KYC is one answer to that question, so the status is a small scale rather than a yes or no: `kyc` (a regulated
//! party holds the owner's identity against this coin), `known` (a person or a service knows it is theirs, with no
//! regulated record), `private` (nothing in the owner's own records links it to them), `unknown` (not classified
//! yet — a work queue, not a fourth kind). A method says how the sats arrived; a counterparty says who was on the
//! other side. Disposal (disposal_set) classifies how sats LEFT; this classifies how they ARRIVED.
//!
//! Everything here is answered from the owner's own records: marks they set on a coin, an address, a transaction or
//! a vault, and the statements of the exchanges and custodians they imported. No chain-analysis service is asked,
//! and no coin is ever called clean or dirty.
//!
//! Inheritance is worst-wins (kyc > known > private) and runs two ways:
//! - FORWARD through the graph: what a spend makes (change, a move to another of the owner's vaults) inherits the
//!   worst status of the coins it spent; a coin paid to an address the owner marked as an exchange's takes that mark.
//! - ACROSS a cluster: coins spent together were controlled together, and the outputs of a transaction whose inputs
//!   were ALL the owner's are linkable to those inputs. So a status of `known` or better flows through the whole
//!   cluster, backwards in time too: deposit one change output to an exchange and its siblings become linkable.
//!   `private` is the ABSENCE of a link and never spreads; `unknown` never wins.
//! A coin whose status came from the cluster rather than its own record is `linked`, and its trail says through what.
//! The trail is first-class: a status can change without the owner touching the coin, and the screen must say why.
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

pub const STATUSES: [&str; 4] = ["kyc", "known", "private", "unknown"];
pub const METHODS: [&str; 11] = ["exchange", "atm", "p2p", "mined", "earned", "gift", "inheritance", "coinjoin", "channel", "swap", "transfer"];
pub const POLICIES: [&str; 3] = ["none", "warn", "refuse"];

/// kyc > known > private > unknown. Only a rank of 2 or more is a LINK (someone knows), and only links spread.
pub fn rank(status: &str) -> u8 { match status { "kyc" => 3, "known" => 2, "private" => 1, _ => 0 } }
/// The status a method implies when the person gives only the method. None = the method says nothing by itself.
pub fn status_of_method(method: &str) -> Option<&'static str> {
    match method {
        "exchange" | "atm" => Some("kyc"),
        "earned" | "gift" | "inheritance" | "swap" | "channel" => Some("known"),
        "mined" | "p2p" | "coinjoin" => Some("private"),
        _ => None,
    }
}
pub fn word(status: &str) -> &'static str { match status { "kyc" => "KYC", "known" => "known", "private" => "private", _ => "unclassified" } }

/// What the person (or a rule) said about one record: a coin, an address, a transaction, or a vault's default.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Mark {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub counterparty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub note: Option<String>,
    #[serde(default)] pub t: u64,
    /// "you" for a mark the person set; "rule:<name>" for one the app derived (an exchange's statement)
    #[serde(default)] pub by: String,
}
/// The mark kept in a record's `more` map under "provenance", if it is one this build understands.
pub fn mark_in(more: &serde_json::Map<String, serde_json::Value>) -> Option<Mark> {
    more.get("provenance").and_then(|p| serde_json::from_value::<Mark>(p.clone()).ok()).filter(|m| STATUSES.contains(&m.status.as_str()) && m.status != "unknown")
}

/// One input of a transaction: its outpoint, and — when it is one of the owner's coins — its sats and its vault.
#[derive(Debug, Clone, Default)]
pub struct In { pub op: String, pub sats: u64, pub vault: Option<String>, /// the address the spent output sat at, when its transaction is known (records.rs looks it up)
    pub addr: Option<String> }
/// One output: outpoint, sats, the owner's vault it paid (None = someone else's), and its address when it has one.
#[derive(Debug, Clone, Default)]
pub struct Out { pub op: String, pub sats: u64, pub vault: Option<String>, pub addr: Option<String> }
/// A transaction as the engine needs it. `time` 0 = not in a block yet (sorted last).
#[derive(Debug, Clone, Default)]
pub struct Tx { pub id: String, pub time: u32, pub ins: Vec<In>, pub outs: Vec<Out> }

/// Every mark the engine reads, gathered from the vaults' records.
#[derive(Debug, Clone, Default)]
pub struct Marks {
    /// by outpoint
    pub coin: HashMap<String, Mark>,
    /// by address — the owner's own (a receive address given to a counterparty) or someone else's (an exchange's deposit address)
    pub addr: HashMap<String, Mark>,
    /// by transaction id, set by the person
    pub tx: HashMap<String, Mark>,
    /// by transaction id, derived by a rule (an exchange's or custodian's statement lists the transaction)
    pub rule: HashMap<String, Mark>,
    /// by vault name: the vault's default for coins nothing else classifies
    pub vault: HashMap<String, Mark>,
}

/// What the engine concluded about one of the owner's coins.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Derived {
    pub vault: String,
    pub sats: u64,
    /// the coin's status after the cluster pass — what the screens show
    pub status: String,
    /// its status from its own record and lineage alone, before the cluster pass
    pub own: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub counterparty: Option<String>,
    /// "you" · "rule:statement" · "inherited" · "vault" · "" (nothing said)
    pub by: String,
    /// why, in order: the first line is the source, later lines are what moved it
    pub trail: Vec<String>,
    /// the status came through the cluster, not the coin's own record or lineage
    pub linked: bool,
    pub cluster: u32,
    pub cluster_size: u32,
    /// the coin has been spent (it is the input of a later transaction)
    pub spent: bool,
    /// when it was spent (block time of that transaction; 0 = unspent or not yet in a block), so a screen can draw
    /// holdings by status over time
    pub spent_at: u32,
    /// the owner froze it (BIP 329 `spendable: false`): kept out of Send, Combine and Split until thawed
    pub frozen: bool,
    pub time: u32,
    /// the address it sits at, when the script has one — so a screen can group coins by address without the vault loaded
    #[serde(skip_serializing_if = "Option::is_none")] pub addr: Option<String>,
    /// public records against the addresses its transactions touched (records.rs): a source listed one on a date —
    /// shown as that, never as a verdict; the status above never moves because of a record
    #[serde(skip_serializing_if = "Vec::is_empty")] pub records: Vec<crate::records::Hit>,
}

fn short(op: &str) -> String {
    let (id, n) = op.split_once(':').unwrap_or((op, ""));
    if id.len() > 12 { format!("{}…{}:{}", &id[..8], &id[id.len() - 4..], n) } else { op.to_string() }
}
fn short_addr(a: &str) -> String { if a.len() > 16 { format!("{}…{}", &a[..8], &a[a.len() - 6..]) } else { a.to_string() } }
/// "3 Mar 2024" from unix seconds, without a calendar crate (civil-from-days, Howard Hinnant).
pub fn day(t: u32) -> String {
    if t == 0 { return "not yet in a block".into(); }
    let z = t as i64 / 86400 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    const MON: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d} {} {y}", MON[(m - 1) as usize])
}

struct Uf { p: Vec<usize> }
impl Uf {
    fn new(n: usize) -> Uf { Uf { p: (0..n).collect() } }
    fn find(&mut self, i: usize) -> usize { let mut i = i; while self.p[i] != i { self.p[i] = self.p[self.p[i]]; i = self.p[i]; } i }
    fn union(&mut self, a: usize, b: usize) { let (a, b) = (self.find(a), self.find(b)); if a != b { self.p[a.max(b)] = a.min(b); } }
}

/// The whole derivation: every coin of the owner's that the transactions make, with its status and why.
pub fn derive(txs: &[Tx], marks: &Marks) -> HashMap<String, Derived> {
    let mut order: Vec<&Tx> = txs.iter().collect();
    let key = |t: &Tx| if t.time == 0 { u32::MAX } else { t.time };
    order.sort_by(|a, b| key(a).cmp(&key(b)).then_with(|| a.id.cmp(&b.id)));
    let mut d: HashMap<String, Derived> = HashMap::new();
    let apply = |e: &mut Derived, m: &Mark, why: String| {
        e.status = m.status.clone(); e.own = m.status.clone();
        e.method = m.method.clone().or_else(|| e.method.take());
        if m.counterparty.is_some() { e.counterparty = m.counterparty.clone(); }
        e.by = if m.by.is_empty() { "you".into() } else { m.by.clone() };
        e.trail.push(why);
    };
    for tx in &order {
        let ours_in: Vec<&In> = tx.ins.iter().filter(|i| i.vault.is_some()).collect();
        let spends = !ours_in.is_empty();
        // 1. a destination mark lands on the coins spent: paid to an exchange, says the person or its statement
        if spends {
            let dest: Option<(Mark, String)> = if let Some(m) = marks.tx.get(&tx.id) { Some((m.clone(), "you marked this transaction".into())) }
                else if let Some(m) = marks.rule.get(&tx.id) { Some((m.clone(), rule_why(m))) }
                else { tx.outs.iter().filter(|o| o.vault.is_none()).find_map(|o| o.addr.as_ref().and_then(|a| marks.addr.get(a).map(|m| (m.clone(), format!("paid to {}, an address you marked{}", short_addr(a), m.counterparty.as_ref().map(|c| format!(" as {c}’s")).unwrap_or_default()))))) };
            if let Some((m, why)) = dest {
                for i in &ours_in {
                    let e = d.entry(i.op.clone()).or_insert_with(|| Derived { vault: i.vault.clone().unwrap_or_default(), sats: i.sats, status: "unknown".into(), own: "unknown".into(), trail: vec!["arrived before this record begins".into()], ..Default::default() });
                    if rank(&m.status) > rank(&e.own) {
                        let was = e.own.clone();
                        e.own = m.status.clone(); e.status = m.status.clone();
                        if m.counterparty.is_some() { e.counterparty = m.counterparty.clone(); }
                        e.trail.push(format!("{} since {}: {}{}", word(&m.status), day(tx.time), why, if rank(&was) > 0 { format!(" (was {})", word(&was)) } else { String::new() }));
                    }
                }
            }
        }
        // 2. each of the owner's outputs: its own mark, its address's, its transaction's, a statement's, its lineage, its vault's
        for o in tx.outs.iter().filter(|o| o.vault.is_some()) {
            let vault = o.vault.clone().unwrap_or_default();
            let mut e = Derived { vault: vault.clone(), sats: o.sats, status: "unknown".into(), own: "unknown".into(), time: tx.time, addr: o.addr.clone(), ..Default::default() };
            if let Some(m) = marks.coin.get(&o.op) { apply(&mut e, m, "set by you on this coin".into()); }
            else if let Some(m) = o.addr.as_ref().and_then(|a| marks.addr.get(a)) { apply(&mut e, m, format!("from its address: you marked {}{}", short_addr(o.addr.as_deref().unwrap_or("")), m.counterparty.as_ref().map(|c| format!(" as {c}’s")).unwrap_or_default())); }
            else if !spends && marks.tx.contains_key(&tx.id) { let m = &marks.tx[&tx.id]; apply(&mut e, m, "you marked this transaction".into()); }
            else if !spends && marks.rule.contains_key(&tx.id) { let m = &marks.rule[&tx.id]; apply(&mut e, m, rule_why(m)); }
            else if spends {
                let from: Vec<&Derived> = ours_in.iter().filter_map(|i| d.get(&i.op)).collect();
                let top = from.iter().max_by_key(|x| rank(&x.status)).copied();
                let same_vault = ours_in.iter().all(|i| i.vault.as_deref() == Some(vault.as_str()));
                match top {
                    Some(x) if rank(&x.status) > 0 || from.iter().any(|f| f.by != "") => {
                        e.status = x.status.clone(); e.own = x.status.clone();
                        e.counterparty = x.counterparty.clone();
                        e.method = if same_vault { x.method.clone() } else { Some("transfer".into()) };
                        e.by = "inherited".into();
                        let n = from.len();
                        let what = if same_vault { "change" } else { "moved between your vaults" };
                        e.trail.push(format!("{what}: inherits from the {} spent on {} — {} ({})", if n == 1 { "coin".to_string() } else { format!("{n} coins") }, day(tx.time), short(&ours_in.iter().find(|i| d.get(&i.op).map(|f| f.status == x.status).unwrap_or(false)).map(|i| i.op.as_str()).unwrap_or("")), x.trail.first().cloned().unwrap_or_default()));
                    }
                    _ => {
                        if let Some(m) = marks.vault.get(&vault) { apply(&mut e, m, "the vault’s default".into()); e.by = "vault".into(); }
                        else { e.trail.push(format!("{}: the coins spent on {} have no record yet", if same_vault { "change" } else { "moved between your vaults" }, day(tx.time))); }
                    }
                }
            }
            else if let Some(m) = marks.vault.get(&vault) { apply(&mut e, m, "the vault’s default".into()); e.by = "vault".into(); }
            else { e.trail.push("not yet classified".into()); }
            d.insert(o.op.clone(), e);
        }
        for i in &ours_in { if let Some(e) = d.get_mut(&i.op) { e.spent = true; e.spent_at = tx.time; } }
    }
    // 3. clusters: coins spent together; and when every input was the owner's, the outputs with them
    let ops: Vec<String> = { let mut v: Vec<String> = d.keys().cloned().collect(); v.sort(); v };
    let at: HashMap<&str, usize> = ops.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
    let mut uf = Uf::new(ops.len());
    for tx in &order {
        let ins: Vec<usize> = tx.ins.iter().filter(|i| i.vault.is_some()).filter_map(|i| at.get(i.op.as_str()).copied()).collect();
        for w in ins.windows(2) { uf.union(w[0], w[1]); }
        let all_ours = !tx.ins.is_empty() && tx.ins.iter().all(|i| i.vault.is_some());
        if all_ours {
            let outs: Vec<usize> = tx.outs.iter().filter(|o| o.vault.is_some()).filter_map(|o| at.get(o.op.as_str()).copied()).collect();
            if let Some(&first) = ins.first() { for o in outs { uf.union(first, o); } }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..ops.len() { groups.entry(uf.find(i)).or_default().push(i); }
    let mut roots: Vec<usize> = groups.keys().copied().collect(); roots.sort();
    for (n, root) in roots.iter().enumerate() {
        let members = &groups[root];
        let top = members.iter().filter_map(|&i| d.get(ops[i].as_str()).map(|e| (i, rank(&e.own), e.own.clone()))).filter(|(_, r, _)| *r >= 2).max_by_key(|(_, r, _)| *r);
        let top_line = top.as_ref().map(|(i, _, s)| { let e = &d[ops[*i].as_str()]; (ops[*i].clone(), s.clone(), e.trail.first().cloned().unwrap_or_default()) });
        for &i in members {
            let e = d.get_mut(ops[i].as_str()).unwrap();
            e.cluster = n as u32 + 1; e.cluster_size = members.len() as u32;
            if let Some((op, s, why)) = &top_line {
                if rank(s) > rank(&e.own) {
                    e.status = s.clone(); e.linked = true;
                    e.trail.push(format!("linked: {} is {} ({}), and the chain shows the two were controlled together", short(op), word(s), why));
                }
            }
        }
    }
    d
}

/// Why a rule mark applies, in words: the rule's own sentence when it brought one (`note` on a `rule:` mark), else
/// the statement wording every rule used to share.
fn rule_why(m: &Mark) -> String {
    match (&m.note, m.by.starts_with("rule:")) {
        (Some(n), true) if !n.is_empty() => n.clone(),
        _ => format!("{}’s statement lists this transaction", m.counterparty.clone().unwrap_or_else(|| "the custodian".into())),
    }
}

/// A vault that is not coins on the bitcoin chain, as the bucket engine needs it: what kind of thing holds the
/// balance, the person's default for the vault, who the counterparty is, and the places where its money touched the
/// chain — a node's funding (deposits and channel opens, by txid), a Liquid wallet's peg-ins (by the bitcoin outpoint
/// claimed) and swaps (by time). Everything else the engine knows comes from `derive`.
#[derive(Debug, Clone, Default)]
pub struct OffVault {
    pub name: String,
    /// "exchange" · "statement" · "spark" · "node" · "liquid"
    pub kind: String,
    pub default: Option<String>,
    pub counterparty: Option<String>,
    /// a node's chain deposits and channel opens: (txid, time)
    pub funded_by: Vec<(String, u32)>,
    /// a Liquid wallet's peg-ins: (bitcoin outpoint claimed, time)
    pub pegins: Vec<(String, u32)>,
    /// a Liquid wallet's swaps: time of each
    pub swaps: Vec<u32>,
}

/// Who can link a balance that is not coins on the chain: one bucket per vault, with its trail. Worst wins here too.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Bucket {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub counterparty: Option<String>,
    /// "vault" · "rule:<kind>" · "" (nothing said)
    pub by: String,
    pub trail: Vec<String>,
}

/// The rules for balances off the chain, each said in the trail:
/// - an exchange balance is KYC (the exchange holds the owner's identity); a custodian's statement and a Spark wallet
///   are known (the custodian, or the service provider, sees every payment);
/// - a Lightning node is at least known (its public key is a public identity, its channels are on the chain), and
///   it inherits the worst status of the coins that FUNDED it: a channel opened, or the node's wallet paid, from a
///   KYC coin of one of the owner's vaults ties the node to that identity;
/// - a Liquid wallet: a peg-in links the L-BTC to the bitcoin it came from, so it inherits that coin's status; a swap
///   provider sees both sides, so a wallet that swapped is at least known; otherwise it is unclassified until said.
/// The vault's own default, set in Vault settings, comes before the rule for its kind — and a funding or a peg-in of
/// a WORSE status still moves it, with the trail saying so, because the chain shows the link whatever was set.
pub fn off_chain(vaults: &[OffVault], txs: &[Tx], coins: &HashMap<String, Derived>) -> HashMap<String, Bucket> {
    let by_id: HashMap<&str, &Tx> = txs.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut out = HashMap::new();
    for v in vaults {
        let own = v.default.as_ref().filter(|d| STATUSES.contains(&d.as_str()) && *d != "unknown");
        let mut b = match (own, v.kind.as_str()) {
            (Some(d), _) => Bucket { status: d.clone(), by: "vault".into(), trail: vec![format!("{}: this vault’s default, set in Vault settings", word(d))], ..Default::default() },
            (None, "exchange") => Bucket { status: "kyc".into(), method: Some("exchange".into()), counterparty: v.counterparty.clone(), by: "rule:exchange".into(), trail: vec![format!("KYC: {} holds your identity against this balance", v.counterparty.as_deref().unwrap_or("the exchange"))] },
            (None, "statement") => Bucket { status: "known".into(), method: Some("transfer".into()), counterparty: v.counterparty.clone(), by: "rule:statement".into(), trail: vec![format!("Known: {} keeps this balance for you and knows it is yours", v.counterparty.as_deref().unwrap_or("the custodian"))] },
            (None, "spark") => Bucket { status: "known".into(), method: Some("channel".into()), counterparty: Some("the Spark service provider".into()), by: "rule:spark".into(), trail: vec!["Known: the Spark service provider sees every payment in and out of this wallet, and your name if you gave the app one".into()] },
            (None, "node") => Bucket { status: "known".into(), method: Some("channel".into()), counterparty: v.counterparty.clone(), by: "rule:node".into(), trail: vec!["Known: a node’s public key is a public identity, and its channels are on the chain".into()] },
            (None, "liquid") => Bucket { status: "unknown".into(), by: String::new(), trail: vec!["Unclassified: nothing in your records says who can link this wallet to you yet".into()], ..Default::default() },
            (None, _) => Bucket { status: "unknown".into(), trail: vec!["not yet classified".into()], ..Default::default() },
        };
        if v.kind == "exchange" || v.kind == "statement" || v.kind == "spark" { out.insert(v.name.clone(), b); continue; }
        // the chain's word: what funded the node, what was pegged in — the worst of them wins over whatever was said
        let mut links: Vec<(u8, String, u32, String)> = Vec::new();   // (rank, status, time, why)
        for (txid, time) in &v.funded_by {
            let Some(t) = by_id.get(txid.as_str()) else { continue };
            for i in t.ins.iter().filter(|i| i.vault.is_some()) {
                let Some(c) = coins.get(&i.op) else { continue };
                if rank(&c.status) >= 2 { links.push((rank(&c.status), c.status.clone(), *time, format!("funded on {} from {} of {}, which is {} ({})", day(*time), short(&i.op), i.vault.as_deref().unwrap_or(""), word(&c.status), c.trail.first().cloned().unwrap_or_default()))); }
            }
        }
        for (op, time) in &v.pegins {
            match coins.get(op) {
                Some(c) if rank(&c.status) >= 2 => links.push((rank(&c.status), c.status.clone(), *time, format!("pegged in on {} from {} of {}, which is {} ({})", day(*time), short(op), c.vault, word(&c.status), c.trail.first().cloned().unwrap_or_default()))),
                Some(c) if rank(&c.status) == 1 => b.trail.push(format!("pegged in on {} from {} of {}, a private coin: the peg-in itself links no one to you", day(*time), short(op), c.vault)),
                _ => {}
            }
        }
        if v.kind == "liquid" && !v.swaps.is_empty() {
            let first = v.swaps.iter().copied().min().unwrap_or(0);
            links.push((2, "known".into(), first, format!("swapped on {}: a swap provider sees both sides of a trade{}", day(first), if v.swaps.len() > 1 { format!(" ({} swaps)", v.swaps.len()) } else { String::new() })));
            if b.method.is_none() { b.method = Some("swap".into()); b.counterparty = Some("the swap provider".into()); }
        }
        links.sort_by(|a, c| c.0.cmp(&a.0).then(a.2.cmp(&c.2)));
        if let Some((r, status, _, why)) = links.first() {
            if *r > rank(&b.status) {
                let was = b.status.clone();
                b.status = status.clone();
                b.trail.push(format!("{} since the chain shows it: {}{}", word(status), why, if rank(&was) > 0 { format!(" (was {})", word(&was)) } else { String::new() }));
                if b.by.is_empty() { b.by = format!("rule:{}", v.kind); }
            } else if *r == rank(&b.status) && v.kind == "liquid" && b.by.is_empty() { b.by = "rule:liquid".into(); b.trail.push(why.clone()); }
            else if *r == rank(&b.status) { b.trail.push(format!("and the chain agrees: {why}")); }
        }
        out.insert(v.name.clone(), b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m(status: &str) -> Mark { Mark { status: status.into(), by: "you".into(), ..Default::default() } }
    fn mc(status: &str, method: &str, cp: &str) -> Mark { Mark { status: status.into(), method: Some(method.into()), counterparty: Some(cp.into()), by: "you".into(), ..Default::default() } }
    fn ours(op: &str, sats: u64, vault: &str) -> In { In { op: op.into(), sats, vault: Some(vault.into()), addr: None } }
    fn theirs(op: &str) -> In { In { op: op.into(), sats: 0, vault: None, addr: None } }
    fn out(txid: &str, n: u32, sats: u64, vault: Option<&str>, addr: &str) -> Out { Out { op: format!("{txid}:{n}"), sats, vault: vault.map(str::to_string), addr: Some(addr.into()) } }
    fn tx(id: &str, time: u32, ins: Vec<In>, outs: Vec<Out>) -> Tx { Tx { id: id.into(), time, ins, outs } }

    #[test]
    fn a_statement_rule_classifies_the_deposit_and_its_change_inherits() {
        // the exchange paid us (its statement lists "dep"), then we spent part of it to a shop with change
        let txs = vec![
            tx("dep", 1_700_000_000, vec![theirs("x:0")], vec![out("dep", 0, 100_000, Some("Cold"), "bc1ours1"), out("dep", 1, 5, None, "bc1theirs")]),
            tx("pay", 1_700_100_000, vec![ours("dep:0", 100_000, "Cold")], vec![out("pay", 0, 60_000, None, "bc1shop"), out("pay", 1, 39_000, Some("Cold"), "bc1ours2")]),
        ];
        let mut marks = Marks::default();
        marks.rule.insert("dep".into(), Mark { status: "kyc".into(), method: Some("exchange".into()), counterparty: Some("Bitaroo".into()), by: "rule:statement".into(), ..Default::default() });
        let d = derive(&txs, &marks);
        let a = &d["dep:0"]; assert_eq!((a.status.as_str(), a.method.as_deref(), a.counterparty.as_deref(), a.spent, a.linked), ("kyc", Some("exchange"), Some("Bitaroo"), true, false));
        assert_eq!((a.time, a.spent_at), (1_700_000_000, 1_700_100_000), "a coin knows when it came and when it went");
        assert!(a.trail[0].contains("Bitaroo’s statement"), "{:?}", a.trail);
        let c = &d["pay:1"]; assert_eq!((c.status.as_str(), c.method.as_deref(), c.by.as_str(), c.spent), ("kyc", Some("exchange"), "inherited", false));
        assert!(c.trail[0].starts_with("change: inherits"), "{:?}", c.trail);
        assert!(!d.contains_key("pay:0"), "the shop's output is not ours");
    }

    #[test]
    fn mixing_a_private_coin_with_a_kyc_coin_links_the_private_one_and_the_change() {
        let txs = vec![
            tx("a", 100, vec![theirs("p:0")], vec![out("a", 0, 50_000, Some("V"), "addrA")]),   // private, by the person's mark
            tx("b", 200, vec![theirs("q:0")], vec![out("b", 0, 50_000, Some("V"), "addrB")]),   // kyc, by mark
            tx("mix", 300, vec![ours("a:0", 50_000, "V"), ours("b:0", 50_000, "V")], vec![out("mix", 0, 70_000, None, "shop"), out("mix", 1, 29_000, Some("V"), "addrC")]),
        ];
        let mut marks = Marks::default();
        marks.coin.insert("a:0".into(), m("private")); marks.coin.insert("b:0".into(), m("kyc"));
        let d = derive(&txs, &marks);
        assert_eq!((d["a:0"].own.as_str(), d["a:0"].status.as_str(), d["a:0"].linked), ("private", "kyc", true));
        assert!(d["a:0"].trail.last().unwrap().starts_with("linked: "), "{:?}", d["a:0"].trail);
        assert_eq!((d["mix:1"].status.as_str(), d["mix:1"].linked), ("kyc", false), "change inherits by lineage, not by linking");
        assert_eq!(d["a:0"].cluster, d["mix:1"].cluster); assert_eq!(d["a:0"].cluster_size, 3);
    }

    #[test]
    fn depositing_one_sibling_to_an_exchange_links_the_other_sibling_backwards_in_time() {
        // a split of our own private coin into X and Y; later X is paid to an address marked as Kraken's
        let txs = vec![
            tx("src", 100, vec![theirs("z:0")], vec![out("src", 0, 100_000, Some("V"), "a0")]),
            tx("split", 200, vec![ours("src:0", 100_000, "V")], vec![out("split", 0, 50_000, Some("V"), "aX"), out("split", 1, 49_000, Some("V"), "aY")]),
            tx("dep", 300, vec![ours("split:0", 50_000, "V")], vec![out("dep", 0, 49_500, None, "krakenDeposit")]),
        ];
        let mut marks = Marks::default();
        marks.coin.insert("src:0".into(), mc("private", "p2p", ""));
        marks.addr.insert("krakenDeposit".into(), mc("kyc", "exchange", "Kraken"));
        let d = derive(&txs, &marks);
        assert_eq!((d["split:0"].own.as_str(), d["split:0"].status.as_str(), d["split:0"].counterparty.as_deref()), ("kyc", "kyc", Some("Kraken")), "{:?}", d["split:0"].trail);
        assert!(d["split:0"].trail.iter().any(|l| l.contains("paid to kraken") || l.contains("paid to krakenD")), "{:?}", d["split:0"].trail);
        let y = &d["split:1"]; assert_eq!((y.own.as_str(), y.status.as_str(), y.linked, y.spent), ("private", "kyc", true, false));
        let s = &d["src:0"]; assert_eq!((s.own.as_str(), s.status.as_str(), s.linked), ("private", "kyc", true), "the parent is linkable too");
    }

    #[test]
    fn a_move_between_the_owners_vaults_carries_the_status_and_says_transfer() {
        let txs = vec![
            tx("in", 100, vec![theirs("z:0")], vec![out("in", 0, 80_000, Some("Hot"), "h0")]),
            tx("mv", 200, vec![ours("in:0", 80_000, "Hot")], vec![out("mv", 0, 79_000, Some("Cold"), "c0")]),
        ];
        let mut marks = Marks::default();
        marks.addr.insert("h0".into(), mc("known", "earned", "Acme Pty Ltd"));
        let d = derive(&txs, &marks);
        let c = &d["mv:0"];
        assert_eq!((c.vault.as_str(), c.status.as_str(), c.method.as_deref(), c.counterparty.as_deref(), c.by.as_str()), ("Cold", "known", Some("transfer"), Some("Acme Pty Ltd"), "inherited"));
        assert!(c.trail[0].starts_with("moved between your vaults"), "{:?}", c.trail);
    }

    #[test]
    fn private_does_not_spread_and_unknown_never_wins() {
        // two coins of ours spent together: one marked private, the other unclassified
        let txs = vec![
            tx("a", 100, vec![theirs("p:0")], vec![out("a", 0, 10_000, Some("V"), "a0")]),
            tx("b", 100, vec![theirs("q:0")], vec![out("b", 0, 10_000, Some("V"), "b0")]),
            tx("sp", 200, vec![ours("a:0", 10_000, "V"), ours("b:0", 10_000, "V")], vec![out("sp", 0, 19_000, None, "shop")]),
        ];
        let mut marks = Marks::default();
        marks.coin.insert("a:0".into(), m("private"));
        let d = derive(&txs, &marks);
        assert_eq!((d["b:0"].status.as_str(), d["b:0"].linked), ("unknown", false));
        assert_eq!(d["a:0"].status, "private");
        assert_eq!(d["a:0"].cluster, d["b:0"].cluster);
    }

    #[test]
    fn the_vaults_default_classifies_what_nothing_else_does_and_a_mark_beats_it() {
        let txs = vec![
            tx("a", 100, vec![theirs("p:0")], vec![out("a", 0, 10_000, Some("V"), "a0")]),
            tx("b", 100, vec![theirs("q:0")], vec![out("b", 0, 10_000, Some("V"), "b0")]),
        ];
        let mut marks = Marks::default();
        marks.vault.insert("V".into(), Mark { status: "private".into(), by: "vault".into(), ..Default::default() });
        marks.tx.insert("b".into(), mc("kyc", "exchange", "Amber"));
        let d = derive(&txs, &marks);
        assert_eq!((d["a:0"].status.as_str(), d["a:0"].by.as_str(), d["a:0"].trail[0].as_str()), ("private", "vault", "the vault’s default"));
        assert_eq!((d["b:0"].status.as_str(), d["b:0"].counterparty.as_deref()), ("kyc", Some("Amber")));
    }

    #[test]
    fn a_coin_received_from_outside_is_not_clustered_with_the_senders_other_outputs() {
        // someone paid us from a transaction with a foreign input: our coin's cluster is just itself
        let txs = vec![tx("pay", 100, vec![theirs("z:0")], vec![out("pay", 0, 1_000, Some("V"), "a0"), out("pay", 1, 2_000, Some("V"), "a1")])];
        let d = derive(&txs, &Marks::default());
        assert_ne!(d["pay:0"].cluster, d["pay:1"].cluster, "two outputs of a stranger's transaction are not linked by it");
        assert_eq!(d["pay:0"].cluster_size, 1);
    }

    #[test]
    fn days_read_right() {
        assert_eq!(day(1_709_424_000), "3 Mar 2024");   // 2024-03-03 00:00 UTC
        assert_eq!(day(0), "not yet in a block");
        assert_eq!(day(1_700_000_000), "14 Nov 2023");
    }

    #[test]
    fn a_node_funded_from_a_kyc_coin_is_kyc_and_a_liquid_wallet_follows_its_peg_in_and_its_swaps() {
        // a KYC coin (by mark) pays the node's wallet in "fund"; a private coin is pegged in to Liquid in "peg"
        let txs = vec![
            tx("k", 100, vec![theirs("x:0")], vec![out("k", 0, 100_000, Some("Cold"), "aK")]),
            tx("p", 110, vec![theirs("y:0")], vec![out("p", 0, 50_000, Some("Cold"), "aP")]),
            tx("fund", 200, vec![ours("k:0", 100_000, "Cold")], vec![out("fund", 0, 99_000, None, "nodeWallet")]),
        ];
        let mut marks = Marks::default();
        marks.coin.insert("k:0".into(), mc("kyc", "exchange", "Kraken")); marks.coin.insert("p:0".into(), m("private"));
        let coins = derive(&txs, &marks);
        let vaults = vec![
            OffVault { name: "Node".into(), kind: "node".into(), counterparty: Some("umbrel".into()), funded_by: vec![("fund".into(), 200)], ..Default::default() },
            OffVault { name: "LBTC".into(), kind: "liquid".into(), pegins: vec![("p:0".into(), 300)], ..Default::default() },
            OffVault { name: "USDt".into(), kind: "liquid".into(), swaps: vec![400, 500], ..Default::default() },
            OffVault { name: "Ex".into(), kind: "exchange".into(), counterparty: Some("Bitaroo".into()), ..Default::default() },
            OffVault { name: "Said".into(), kind: "node".into(), default: Some("private".into()), funded_by: vec![("fund".into(), 200)], ..Default::default() },
        ];
        let b = off_chain(&vaults, &txs, &coins);
        let n = &b["Node"]; assert_eq!((n.status.as_str(), n.by.as_str()), ("kyc", "rule:node"));
        assert!(n.trail[0].starts_with("Known: a node") && n.trail[1].contains("funded on") && n.trail[1].contains("which is KYC"), "{:?}", n.trail);
        let l = &b["LBTC"]; assert_eq!(l.status, "unknown", "a private peg-in links no one");
        assert!(l.trail.iter().any(|x| x.contains("a private coin")), "{:?}", l.trail);
        let u = &b["USDt"]; assert_eq!((u.status.as_str(), u.method.as_deref(), u.by.as_str()), ("known", Some("swap"), "rule:liquid"));
        assert!(u.trail.last().unwrap().contains("2 swaps"), "{:?}", u.trail);
        assert_eq!(b["Ex"].status, "kyc");
        let s = &b["Said"]; assert_eq!(s.status, "kyc", "the chain's word beats a default that says less");
        assert!(s.trail[1].contains("(was private)"), "{:?}", s.trail);
    }

    #[test]
    fn a_rule_mark_with_its_own_sentence_is_said_in_those_words() {
        let txs = vec![tx("w", 100, vec![theirs("node:0")], vec![out("w", 0, 10_000, Some("Cold"), "a0")])];
        let mut marks = Marks::default();
        marks.rule.insert("w".into(), Mark { status: "known".into(), method: Some("channel".into()), counterparty: Some("umbrel".into()), note: Some("withdrawn from your node umbrel on 1 Jan 2026".into()), by: "rule:node".into(), ..Default::default() });
        let d = derive(&txs, &marks);
        assert_eq!((d["w:0"].status.as_str(), d["w:0"].trail[0].as_str()), ("known", "withdrawn from your node umbrel on 1 Jan 2026"));
    }
}
