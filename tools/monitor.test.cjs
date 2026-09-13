const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const { fixture } = require('./console-fixture.cjs');
require('../src/monitor.js');
const M = globalThis.CueMixModel, Monitor = globalThis.CueMixMonitor;
globalThis.escapeHtml = s => s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('"','&quot;');

test('monitor sources use separate ABC banks and a shared stereo destination', () => {
  const model = M.create(fixture());
  assert.equal(model.monitorAvailable,true);
  assert.equal(model.sourceName('13b90201'),'ABC C R');
  assert.deepEqual(model.destinations.filter(d=>d.group==='monitor').map(d=>d.index),[0,1]);
  assert.equal(M.serialize(M.edits(model,'monitor-members','monitor',[0],2314)),'monitor-members:monitor:0:0003:2314');
  assert.equal(M.encoded('monitor-members',2314),'090a');
  assert.equal(M.encoded('monitor-level','-inf'),'64');
  assert.equal(M.encoded('monitor-level',-30),'1e');
});
test('monitor availability fails closed for missing, malformed or additional channels', () => {
  for(const mutate of [s=>s.records.splice(s.records.findIndex(r=>r[0]===0x93b9&&r[1]===1),1),s=>s.records.push([0x93b9,2,'00000000']),s=>s.records.find(r=>r[0]===0x13b6)[2]='0000']) {
    const snapshot=fixture();mutate(snapshot);
    const model=M.create(snapshot);
    assert.equal(!!model.monitorAvailable,false);
    assert.equal(model.sources.filter(s=>s.group==='ABC speakers').length,0);
  }
});
test('moving a speaker pair clears previous assignments and preserves unrelated routes', () => {
  const snapshot=fixture();
  for(let i=0;i<2;i++) snapshot.records.find(r=>r[0]===0x93ac&&r[1]===i)[2]=M.path(0x13b9,0,i);
  const model=M.create(snapshot);
  const edits=Monitor.pairChanges(model,new Map(),0,'line:2,line:3');
  assert.deepEqual(edits.map(c=>[c.destination.id,c.source]),[['line:0','00000000'],['line:1','00000000'],['line:2','13b90000'],['line:3','13b90001']]);
  const drafts=new Map(edits.map(c=>[M.key(c.destination.property,c.destination.index),{value:c.source}]));
  assert.deepEqual(Monitor.pairChanges(model,drafts,0,'none').map(c=>c.destination.id),['line:2','line:3']);
  assert.throws(()=>Monitor.pairChanges(model,drafts,0,'line:1,line:2'),/advertised/);
});
test('front-panel multi-selection, muted state and disconnected setup stay visible', () => {
  const snapshot=fixture();snapshot.records.find(r=>r[0]===0x13b6)[2]='03';snapshot.records.find(r=>r[0]===0x139b)[2]='01';
  const html=Monitor.render(M.create(snapshot),new Map());
  assert.match(html,/Speaker selection · A \+ B/);
  assert.match(html,/Muted on the device/);
  assert.match(html,/unassigned input/);
  assert.match(html,/Monitor Group/);
  assert.match(html,/<button[^>]+data-monitor-toggle="1"[^>]+data-value="2"[^>]+aria-pressed="true"/);
  assert.match(html,/<button[^>]+data-monitor-toggle="2"[^>]+data-value="1"[^>]+aria-pressed="true"/);
  assert.match(html,/<option value="3" selected>A \+ B/);
  assert.match(html,/<option value="5">A \+ C/);
  assert.match(html,/<option value="6">B \+ C/);
  assert.match(html,/value="-30"/);
});

function syncHarness() {
  let next=0;
  const scheduled=new Map(), requests=[], states=[], statuses=[];
  const sync=Monitor.createSync({
    send:changes=>new Promise((resolve,reject)=>requests.push({changes,resolve,reject})),
    onState:(confirmed,display)=>states.push({confirmed,display}),
    onStatus:(message,state)=>statuses.push({message,state}),
    schedule:fn=>{scheduled.set(++next,fn);return next;},
    cancel:id=>scheduled.delete(id),
  });
  const receive=(hex,revision)=>sync.receive({records:[[0x1393,0,hex]],revision});
  const run=()=>{
    const [id,fn]=scheduled.entries().next().value;
    scheduled.delete(id);
    return fn();
  };
  receive('1e',1);
  return {sync,requests,states,statuses,scheduled,receive,run};
}
const current=(state,property=0x1393)=>state.records.find(r=>r[0]===property)?.[2];

test('mute, mono and talk render independently and require valid advertised latches',()=>{
  const snapshot=fixture();
  snapshot.records.find(r=>r[0]===0x13a3)[2]='01';
  snapshot.records.find(r=>r[0]===0x139a)[2]='01';
  let html=Monitor.render(M.create(snapshot),new Map());
  assert.match(html,/<button[^>]+data-control="monitor-mute"[^>]+data-value="1"[^>]+aria-pressed="false"><span class="monitor-action-label">Mute<\/span><span class="monitor-action-state">Off/);
  assert.match(html,/<button[^>]+data-control="monitor-talk"[^>]+data-value="0"[^>]+aria-pressed="true"><span class="monitor-action-label">Talk<\/span><span class="monitor-action-state">On/);
  assert.match(html,/<button[^>]+data-control="monitor-mono"[^>]+data-value="0"[^>]+aria-pressed="true"><span class="monitor-action-label">Mono<\/span><span class="monitor-action-state">On/);
  assert.match(html,/Click to talk; click again to stop/);
  for(const [operation,property] of [['monitor-talk',0x13a3],['monitor-mono',0x139a]]) for(const value of [undefined,'02','0000']) {
    snapshot.records=snapshot.records.filter(r=>r[0]!==property);
    if(value!==undefined)snapshot.records.push([property,0,value]);
    html=Monitor.render(M.create(snapshot),new Map());
    assert.match(html,new RegExp(`<button[^>]+data-control="${operation}"[^>]+data-unavailable="true" disabled`));
  }
});

test('live front-panel refresh preserves pointer targets and only changes state text when needed',()=>{
  for(const [operation,property,label] of [['monitor-mute',0x139b,'Mute'],['monitor-mono',0x139a,'Mono'],['monitor-talk',0x13a3,'Talk']]) {
    let text='Off', textUpdates=0;
    const nameNode={textContent:label};
    const stateNode={get textContent(){return text;},set textContent(value){text=value;textUpdates++;}};
    const button={dataset:{},setAttribute(name,value){this[name]=value;},
      querySelector:selector=>selector==='.monitor-action-state'?stateNode:nameNode,
      set innerHTML(_){throw Error('A live refresh replaced the pending pointer target');}};
    const context={CueMixModel:M,document:{
      getElementById:id=>id==='monitorLevel'?{}:null,
      querySelector:selector=>selector===`[data-control="${operation}"]`?button:null,
      querySelectorAll:()=>[],
    }};
    vm.runInNewContext(fs.readFileSync(require.resolve('../src/monitor.js'),'utf8'),context);
    for(let i=0;i<20;i++) context.CueMixMonitor.syncDom({records:[[property,0,'00']]});
    assert.equal(textUpdates,0,'unchanged meter events must not rewrite labels');
    context.CueMixMonitor.syncDom({records:[[property,0,'01']]});
    assert.equal(stateNode.textContent,'On');
    assert.equal(textUpdates,1);
    assert.equal(button['aria-pressed'],'true');
    assert.equal(button.dataset.value,'0');
    assert.equal(nameNode.textContent,label);
  }
});

test('front-panel buttons coalesce rapid toggles, recover from rejection and track physical changes',async()=>{
  for(const [operation,property] of [['monitor-talk',0x13a3],['monitor-mono',0x139a]]) {
  const h=syncHarness();
  h.sync.receive({records:[[0x139b,0,'00'],[property,0,'00']],revision:2});
  h.sync.queue('monitor-mute',1);
  h.sync.queue('monitor-mute',0);
  await h.run();
  assert.equal(h.requests.length,0,'a cancelled preview must not touch hardware');
  h.sync.queue(operation,1);
  const first=h.run();
  assert.equal(M.serialize(h.requests[0].changes),`${operation}:monitor:0:00:1`);
  h.sync.queue(operation,0);
  h.requests[0].resolve({monitor:{records:[[property,0,'01']],revision:3}});
  await first;
  assert.equal(current(h.sync.display(),property),'00');
  const second=h.run();
  assert.equal(M.serialize(h.requests[1].changes),`${operation}:monitor:0:01:0`);
  h.requests[1].reject(Error('Device rejected change'));
  await second;
  assert.equal(current(h.sync.display(),property),'01','rejection restores confirmed On state');
  assert.equal(h.scheduled.size,0,'never retry a failed setter');
  h.sync.receive({records:[[0x139b,0,'01'],[property,0,'00']],revision:4});
  h.sync.receive({records:[[0x139b,0,'00'],[property,0,'01']],revision:3});
  assert.equal(current(h.sync.display(),0x139b),'01');
  assert.equal(current(h.sync.display(),property),'00');
  assert.equal(current(h.sync.display()),'1e','front-panel buttons never change the saved level');
  }
});

test('continuous monitor dragging coalesces values and verifies each write before the next',async()=>{
  const h=syncHarness();
  h.sync.queue('monitor-level',-31);
  h.sync.queue('monitor-level',-32);
  h.sync.queue('monitor-level',-33);
  assert.equal(h.scheduled.size,1);
  const first=h.run();
  assert.equal(h.requests.length,1);
  assert.equal(h.requests[0].changes[0].expected,'1e');
  assert.equal(h.requests[0].changes[0].value,'-33');
  h.sync.queue('monitor-level',-34);
  h.sync.queue('monitor-level',-35);
  h.receive('21',2); // Device event can arrive before the HTTP acknowledgement.
  assert.equal(current(h.sync.display()),'23');
  assert.equal(h.requests.length,1);
  h.requests[0].resolve({monitor:{records:[[0x1393,0,'21']],revision:2}});
  await first;
  const second=h.run();
  assert.equal(h.requests[1].changes[0].expected,'21');
  assert.equal(h.requests[1].changes[0].value,'-35');
  h.requests[1].resolve({monitor:{records:[[0x1393,0,'23']],revision:3}});
  await second;
  assert.equal(h.sync.busy(),false);
  assert.equal(current(h.sync.snapshot()),'23');
});

test('external changes preserve conflict expectations and newer state wins over delayed readback',async()=>{
  const h=syncHarness();
  h.sync.queue('monitor-level',-31);
  h.receive('28',2);
  assert.equal(current(h.sync.display()),'1f');
  const task=h.run();
  assert.equal(h.requests[0].changes[0].expected,'1e');
  h.receive('29',4);
  h.requests[0].resolve({monitor:{records:[[0x1393,0,'1f']],revision:3}});
  await task;
  assert.equal(current(h.sync.display()),'29');
  assert.equal(h.statuses.at(-1).state,'error');
  h.sync.seed({records:[[0x1393,0,'1e']]});
  assert.equal(current(h.sync.snapshot()),'29');
});

test('failed monitor writes discard unsent changes without retrying an uncertain outcome',async()=>{
  const h=syncHarness();
  h.sync.queue('monitor-level',-31);
  const task=h.run();
  h.sync.queue('monitor-level',-32);
  h.requests[0].reject(Error('Connection lost; outcome unknown'));
  await task;
  assert.equal(h.sync.busy(),false);
  assert.equal(h.scheduled.size,0);
  assert.equal(h.requests.length,1);
  assert.equal(current(h.sync.display()),'1e');
  h.receive('1f',2);
  assert.equal(current(h.sync.display()),'1f');
});

test('switching devices cancels queued edits and ignores previous-device responses',async()=>{
  const h=syncHarness();
  h.sync.queue('monitor-level',-31);
  h.sync.reset();
  assert.equal(h.scheduled.size,0);
  h.receive('28',5);
  h.sync.queue('monitor-level',-41);
  const task=h.run();
  h.sync.reset();
  h.receive('32',6);
  h.requests[0].resolve({monitor:{records:[[0x1393,0,'29']],revision:7}});
  await task;
  assert.equal(current(h.sync.display()),'32');
  assert.equal(h.sync.busy(),false);
});

test('invalid event records cannot replace the last confirmed monitor level',()=>{
  const h=syncHarness();
  h.sync.receive({records:[null,[],[0x1393,0,'ff'],[0x1393,0,40],[0x1393,1,'01']],revision:2});
  assert.equal(current(h.sync.snapshot()),'1e');
  h.sync.queue('monitor-level',-30);
  h.run();
  assert.equal(h.requests.length,0);
  assert.equal(h.sync.busy(),false);
});
