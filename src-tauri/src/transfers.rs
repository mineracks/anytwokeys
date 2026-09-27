//! Read-only transfer evidence. Matching happens as facts arrive, never while drawing or reading a balance.
use crate::{activity, wallet};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap};
use std::sync::Mutex;

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Fact { id: String, net: i64, time: u32, counter: String, own: bool, #[serde(default)] known: bool,
    /// for an on-chain outflow: every output value of its transaction — what it PAID, as distinct from the vault's net
    #[serde(default)] paid: Vec<u64>,
    /// where money went, and where it arrived, as (address, sats): an on-chain outflow's outputs; an inflow's own
    /// outputs; a statement row's one address and amount. The same pair on both sides of two vaults is the same payment.
    #[serde(default)] paid_to: Vec<(String, u64)>, #[serde(default)] got: Vec<(String, u64)>,
    /// a Lightning payment hash: the one identifier both ends of a Lightning payment know
    #[serde(default)] hash: String }
#[derive(Clone, Default, Serialize, Deserialize)]
struct Snapshot { name: String, network: String, identity: String, #[serde(default)] muun: bool, #[serde(default)] statement: bool, #[serde(default)] node: bool, #[serde(default)] exchange: bool, fingerprint: String, rows: Vec<Fact> }
#[derive(Clone, Default, Serialize, Deserialize)]
struct Cache { vaults: BTreeMap<String, Snapshot> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Link { pub peer: String, pub peer_id: String, pub event: String, pub tier: u8, pub forgotten: bool, pub own: bool }
#[derive(Clone, Default, Serialize)]
pub struct View { pub links: BTreeMap<String, BTreeMap<String, Link>>, pub proposals: Vec<Proposal> }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Candidate { #[serde(default)] pub exact:bool, from_key:String, to_key:String, pub from:String, pub to:String, pub from_id:String, pub to_id:String, pub out:u64, pub incoming:u64, pub at:u32, pub arrived:u32 }
impl Candidate { fn key(&self)->String { pair_key(&self.from_key,&self.from_id,&self.to_key,&self.to_id) } }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Answer { pub yes:bool, pub pair:Candidate }
#[derive(Clone, Serialize)]
pub struct Proposal { pub key:String, pub pair:Candidate, pub unique:bool }
#[derive(Default)]
struct PairCache { fingerprint:(String,String), candidates:Vec<Candidate> }
// Bucket by amount, then check bounded time. No outflow × all inflows scan.
fn candidates(ak:&str,a:&Snapshot,bk:&str,b:&Snapshot)->Vec<Candidate> {
    if a.network != b.network { return vec![]; }
    // a statement's Lightning rows can only have gone to, or come from, a wallet that speaks Lightning: Spark, or Muun
    // (whose Lightning shows on-chain). Its on-chain rows are matched by address, never here.
    if a.statement || b.statement { let (s, o) = if a.statement { (a, b) } else { (b, a) }; if s.statement && o.statement { return vec![]; } if o.identity.is_empty() && !o.muun && !o.node { return vec![]; } }
    // an exchange's withdrawals land in any wallet of the person's; two exchanges never pay each other here
    else if a.exchange || b.exchange { if a.exchange && b.exchange { return vec![]; } }
    else if (a.node || b.node) && a.identity.is_empty() && b.identity.is_empty() && !(a.muun || b.muun) && !(a.node && b.node) { return vec![]; }
    else if a.identity.is_empty() && b.identity.is_empty() { return vec![]; }
    let both_spark=!a.identity.is_empty() && !b.identity.is_empty();
    let allowance=if both_spark {0} else if a.muun || b.muun {1000} else {10000};
    // an exchange's withdrawal can take a working day or three to be sent, so its window is longer
    let after=if both_spark {3600} else if a.exchange || b.exchange {259200} else {86400};
    let mut buckets:BTreeMap<(u64,u32),Vec<&Fact>>=BTreeMap::new();
    let lightning = |s: &Snapshot, f: &Fact| !s.statement || (f.paid_to.is_empty() && f.got.is_empty());   // a statement's chain rows sit this out
    for r in &b.rows { if r.net>0 && r.time>0 && !r.own && !r.known && lightning(b, r) { buckets.entry((r.net as u64,r.time)).or_default().push(r); } }
    let mut result=vec![];
    for r in &a.rows { if r.net>=0 || r.time==0 || r.own || r.known || !lightning(a, r) {continue;}
        let out=r.net.unsigned_abs();
        // Each amount bucket is searched by time too. Range bounds limit the amount search to the fee cap.
        let mut amount=out.saturating_sub(allowance);
        while let Some((&(n,_),_))=buckets.range((amount,0)..=(out,u32::MAX)).next() {
            for (_,rs) in buckets.range((n,r.time.saturating_sub(3600))..=(n,r.time.saturating_add(after))) { for other in rs {
                if r.id == other.id { continue; } // already handled by shared-id evidence
                result.push(Candidate {exact:false,from_key:ak.into(),to_key:bk.into(),from:a.name.clone(),to:b.name.clone(),from_id:r.id.clone(),to_id:other.id.clone(),out,incoming:other.net as u64,at:r.time,arrived:other.time});
            } }
            if n>=out {break;} amount=n+1;
        }
        // an on-chain transaction paying exactly what arrived off-chain: the deposit itself, whatever change and fee
        // the vault's net also carried — so the fee allowance does not apply
        for &o in &r.paid { if o == 0 { continue; }
            for (_,rs) in buckets.range((o,r.time.saturating_sub(3600))..=(o,r.time.saturating_add(after))) { for other in rs {
                if r.id == other.id { continue; }
                result.push(Candidate {exact:true,from_key:ak.into(),to_key:bk.into(),from:a.name.clone(),to:b.name.clone(),from_id:r.id.clone(),to_id:other.id.clone(),out,incoming:other.net as u64,at:r.time,arrived:other.time});
            } }
        }
    } result
}
#[derive(Default)]
struct Index { cache: Cache, active: BTreeMap<String, String>, view: View, builds: usize, pairs:BTreeMap<String,PairCache>, answers:BTreeMap<String,Answer>, pair_builds:usize, certain:BTreeMap<String,(String,String,BTreeMap<String,Link>)> }
static INDEX: Mutex<Option<Index>> = Mutex::new(None);
fn digest(s: &[u8]) -> String { hex::encode(Sha256::digest(s)) }
fn key(v: &wallet::Vault) -> String {
    digest(format!("{}|{}", v.network, v.spark.as_ref().map(|s| s.identity.to_lowercase()).unwrap_or_else(||crate::wallet::identity(v))).as_bytes())
}
fn pair_key(a: &str, ai: &str, b: &str, bi: &str) -> String {
    let mut ends = [(a, ai), (b, bi)]; ends.sort();
    digest(&serde_json::to_vec(&ends).unwrap())
}
impl Index {
    fn reconcile(&mut self, s: &wallet::Store, changed: bool) {
        let active: BTreeMap<_, _> = s.vaults.iter().map(|v| (key(v), v.name.clone())).collect();
        if !changed && active == self.active && self.answers == s.transfer_answers { return; }
        self.answers=s.transfer_answers.clone();
        self.active = active;
        for v in &s.vaults {
            let snap = self.cache.vaults.entry(key(v)).or_default();
            snap.name = v.name.clone(); snap.network = v.network.clone();
            snap.muun = v.muun.is_some(); snap.statement = v.statement.is_some(); snap.node = v.node.is_some(); snap.exchange = v.exchange.is_some();
            snap.identity = v.spark.as_ref().map(|sp| sp.identity.to_lowercase()).unwrap_or_default();
        }
        self.build();
    }
    fn build(&mut self) {
        self.builds += 1;
        let mut view = View::default();
        let roster=digest(&serde_json::to_vec(&self.cache.vaults.iter().map(|(k,v)|(k,&v.name,&v.network,&v.identity,self.active.contains_key(k))).collect::<Vec<_>>()).unwrap());
        for (id, name) in &self.active {
            let a = &self.cache.vaults[id];
            if let Some((fp,r,links))=self.certain.get(id) { if fp==&a.fingerprint && r==&roster {view.links.insert(name.clone(),links.clone());continue;} }
            let links = view.links.entry(name.clone()).or_default();
            for r in &a.rows {
                if r.own {
                    links.insert(r.id.clone(), Link { peer: name.clone(), peer_id: r.id.clone(), event: pair_key(id,&r.id,id,&r.id), tier: 1, forgotten: false, own: true });
                    continue;
                }
                if r.counter.is_empty() { continue; }
                // Duplicate imports of the same identity are ambiguous; don't double-count them.
                let peers: Vec<_> = self.cache.vaults.iter().filter(|(k,b)| *k != id && a.network == b.network && r.counter == b.identity).collect();
                if peers.len() != 1 { continue; }
                let (bid,b) = peers[0];
                links.insert(r.id.clone(), Link { peer: b.name.clone(), peer_id: r.id.clone(), event: pair_key(id,&r.id,bid,&r.id), tier:1, forgotten:!self.active.contains_key(bid), own:false });
            }
        }
        for (id,name) in &self.active { self.certain.insert(id.clone(),(self.cache.vaults[id].fingerprint.clone(),roster.clone(),view.links[name].clone())); }
        self.by_address(&mut view);
        self.by_hash(&mut view);
        // Match only changed vault pairs. Turning cached matches into a view is a cheap index lookup.
        let keys:Vec<_>=self.active.keys().cloned().collect();
        for (n,ak) in keys.iter().enumerate() { for bk in keys.iter().skip(n+1) {
            let a=&self.cache.vaults[ak];let b=&self.cache.vaults[bk];
            let id=pair_key(ak,"",bk,"");let fp=(a.fingerprint.clone(),b.fingerprint.clone());
            if self.pairs.get(&id).map(|p| &p.fingerprint)==Some(&fp) {continue;}
            let mut found=candidates(ak,a,bk,b);found.extend(candidates(bk,b,ak,a));
            self.pairs.insert(id,PairCache{fingerprint:fp,candidates:found});self.pair_builds+=1;
        } }
        // A saved Yes applies only to the same signed amounts and still-existing rows. A changed history cannot
        // silently inherit an old answer. Forgotten peers retain a plain-language note, not an accounting link.
        for answer in self.answers.values().filter(|a|a.yes) {
            let p=&answer.pair;
            for (key,id,peer,pid,net) in [(&p.from_key,&p.from_id,&p.to_key,&p.to_id,-(p.out as i64)),(&p.to_key,&p.to_id,&p.from_key,&p.from_id,p.incoming as i64)] {
                let Some(name)=self.active.get(key) else {continue};
                let Some(a)=self.cache.vaults.get(key) else {continue};
                if !a.rows.iter().any(|r|&r.id==id && r.net==net) {continue;}
                let forgotten=!self.active.contains_key(peer);
                let peer_name=self.active.get(peer).cloned().unwrap_or_else(||if peer==&p.to_key {p.to.clone()}else{p.from.clone()});
                let valid=forgotten || self.cache.vaults.get(peer).map(|b|b.network==a.network && b.rows.iter().any(|r|&r.id==pid && r.net==if peer==&p.to_key {p.incoming as i64}else{-(p.out as i64)})).unwrap_or(false);
                if valid { view.links.entry(name.clone()).or_default().entry(id.clone()).or_insert(Link {peer:peer_name,peer_id:pid.clone(),event:p.key(),tier:3,forgotten,own:false}); }
            }
        }
        let mut pending=vec![];
        for pc in self.pairs.values() { for p in &pc.candidates {
            if !self.active.contains_key(&p.from_key) || !self.active.contains_key(&p.to_key) || self.answers.contains_key(&p.key()) {continue;}
            let from=&self.active[&p.from_key];let to=&self.active[&p.to_key];
            if view.links.get(from).and_then(|m|m.get(&p.from_id)).is_some_and(|l|!l.forgotten) || view.links.get(to).and_then(|m|m.get(&p.to_id)).is_some_and(|l|!l.forgotten) {continue;}
            let mut p=p.clone();p.from=from.clone();p.to=to.clone();pending.push(p);
        } }
        // one candidate is one proposal, however many times it was found: a pair seen twice is not two pairs, and
        // must not be counted as ambiguity against itself
        pending.sort_by_key(|p|(p.key(),!p.exact)); pending.dedup_by_key(|p|p.key());   // the exact reading of a pair wins
        let mut counts:BTreeMap<(String,String),usize>=BTreeMap::new();
        for p in &pending { *counts.entry((p.from_key.clone(),p.from_id.clone())).or_default()+=1;*counts.entry((p.to_key.clone(),p.to_id.clone())).or_default()+=1; }
        view.proposals=pending.into_iter().map(|p|Proposal{key:p.key(),unique:counts[&(p.from_key.clone(),p.from_id.clone())]==1 && counts[&(p.to_key.clone(),p.to_id.clone())]==1,pair:p}).collect();
        self.view = view;
    }
    /// Certain by address: one vault paid (address, sats) and another received exactly (address, sats), within three
    /// days — the same payment seen from both ends, whatever their ids. Only when the pair is unique both ways: two
    /// identical payments to one address are left to be proposed, not assumed. Identity links, already in place, win.
    fn by_address(&self, view: &mut View) {
        let keys: Vec<_> = self.active.keys().cloned().collect();
        for (n, ak) in keys.iter().enumerate() { for bk in keys.iter().skip(n + 1) {
            let (a, b) = (&self.cache.vaults[ak], &self.cache.vaults[bk]);
            if a.network != b.network { continue; }
            for (pk, p, rk, r) in [(ak, a, bk, b), (bk, b, ak, a)] {
                let mut got: BTreeMap<&(String, u64), Vec<&Fact>> = BTreeMap::new();
                for f in &r.rows { if f.net > 0 { for g in &f.got { got.entry(g).or_default().push(f); } } }
                let mut pairs: Vec<(&Fact, &Fact)> = Vec::new();
                for f in &p.rows { if f.net < 0 { for t in &f.paid_to { if let Some(rs) = got.get(t) { for o in rs { if o.time.abs_diff(f.time) <= 3 * 86400 { pairs.push((f, *o)); } } } } } }
                let mut out_n: BTreeMap<&str, usize> = BTreeMap::new(); let mut in_n: BTreeMap<&str, usize> = BTreeMap::new();
                for (f, o) in &pairs { *out_n.entry(&f.id).or_default() += 1; *in_n.entry(&o.id).or_default() += 1; }
                for (f, o) in pairs {
                    if out_n[f.id.as_str()] != 1 || in_n[o.id.as_str()] != 1 { continue; }
                    let event = pair_key(pk, &f.id, rk, &o.id);
                    view.links.entry(p.name.clone()).or_default().entry(f.id.clone()).or_insert(Link { peer: r.name.clone(), peer_id: o.id.clone(), event: event.clone(), tier: 2, forgotten: false, own: false });
                    view.links.entry(r.name.clone()).or_default().entry(o.id.clone()).or_insert(Link { peer: p.name.clone(), peer_id: f.id.clone(), event, tier: 2, forgotten: false, own: false });
                }
            }
        } }
    }
    /// Certain by payment hash: a Lightning payment one vault made and another received carry the same hash — the
    /// same payment, whatever else the two records say. Unique both ways, as by_address; identity links win.
    fn by_hash(&self, view: &mut View) {
        let keys: Vec<_> = self.active.keys().cloned().collect();
        for (n, ak) in keys.iter().enumerate() { for bk in keys.iter().skip(n + 1) {
            let (a, b) = (&self.cache.vaults[ak], &self.cache.vaults[bk]);
            if a.network != b.network { continue; }
            for (pk, p, rk, r) in [(ak, a, bk, b), (bk, b, ak, a)] {
                let mut got: BTreeMap<&str, Vec<&Fact>> = BTreeMap::new();
                for f in &r.rows { if f.net > 0 && !f.hash.is_empty() { got.entry(f.hash.as_str()).or_default().push(f); } }
                let mut pairs: Vec<(&Fact, &Fact)> = Vec::new();
                for f in &p.rows { if f.net < 0 && !f.hash.is_empty() { if let Some(rs) = got.get(f.hash.as_str()) { for o in rs { if o.time.abs_diff(f.time) <= 86400 { pairs.push((f, *o)); } } } } }
                let mut out_n: BTreeMap<&str, usize> = BTreeMap::new(); let mut in_n: BTreeMap<&str, usize> = BTreeMap::new();
                for (f, o) in &pairs { *out_n.entry(&f.id).or_default() += 1; *in_n.entry(&o.id).or_default() += 1; }
                for (f, o) in pairs {
                    if out_n[f.id.as_str()] != 1 || in_n[o.id.as_str()] != 1 { continue; }
                    let event = pair_key(pk, &f.id, rk, &o.id);
                    view.links.entry(p.name.clone()).or_default().entry(f.id.clone()).or_insert(Link { peer: r.name.clone(), peer_id: o.id.clone(), event: event.clone(), tier: 2, forgotten: false, own: false });
                    view.links.entry(r.name.clone()).or_default().entry(o.id.clone()).or_insert(Link { peer: p.name.clone(), peer_id: f.id.clone(), event, tier: 2, forgotten: false, own: false });
                }
            }
        } }
    }
    fn ingest(&mut self, v: &wallet::Vault, a: &activity::Activity, s: &wallet::Store) -> bool {
        let identity = v.spark.as_ref().map(|sp| sp.identity.to_lowercase()).unwrap_or_default();
        let raw: BTreeMap<_,_> = a.spark.iter().map(|t| (t.id.as_str(),t)).collect();
        let mut rows: Vec<_> = a.rows.iter().filter_map(|r| {
            let net = r.net?;
            let t = raw.get(r.txid.as_str());
            let own = t.map(|t| !identity.is_empty() && t.sender.eq_ignore_ascii_case(&identity) && t.receiver.eq_ignore_ascii_case(&identity)).unwrap_or(false);
            let counter = t.map(|t| if t.direction == "out" {t.receiver.to_lowercase()} else {t.sender.to_lowercase()}).unwrap_or_default();
            Some(Fact { id:r.txid.clone(),net,time:r.time.unwrap_or(0),counter,own,known:a.transfers.contains_key(&r.txid), paid:Vec::new(), paid_to:Vec::new(), got:Vec::new(), hash:String::new() })
        }).collect();
        if let Some(st) = &v.statement {
            // a statement row names one address and one amount: what it paid, or what arrived
            let by_id: BTreeMap<&str, &crate::statement::Row> = st.rows.iter().map(|r| (r.id.as_str(), r)).collect();
            for f in rows.iter_mut() { if let Some(r) = by_id.get(f.id.as_str()) { if r.layer == "lightning" { f.hash = r.hash.clone(); } if r.layer == "chain" && !r.address.is_empty() {
                let amount = (f.net.unsigned_abs()).saturating_sub(r.fee);
                if f.net < 0 { f.paid_to = vec![(r.address.clone(), amount)]; f.paid = vec![amount]; } else { f.got = vec![(r.address.clone(), amount)]; }
            } } }
        } else if let Some(n) = &v.node {
            let by_id: BTreeMap<&str, &crate::node::Event> = n.events.iter().map(|e| (e.id.as_str(), e)).collect();
            for f in rows.iter_mut() { if let Some(e) = by_id.get(f.id.as_str()) { if e.layer == "lightning" { f.hash = e.hash.clone(); } } }
        } else if identity.is_empty() {
            use std::str::FromStr;
            let net_of = crate::wallet::network_of(&v.network);
            let ids: Vec<(usize, bitcoin::Txid)> = rows.iter().enumerate().filter(|(_, r)| r.net < 0).filter_map(|(i, r)| bitcoin::Txid::from_str(&r.id).ok().map(|t| (i, t))).collect();
            let txs = crate::db::txs(&v.network, &ids.iter().map(|(_, t)| *t).collect::<Vec<_>>());
            for ((i, _), t) in ids.iter().zip(txs) { if let Some(t) = t {
                let mut paid: Vec<u64> = t.output.iter().map(|o| o.value.to_sat()).collect(); paid.sort(); paid.dedup(); rows[*i].paid = paid;
                let mut to: Vec<(String, u64)> = t.output.iter().filter_map(|o| bitcoin::Address::from_script(&o.script_pubkey, net_of).ok().map(|a| (a.to_string().to_lowercase(), o.value.to_sat()))).collect(); to.sort(); rows[*i].paid_to = to;
            } }
            // an inflow's own outputs, by address: the activity row already names them by index
            let outs: BTreeMap<&str, &Vec<(u32, u64)>> = a.rows.iter().map(|r| (r.txid.as_str(), &r.outs)).collect();
            for f in rows.iter_mut() { if f.net > 0 { if let Some(o) = outs.get(f.id.as_str()) {
                let mut got: Vec<(String, u64)> = o.iter().filter_map(|(i, s)| crate::wallet::address_at(v, *i).ok().map(|a| (a.to_lowercase(), *s))).collect(); got.sort(); f.got = got;
            } } }
        }
        rows.sort_by(|a,b| a.id.cmp(&b.id));
        let fingerprint = digest(&serde_json::to_vec(&rows).unwrap());
        let id=key(v);
        let changed = self.cache.vaults.get(&id).map(|p| &p.fingerprint) != Some(&fingerprint);
        if changed { self.cache.vaults.insert(id, Snapshot {name:v.name.clone(),network:v.network.clone(),identity,muun:v.muun.is_some(),statement:v.statement.is_some(),node:v.node.is_some(),exchange:v.exchange.is_some(),fingerprint,rows}); }
        self.reconcile(s,changed); changed
    }
}
fn with_index<T>(f: impl FnOnce(&mut Index)->T) -> T {
    let mut g=INDEX.lock().unwrap_or_else(|p|p.into_inner());
    let i=g.get_or_insert_with(|| Index {cache:crate::db::transfer_cache_get().and_then(|s|serde_json::from_str(&s).ok()).unwrap_or_default(),..Default::default()});
    f(i)
}
pub fn record(v:&wallet::Vault,a:&mut activity::Activity,s:&wallet::Store) {
    with_index(|i| {
        let before=i.builds;
        i.ingest(v,a,s);
        if i.builds != before { if let Ok(json)=serde_json::to_string(&i.cache) {crate::db::transfer_cache_put(&json);} }
        if let Some(links)=i.view.links.get(&v.name) { for (id,l) in links {
            if !l.forgotten && !l.own { a.transfers.insert(id.clone(),vec![l.peer.clone()]); }
        } }
    });
}
pub fn view(s:&wallet::Store)->View { with_index(|i| {i.reconcile(s,false); i.view.clone()}) }
pub fn answer(key:&str,yes:bool)->Result<wallet::Store,String> {
    // Lock order is index -> store throughout the matcher; the answer and view advance together.
    with_index(|i| {
        let s=wallet::load()?;i.reconcile(&s,false);
        let p=i.view.proposals.iter().find(|p|p.key==key).ok_or("this suggestion changed; check the refreshed list")?;
        // a Yes on a contested pair is the person choosing between the matches: once this leg is linked, the
        // reconcile below drops every other candidate that shares it
        let answer=Answer{yes,pair:p.pair.clone()};
        let s=wallet::update(|s| {s.transfer_answers.insert(key.to_string(),answer);})?;
        i.reconcile(&s,false);Ok(s)
    })
}
pub fn clear() { if let Ok(mut g)=INDEX.lock(){*g=None;} }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identities_directions_network_self_and_forgotten() {
        let mut i=Index::default();
        i.active=BTreeMap::from([("a".into(),"A".into()),("b".into(),"B".into()),("c".into(),"C".into())]);
        i.cache.vaults.insert("a".into(),Snapshot{name:"A".into(),network:"mainnet".into(),identity:"aa".into(),rows:vec![Fact{id:"out".into(),net:-10,counter:"bb".into(),..Default::default()},Fact{id:"self".into(),own:true,..Default::default()},Fact{id:"wrongnet".into(),counter:"cc".into(),..Default::default()}],..Default::default()});
        i.cache.vaults.insert("b".into(),Snapshot{name:"B".into(),network:"mainnet".into(),identity:"bb".into(),rows:vec![Fact{id:"out".into(),net:10,counter:"aa".into(),..Default::default()}],..Default::default()});
        i.cache.vaults.insert("c".into(),Snapshot{name:"C".into(),network:"signet".into(),identity:"cc".into(),..Default::default()});
        i.build();assert_eq!(i.view.links["A"]["out"].event,i.view.links["B"]["out"].event);assert!(i.view.links["A"]["self"].own);assert!(!i.view.links["A"].contains_key("wrongnet"));
        i.active.remove("b");i.build();assert!(i.view.links["A"]["out"].forgotten);
    }
    fn vault(name:&str)->wallet::Vault { serde_json::from_value(serde_json::json!({"name":name,"network":"mainnet","threshold":0,"keys":[],"descriptor":name,"address":name,"created":0})).unwrap() }
    #[test] fn fingerprint_gate_ignores_prices_but_not_amount_or_time() {
        let v=vault("A"); let mut s=wallet::Store::default();s.vaults.push(v.clone());
        let mut a=activity::Activity {spark:vec![],ccy:"AUD".into(),rows:vec![activity::Row{txid:"r".into(),time:Some(1),net:Some(10),fee:None,quote:None,ins:vec![],outs:vec![]}],now:None,note:None,transfers:Default::default()};
        let mut i=Index::default();assert!(i.ingest(&v,&a,&s));let n=i.builds;
        a.rows[0].quote=Some(activity::Quote{price:100.,basis:"test".into()});assert!(!i.ingest(&v,&a,&s));assert_eq!(i.builds,n);
        a.rows[0].net=Some(11);assert!(i.ingest(&v,&a,&s));a.rows[0].time=Some(2);assert!(i.ingest(&v,&a,&s));
    }
    fn snap(name:&str, identity:&str, amounts:&[(i64,u32,&str)])->Snapshot {
        Snapshot{name:name.into(),network:"mainnet".into(),identity:identity.into(),fingerprint:name.into(),rows:amounts.iter().map(|(n,t,id)|Fact{id:(*id).into(),net:*n,time:*t,..Default::default()}).collect(),..Default::default()}
    }
    fn indexed()->Index {
        let mut i=Index::default();i.active=BTreeMap::from([("a".into(),"A".into()),("b".into(),"B".into())]);
        i.cache.vaults.insert("a".into(),snap("A","aa",&[(-20000,10000,"out")]));
        i.cache.vaults.insert("b".into(),snap("B","",&[(19500,10100,"in")]));i.build();i
    }
    #[test] fn candidates_are_bounded_by_amount_time_and_network() {
        let a=snap("A","aa",&[(-20000,10000,"out")]);let mut b=snap("B","bb",&[(20000,13600,"in")]);
        assert_eq!(candidates("a",&a,"b",&b).len(),1);
        b.rows[0].time=13601;assert!(candidates("a",&a,"b",&b).is_empty());
        b.rows[0].time=6400;assert_eq!(candidates("a",&a,"b",&b).len(),1);
        b.rows[0].time=6399;assert!(candidates("a",&a,"b",&b).is_empty());
        b.rows[0].time=10000;b.rows[0].net=19999;assert!(candidates("a",&a,"b",&b).is_empty());
        b.identity.clear();b.rows[0].time=96400;b.rows[0].net=10000;assert_eq!(candidates("a",&a,"b",&b).len(),1);
        b.rows[0].time=96401;assert!(candidates("a",&a,"b",&b).is_empty());
        b.rows[0].time=10000;b.rows[0].net=9999;assert!(candidates("a",&a,"b",&b).is_empty());
        b.rows[0].net=19000;b.muun=true;assert_eq!(candidates("a",&a,"b",&b).len(),1);
        b.rows[0].net=18999;assert!(candidates("a",&a,"b",&b).is_empty());
        b.rows[0].net=20000;b.network="signet".into();assert!(candidates("a",&a,"b",&b).is_empty());
    }
    #[test] fn an_output_of_exactly_the_amount_that_arrived_is_a_candidate_whatever_the_net() {
        let mut i=Index::default();i.active=BTreeMap::from([("c".into(),"Chain".into()),("s".into(),"Spark".into())]);
        let mut chain=snap("Chain","",&[(-50000,10000,"tx")]); chain.rows[0].paid=vec![1095,48000];
        i.cache.vaults.insert("c".into(),chain); i.cache.vaults.insert("s".into(),snap("Spark","ss",&[(1095,10500,"dep")])); i.build();
        assert_eq!(i.view.proposals.len(),1,"the 48,905-sat difference is change and fee, not a reason to refuse");
        assert!(i.view.proposals[0].pair.exact && i.view.proposals[0].unique);
        // the same rows with nothing known about what the transaction paid: outside the allowance, not a candidate
        let mut j=Index::default();j.active=i.active.clone();
        j.cache.vaults.insert("c".into(),snap("Chain","",&[(-50000,10000,"tx")])); j.cache.vaults.insert("s".into(),snap("Spark","ss",&[(1095,10500,"dep")])); j.build();
        assert!(j.view.proposals.is_empty());
        // found both ways — within the allowance AND by an exact output — it is one proposal, and the exact reading
        let mut k=Index::default();k.active=i.active.clone();
        let mut near=snap("Chain","",&[(-2532,10000,"tx")]); near.rows[0].paid=vec![1095,1000];
        k.cache.vaults.insert("c".into(),near); k.cache.vaults.insert("s".into(),snap("Spark","ss",&[(1095,10500,"dep")])); k.build();
        assert_eq!(k.view.proposals.len(),1); assert!(k.view.proposals[0].pair.exact && k.view.proposals[0].unique);
    }
    #[test] fn the_same_address_and_amount_on_both_sides_is_one_payment_unless_it_repeats() {
        let mut i=Index::default();i.active=BTreeMap::from([("s".into(),"Statement".into()),("c".into(),"Chain".into())]);
        let mut st=snap("Statement","",&[(-21000,10000,"row1"),(-5000,20000,"row2"),(-5000,30000,"row3")]);
        st.rows[0].paid_to=vec![("bc1qcold".into(),20000)]; st.rows[1].paid_to=vec![("bc1qsame".into(),5000)]; st.rows[2].paid_to=vec![("bc1qsame".into(),5000)];
        let mut ch=snap("Chain","",&[(20000,10100,"tx1"),(5000,20100,"tx2"),(5000,30100,"tx3"),(20000,9_000_000,"far")]);
        ch.rows[0].got=vec![("bc1qcold".into(),20000)]; ch.rows[1].got=vec![("bc1qsame".into(),5000)]; ch.rows[2].got=vec![("bc1qsame".into(),5000)]; ch.rows[3].got=vec![("bc1qcold".into(),20000)];
        i.cache.vaults.insert("s".into(),st); i.cache.vaults.insert("c".into(),ch); i.build();
        let l=&i.view.links["Statement"]["row1"]; assert_eq!((l.peer.as_str(),l.peer_id.as_str(),l.tier),("Chain","tx1",2),"paid to the address, received at it: one payment, both ways");
        assert_eq!(i.view.links["Chain"]["tx1"].peer_id,"row1");
        assert!(!i.view.links["Statement"].contains_key("row2") && !i.view.links["Chain"].contains_key("tx2"),"two identical payments to one address are not assumed");
        assert!(!i.view.links["Chain"].contains_key("far"),"a receipt three months later is not the same payment");
    }
    #[test] fn a_statements_lightning_rows_are_proposed_to_spark_and_muun_only_and_its_chain_rows_never() {
        let mut i=Index::default();i.active=BTreeMap::from([("s".into(),"Statement".into()),("k".into(),"Spark".into()),("c".into(),"Chain".into())]);
        let mut st=snap("Statement","",&[(-70,10000,"ln"),(-21000,20000,"chain")]); st.statement=true; st.rows[1].paid_to=vec![("bc1qx".into(),20000)];
        i.cache.vaults.insert("s".into(),st); i.cache.vaults.insert("k".into(),snap("Spark","kk",&[(70,10300,"in")])); i.cache.vaults.insert("c".into(),snap("Chain","",&[(21000,20300,"got")])); i.build();
        let p: Vec<_>=i.view.proposals.iter().map(|p|(p.pair.from_id.clone(),p.pair.to_id.clone())).collect();
        assert_eq!(p,vec![("ln".to_string(),"in".to_string())],"the Lightning row is proposed to the Spark wallet; the chain row is not proposed to anyone: {p:?}");
    }
    #[test] fn the_same_payment_hash_paid_by_one_and_received_by_another_is_one_payment() {
        let mut i=Index::default();i.active=BTreeMap::from([("s".into(),"Statement".into()),("n".into(),"Node".into())]);
        let mut st=snap("Statement","",&[(-70,10000,"pay")]); st.statement=true; st.rows[0].hash="ab".repeat(32);
        let mut nd=snap("Node","",&[(70,10020,"inv"),(70,10030,"other")]); nd.rows[0].hash="ab".repeat(32); nd.rows[1].hash="cd".repeat(32);
        i.cache.vaults.insert("s".into(),st); i.cache.vaults.insert("n".into(),nd); i.build();
        let l=&i.view.links["Statement"]["pay"]; assert_eq!((l.peer.as_str(),l.peer_id.as_str(),l.tier),("Node","inv",2));
        assert_eq!(i.view.links["Node"]["inv"].peer_id,"pay"); assert!(!i.view.links["Node"].contains_key("other"));
    }
    #[test] fn a_row_that_arrives_twice_is_one_candidate_and_still_unique() {
        let mut i=indexed();
        let (ak,a)=i.cache.vaults.iter().find(|(_,v)|v.name=="A").map(|(k,v)|(k.clone(),v.clone())).unwrap();
        let mut twice=a.clone(); let dup=twice.rows[0].clone(); twice.rows.push(dup); twice.fingerprint.push('x');
        i.cache.vaults.insert(ak,twice); i.pairs.clear(); i.build();
        assert_eq!(i.view.proposals.len(),1,"{:?}",i.view.proposals.iter().map(|p|&p.pair.from_id).collect::<Vec<_>>());
        assert!(i.view.proposals[0].unique,"a duplicate of itself is not a second fit");
    }
    #[test] fn uniqueness_is_both_sides_and_answers_survive_rebuild() {
        let mut i=indexed();assert_eq!(i.view.proposals.len(),1);assert!(i.view.proposals[0].unique);assert!(i.view.links["A"].is_empty());
        let p=i.view.proposals[0].pair.clone();let key=p.key();
        i.cache.vaults.get_mut("a").unwrap().rows.push(Fact{id:"out2".into(),net:-20000,time:10000,..Default::default()});i.cache.vaults.get_mut("a").unwrap().fingerprint="changed".into();i.build();
        assert_eq!(i.view.proposals.len(),2);assert!(i.view.proposals.iter().all(|p|!p.unique));
        i.answers.insert(key.clone(),Answer{yes:false,pair:p.clone()});i.build();assert_eq!(i.view.proposals.len(),1);assert!(i.view.proposals[0].unique);
        i.answers.insert(key,Answer{yes:true,pair:p});i.build();assert_eq!(i.view.links["A"]["out"].tier,3);assert!(i.view.proposals.is_empty());
        i.active.remove("b");i.build();assert!(i.view.links["A"]["out"].forgotten);
        i.active.insert("b".into(),"Renamed".into());i.build();assert_eq!(i.view.links["A"]["out"].peer,"Renamed");assert!(!i.view.links["A"]["out"].forgotten);
        i.cache.vaults.get_mut("b").unwrap().rows[0].net=19000;i.build();assert!(!i.view.links["A"].contains_key("out"));
    }
    #[test] fn historical_addition_and_only_changed_pairs_are_matched() {
        let mut i=indexed();let n=i.pair_builds;i.build();assert_eq!(i.pair_builds,n);
        i.active.insert("c".into(),"C".into());i.cache.vaults.insert("c".into(),snap("C","cc",&[(20000,10001,"historic")]));i.build();assert_eq!(i.pair_builds,n+2);assert!(i.view.proposals.iter().all(|p|!p.unique));
        i.cache.vaults.get_mut("b").unwrap().fingerprint="different".into();i.build();assert_eq!(i.pair_builds,n+4);
    }
    #[test] fn user_answers_round_trip_in_store() {
        let i=indexed();let p=i.view.proposals[0].pair.clone();let mut s=wallet::Store::default();s.transfer_answers.insert(p.key(),Answer{yes:false,pair:p});
        let back:wallet::Store=serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();assert_eq!(s.transfer_answers,back.transfer_answers);
    }
    #[test] fn pair_keys_are_symmetric_and_scoped(){assert_eq!(pair_key("a","1","b","2"),pair_key("b","2","a","1"));assert_ne!(pair_key("a","1","b","2"),pair_key("c","1","b","2"));}
}
