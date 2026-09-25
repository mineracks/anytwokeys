//! Read-only transfer evidence. Matching happens as facts arrive, never while drawing or reading a balance.
use crate::{activity, wallet};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap};
use std::sync::Mutex;

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Fact { id: String, net: i64, time: u32, counter: String, own: bool }
#[derive(Clone, Default, Serialize, Deserialize)]
struct Snapshot { name: String, network: String, identity: String, fingerprint: String, rows: Vec<Fact> }
#[derive(Clone, Default, Serialize, Deserialize)]
struct Cache { vaults: BTreeMap<String, Snapshot> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Link { pub peer: String, pub peer_id: String, pub event: String, pub tier: u8, pub forgotten: bool, pub own: bool }
#[derive(Clone, Default, Serialize)]
pub struct View { pub links: BTreeMap<String, BTreeMap<String, Link>> }
#[derive(Default)]
struct Index { cache: Cache, active: BTreeMap<String, String>, view: View, builds: usize }
static INDEX: Mutex<Option<Index>> = Mutex::new(None);
fn digest(s: &[u8]) -> String { hex::encode(Sha256::digest(s)) }
fn key(v: &wallet::Vault) -> String {
    digest(format!("{}|{}", v.network, v.spark.as_ref().map(|s| s.identity.as_str()).unwrap_or(if v.descriptor.is_empty() { &v.address } else { &v.descriptor })).as_bytes())
}
fn pair_key(a: &str, ai: &str, b: &str, bi: &str) -> String {
    let mut ends = [(a, ai), (b, bi)]; ends.sort();
    digest(&serde_json::to_vec(&ends).unwrap())
}
impl Index {
    fn reconcile(&mut self, s: &wallet::Store, changed: bool) {
        let active: BTreeMap<_, _> = s.vaults.iter().map(|v| (key(v), v.name.clone())).collect();
        if !changed && active == self.active { return; }
        self.active = active;
        for v in &s.vaults {
            let snap = self.cache.vaults.entry(key(v)).or_default();
            snap.name = v.name.clone(); snap.network = v.network.clone();
            snap.identity = v.spark.as_ref().map(|sp| sp.identity.to_lowercase()).unwrap_or_default();
        }
        self.build();
    }
    fn build(&mut self) {
        self.builds += 1;
        let mut view = View::default();
        for (id, name) in &self.active {
            let a = &self.cache.vaults[id];
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
        self.view = view;
    }
    fn ingest(&mut self, v: &wallet::Vault, a: &activity::Activity, s: &wallet::Store) -> bool {
        let identity = v.spark.as_ref().map(|sp| sp.identity.to_lowercase()).unwrap_or_default();
        let raw: BTreeMap<_,_> = a.spark.iter().map(|t| (t.id.as_str(),t)).collect();
        let mut rows: Vec<_> = a.rows.iter().filter_map(|r| {
            let net = r.net?;
            let t = raw.get(r.txid.as_str());
            let own = t.map(|t| !identity.is_empty() && t.sender.eq_ignore_ascii_case(&identity) && t.receiver.eq_ignore_ascii_case(&identity)).unwrap_or(false);
            let counter = t.map(|t| if t.direction == "out" {t.receiver.to_lowercase()} else {t.sender.to_lowercase()}).unwrap_or_default();
            Some(Fact { id:r.txid.clone(),net,time:r.time.unwrap_or(0),counter,own })
        }).collect();
        rows.sort_by(|a,b| a.id.cmp(&b.id));
        let fingerprint = digest(&serde_json::to_vec(&rows).unwrap());
        let id=key(v);
        let changed = self.cache.vaults.get(&id).map(|p| &p.fingerprint) != Some(&fingerprint);
        if changed { self.cache.vaults.insert(id, Snapshot {name:v.name.clone(),network:v.network.clone(),identity,fingerprint,rows}); }
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
    #[test] fn pair_keys_are_symmetric_and_scoped(){assert_eq!(pair_key("a","1","b","2"),pair_key("b","2","a","1"));assert_ne!(pair_key("a","1","b","2"),pair_key("c","1","b","2"));}
}
