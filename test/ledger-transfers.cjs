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

for(const method of ['utxo','fifo','average']) {
 const ab={peer:'B',peer_id:'bin',event:'ab'},ba={peer:'A',peer_id:'aout',event:'ab'},bc={peer:'C',peer_id:'cin',event:'bc'},cb={peer:'B',peer_id:'bout',event:'bc'};
 const inputs=[{name:'A',network:'mainnet',links:{aout:ab},rows:[row('deposit',9000,100000,100000),row('aout',10000,-60000)]},{name:'B',network:'mainnet',links:{bin:ba,bout:bc},rows:[row('bin',7000,59000),row('bout',12000,-30000)]},{name:'C',network:'mainnet',links:{cin:cb},rows:[row('cin',12001,29000)]}];
 const l=buildLedger(inputs,method,{},['C']);assert.ok(Math.abs(l.timeline.at(-1).basis-29)<1e-8,method+' multi-hop / clock skew');assert.equal(l.timeline.at(-1).sats,29000);assert.equal(Object.keys(l.realised).length,0);
 const a=buildLedger(inputs,method,{},['A']);assert.equal(a.realised.aout.sats,1000);assert.equal(Object.keys(a.realised).length,1);
 const twice=buildLedger(inputs,method,{},['C']);assert.deepEqual(twice,l,'ledger never mutates its inputs');
 const cross=buildLedger([{name:'A',network:'mainnet',rows:[row('deposit',1,100000,100000),row('move',2,-60000)]},{name:'B',network:'signet',rows:[row('move',2,60000)]}],method);assert.equal(cross.internal,0,'networks never join');
 const gone=buildLedger([{...inputs[0],links:{aout:{...ab,forgotten:true}}}],method);assert.equal(gone.realised.aout.sats,60000,'forgotten peer is a plain outflow');
 const reverse=[{name:'S',network:'mainnet',rows:[row('fund',1,100000,100000),row('x',2,-60000)],links:{x:{peer:'C',peer_id:'y',event:'bridge'}}},{name:'C',network:'mainnet',rows:[{...row('y',3,59000),outs:[[0,59000]]}],links:{y:{peer:'S',peer_id:'x',event:'bridge'}}}];
 const back=buildLedger(reverse,method);assert.ok(Math.abs(back.perVault.basis.C-59)<1e-8);assert.equal(back.realised.x.sats,1000);
}
console.log('PASS: scoped Home/report totals, multi-hop, clock skew, both bridge directions, repeatability, networks, forgotten peer');

{
 const l=buildLedger([{name:'S',rows:[row('one',1,50000,100000),row('two',2,50000,200000),row('spend',3,-60000)]}],'utxo');
 assert.ok(Math.abs(l.realised.spend.cost-70)<1e-8);assert.ok(Math.abs(l.perVault.basis.S-80)<1e-8);
 console.log('PASS: partial Spark account entries retain the unused basis');
}
