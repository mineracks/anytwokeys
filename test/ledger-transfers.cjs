const fs=require('fs'),assert=require('assert/strict');
const src=fs.readFileSync(require('path').join(__dirname,'../src/app.js'),'utf8');
const body=src.slice(src.indexOf('function buildLedger('),src.indexOf('// Which coins a disposal'));
const buildLedger=new Function(body+';return buildLedger')();
const row=(id,t,n,p=200000)=>({txid:id,time:t,net:n,quote:{price:p},ins:[],outs:[]});
for(const method of ['utxo','fifo','average']) {
 const inputs=[{name:'A',network:'mainnet',rows:[row('deposit',1,100000,100000),row('move',2,-60000)]},{name:'B',network:'mainnet',rows:[row('move',2,60000)]}];
 const l=buildLedger(inputs,method);assert.equal(l.perVault.basis.A,40,method);assert.equal(l.perVault.basis.B,60,method);assert.equal(Object.keys(l.realised).length,0,method);
 const fee=[{name:'A',network:'mainnet',rows:[row('deposit',1,100000,100000),row('out',2,-60000)],links:{out:{peer:'B',peer_id:'in',event:'pair'}}},{name:'B',network:'mainnet',rows:[row('in',3,59000)],links:{in:{peer:'A',peer_id:'out',event:'pair'}}}];
 const f=buildLedger(fee,method);assert.ok(Math.abs(f.perVault.basis.B-59)<1e-8);assert.equal(f.realised.out.sats,1000);assert.ok(Math.abs(f.realised.out.cost-1)<1e-8);
 const no=buildLedger(fee.map(v=>({...v,links:{}})),method);assert.equal(no.realised.out.sats,60000);
 const chain=[{name:'A',network:'mainnet',rows:[{...row('deposit',1,100000,100000),outs:[[0,100000]]},{...row('out',2,-60000),ins:[['deposit:0',100000]],outs:[[1,40000]]}],links:fee[0].links},fee[1]];
 const c=buildLedger(chain,method);assert.ok(Math.abs(c.perVault.basis.B-59)<1e-8,method+' chain');assert.equal(c.realised.out.sats,1000);
 const self=buildLedger([{name:'A',network:'mainnet',rows:[row('deposit',1,100000,100000),row('self',2,-1000)],links:{self:{own:true}}}],method);assert.equal(self.perVault.basis.A,100);assert.equal(Object.keys(self.realised).length,0);
}
console.log('PASS all three methods: Spark moves, confirmed distinct ids, proposals unchanged, chain bridge, fee-only disposal, self');
