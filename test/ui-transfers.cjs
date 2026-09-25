const {webkit}=require('playwright'); const fs=require('fs'); const path=require('path'); const assert=require('assert/strict');
const source=path.resolve(__dirname, '../src/index.html');
const out=path.resolve(process.argv[2] || '/tmp/anytwokeys-spark-ui'); fs.mkdirSync(out,{recursive:true});
const homeMock=fs.readFileSync(path.join(__dirname,'ui-home.py'),'utf8').match(/MOCK = """([\s\S]*?)"""/)[1];
const values={vault:JSON.stringify({name:'demo vault',network:'mainnet',threshold:2,keys:[],descriptor:'wsh(…)',address:'bc1qexample',created:0,chain_code:null,labels:{},seal:{shares:[],sealed:[]}}),fiat:'""',os:'{"region":"AU","currency":"AUD"}',t0:'1'.repeat(64),t1:'2'.repeat(64),t2:'3'.repeat(64),t3:'4'.repeat(64)};

const mock=homeMock.replace(/%\((\w+)\)s/g,(_,k)=>values[k])+`
V.name='A'; V.spark={identity:'aa',address:'spark1a'}; __extra.push({...V,name:'B',spark:undefined,descriptor:'B'});
window.__answer=null; window.__ambiguous=false;
const proposal={key:'pair',unique:true,pair:{from:'A',to:'B',from_id:'out',to_id:'in',out:60000,incoming:59000,at:1789640000,arrived:1789640100}};
H.transfer_links=()=>({links:__answer===true?{A:{out:{peer:'B',peer_id:'in',event:'pair',tier:3}},B:{in:{peer:'A',peer_id:'out',event:'pair',tier:3}}}:{},proposals:__answer===null?[{...proposal,unique:!__ambiguous}]:[]});
H.transfer_answer=a=>{if(a.yes&&__ambiguous)throw 'ambiguous';window.__answer=a.yes;return store();};
H.vault_sync=()=>({coins:[],balance:40000,history:[],height:0,servers:{electrum:'test',explorer:'test'},receive:'',rotating:false});
const row=(id,time,net,price)=>({txid:id,time,net,fee:null,quote:{price,basis:'fixture'},ins:[],outs:[]});
H.vault_activity=a=>({ccy:'AUD',now:{price:200000,basis:'fixture'},note:null,transfers:{},rows:a.name==='A'?[row('deposit',1789630000,100000,100000),row('out',1789640000,-60000,200000)]:[row('in',1789640100,59000,200000)],spark:a.name==='A'?[{id:'out',status:5,at:1789640000,sats:60000,direction:'out',sender:'aa',receiver:'outside'}]:[]});
`;
(async()=>{
 const browser=await webkit.launch(process.env.A2K_WEBKIT?{executablePath:process.env.A2K_WEBKIT}:{});
 const page=await browser.newPage({viewport:{width:1250,height:900}});const errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(mock);await page.goto('file://'+source);await page.waitForTimeout(900);
 await page.evaluate(async()=>{await sumInput('A');await sumInput('B');vault=store.vaults.find(v=>v.name==='A');show('activity');await refreshTransferLinks();});
 assert.equal(await page.locator('[data-transfer-answer][data-yes="true"]').count(),1);
 for(const method of ['utxo','fifo','average']) {
  const before=await page.evaluate(m=>{store.settings.cost_method=m;vaultLedger.memo={};return vaultLedger('B').perVault.basis.B;},method);assert.equal(before,118,'proposal does not move basis');
 }
 await page.screenshot({path:path.join(out,'possible-transfer.png'),fullPage:true});
 await page.click('[data-transfer-answer][data-yes="true"]');await page.waitForTimeout(100);
 for(const method of ['utxo','fifo','average']) {
  const after=await page.evaluate(m=>{store.settings.cost_method=m;vaultLedger.memo={};const a=vaultLedger('A'),b=vaultLedger('B');return {basis:b.perVault.basis.B,fee:a.realised.out.sats,bTimeline:b.timeline.at(-1),aTimeline:a.timeline.at(-1)};},method);
  assert.ok(Math.abs(after.basis-59)<1e-8);assert.equal(after.fee,1000);assert.equal(after.bTimeline.sats,59000);assert.ok(Math.abs(after.bTimeline.basis-59)<1e-8);assert.equal(after.aTimeline.sats,40000);
 }
 assert.equal(await page.locator('[data-transfer-answer]').count(),0);assert.match(await page.innerText('#act-all'),/B/);
 await page.evaluate(()=>openTx('out'));await page.waitForTimeout(100);assert.match(await page.innerText('#tx-dl'),/Transfer to B/);
 await page.evaluate(async()=>{vault=store.vaults.find(v=>v.name==='B');show('home');await renderSummary(document.querySelector('#home-summary'),['B']);});
 assert.match(await page.innerText('#home-summary'),/\$59(?:\.00)?/,'Home shows carried basis, not acquisition price');
 await page.screenshot({path:path.join(out,'confirmed-home.png'),fullPage:true});
 await page.evaluate(async()=>{window.__answer=null;window.__ambiguous=true;show('activity');await refreshTransferLinks();});
 assert.equal(await page.locator('[data-transfer-answer][data-yes="true"]').count(),0);assert.match(await page.innerText('#transfer-proposals'),/More than one/);
 await page.click('[data-transfer-answer][data-yes="false"]');await page.waitForTimeout(100);await page.evaluate(()=>refreshTransferLinks());assert.equal(await page.locator('[data-transfer-answer]').count(),0);
 assert.deepEqual(errors,[]);console.log('PASS: proposals leave accounting unchanged, confirmation carries basis in all methods and Home, fee only, details, ambiguity, denial');await browser.close();
})().catch(e=>{console.error(e);process.exit(1)});
