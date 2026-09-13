// Exercise asynchronous UI races using a small DOM surface and synthetic
// device. No browser packages or hardware writes are needed.
const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const { fixture } = require('./console-fixture.cjs');
const M = globalThis.CueMixModel;
const flush = () => new Promise(resolve => setImmediate(resolve));
function harness() {
  const elements = new Map(), events = {}, ticks = [], reads = [], writes = [];
  const control = { dataset:{control:'level',target:'main',index:'0'},type:'range',value:'-18',disabled:true,getAttribute:()=> 'Input level',setAttribute(){},closest:()=> ({querySelector:selector=>element(`strip ${selector}`)}) };
  const element = id => {
    if (!elements.has(id)) elements.set(id,{ value: id==='host' ? 'simulated-device' : '',dataset:{},innerHTML:'',textContent:'',checked:false,addEventListener(type,fn){this[type]=fn;},setAttribute(name,value){this[name]=value;},querySelector:selector=>element(`${id} ${selector}`) });
    return elements.get(id);
  };
  const status = element('status');
  const panel = {dataset:{},querySelectorAll:()=> [control]};
  const document = {hidden:false,activeElement:null,getElementById:element,
    querySelector:selector=>selector.startsWith('[data-control="monitor-') ? element(selector) : ({dataset:{tab:'mixer'}}),
    querySelectorAll:selector=>selector==='.console-panel'?[panel]:selector==='.console-status'?[status]:selector==='input[type="range"][data-control]'?[control]:[],
    addEventListener(type,fn){(events[type]??=[]).push(fn);}};
  let now = 100000;
  const context = { CueMixModel:M,document,localStorage:{getItem:()=>null,setItem(){}},
    Date:class extends Date {static now(){return now;}},
    setInterval:fn=>ticks.push(fn),setTimeout,clearTimeout,
    escapeHtml:String,meterScaleMarkup:()=>'',lastMeterRecords:[],loadMeters(){},
    qs:o=>new URLSearchParams(o).toString(),sessionToken:'test-token',
    fetchJson:()=>new Promise((resolve,reject)=>reads.push({resolve,reject})),
    fetch:(url,options)=>new Promise(resolve=>writes.push({form:new URLSearchParams(options.body),resolve})),
  };
  vm.runInNewContext(fs.readFileSync(require.resolve('../src/slider_queue.js'),'utf8'),context);
  vm.runInNewContext(fs.readFileSync(require.resolve('../src/db_entry.js'),'utf8'),context);
  vm.runInNewContext(fs.readFileSync(require.resolve('../src/monitor.js'),'utf8'),context);
  context.CueMixMonitor.render=model=>`Monitor ${model.get(0x13b6,0)}`;
  vm.runInNewContext(fs.readFileSync(require.resolve('../src/console.js'),'utf8'),context);
  return {control,document,element,status,reads,writes,ui:context.consoleUi,
    event:(type,target=control)=>events[type]?.forEach(fn=>fn({target})),
    tick:()=>{now+=5001;ticks.forEach(fn=>fn());},
    async ready(){reads.shift().resolve(fixture());await flush();assert.equal(control.disabled,false);},
  };
}
test('background refresh leaves controls enabled and an overlapping edit is sent once',async()=> {
  const h=harness();await h.ready();h.tick();
  assert.equal(h.reads.length,1);assert.equal(h.control.disabled,false);
  h.document.activeElement=h.control;h.event('change');await flush();
  assert.equal(h.writes.length,0,'wait for the already-running read');
  h.reads.shift().resolve(fixture());await flush();
  assert.equal(h.writes.length,1);
  assert.equal(h.writes[0].form.get('changes'),'level:main:0:00404de6:-18');
  const current=fixture();current.records.find(r=>r[0]===0x841a&&r[1]===0)[2]=M.encoded('level',-18);
  h.writes[0].resolve({ok:true,json:async()=>({acknowledged:1})});await flush();
  assert.equal(h.control.disabled,false,'keep the fader draggable through write readback');
  h.reads.shift().resolve(current);await flush();
  assert.equal(h.control.disabled,false);assert.match(h.status.textContent,/Verified/);
});
test('a read arriving during an edit preserves its DOM and original conflict value',async()=> {
  const h=harness();await h.ready();const before=h.element('consoleStrips').innerHTML;
  h.tick();h.document.activeElement=h.control;
  const external=fixture();external.records.find(r=>r[0]===0x841a&&r[1]===0)[2]=M.encoded('level',-6);
  h.reads.shift().resolve(external);await flush();
  assert.equal(h.element('consoleStrips').innerHTML,before);
  h.event('change');await flush();
  assert.match(h.writes[0].form.get('changes'),/:00404de6:-18$/,'retain the displayed prior value so the server can reject a conflict');
  h.writes[0].resolve({ok:false,status:409,json:async()=>({error:'conflict'})});await flush();
  h.reads.shift().resolve(external);await flush();assert.match(h.status.textContent,/conflict/);
});
test('mixer input events save during dragging, preserve the fader, and advance only verified expected bytes',async()=>{
  const h=harness();await h.ready();h.document.activeElement=h.control;
  const before=h.element('consoleStrips').innerHTML;
  h.event('pointerdown');h.event('input');
  await new Promise(resolve=>setTimeout(resolve,100));
  assert.equal(h.writes.length,1,'input alone writes before mouseup/change');
  assert.equal(h.control.disabled,false);
  for(const value of ['-17','-16','-15']) {h.control.value=value;h.event('input');}
  assert.equal(h.writes.length,1,'rapid changes wait for readback');
  h.writes[0].resolve({ok:true,json:async()=>({acknowledged:1})});await flush();
  const current=fixture();current.records.find(r=>r[0]===0x841a&&r[1]===0)[2]=M.encoded('level',-18);
  h.reads.shift().resolve(current);await flush();
  assert.equal(h.element('consoleStrips').innerHTML,before,'readback must not replace the active fader');
  assert.equal(h.control.value,'-15','a previous response cannot move the latest preview');
  await new Promise(resolve=>setTimeout(resolve,100));
  assert.equal(h.writes.length,2);
  assert.equal(h.writes[1].form.get('changes'),`level:main:0:${M.encoded('level',-18)}:-15`);
  h.event('change');h.writes[1].resolve({ok:true,json:async()=>({acknowledged:1})});await flush();
  current.records.find(r=>r[0]===0x841a&&r[1]===0)[2]=M.encoded('level',-15);
  h.reads.shift().resolve(current);await flush();
  assert.equal(h.ui.busy(),false);assert.equal(h.control.disabled,false);
  assert.match(h.status.textContent,/Verified/);
});
test('a failed continuous mix write drops buffered positions and mouseup without retrying',async()=>{
  const h=harness();await h.ready();h.document.activeElement=h.control;h.event('input');
  await new Promise(resolve=>setTimeout(resolve,100));
  h.control.value='-14';h.event('input');
  h.writes[0].resolve({ok:false,status:409,json:async()=>({error:'external conflict'})});await flush();
  h.reads.shift().resolve(fixture());await flush();
  assert.ok(Math.abs(Number(h.control.value) + 12) < 0.001,'failed preview reconciles to device readback without replacing the fader');
  assert.equal(h.element('strip .console-value').textContent,'-12.0 dB');
  h.event('change');await new Promise(resolve=>setTimeout(resolve,100));
  assert.equal(h.writes.length,1);assert.match(h.status.textContent,/external conflict/);
});
test('aux send, pan, and stereo bus master input events use their respective control addresses',async()=>{
  for(const [control,target,index,value] of [['level','aux-0','0','-23'],['pan','main','0','0.25'],['master','aux-0','0','-21']]) {
    const h=harness();await h.ready();h.document.activeElement=h.control;
    h.control.dataset={control,target,index};h.control.value=value;h.event('input');
    await new Promise(resolve=>setTimeout(resolve,100));
    assert.equal(h.writes.length,1,control);
    const changes=h.writes[0].form.get('changes');
    assert.ok(changes.startsWith(`${control}:${target}:${index}:`),changes);
    if(control==='master')assert.match(changes,/master:aux-1:1:/,'linked bus master updates both advertised members');
    h.writes[0].resolve({ok:false,status:409,json:async()=>({error:'test finished'})});await flush();
    h.reads.shift().resolve(fixture());await flush();
  }
});

test('typed mix, aux and linked bus levels use the queued slider changes path',async()=>{
  for(const [control,target,index,value] of [['level','main','0','-12.5'],['level','aux-0','0','-23.2'],['master','aux-0','0','-91']]) {
    const h=harness();await h.ready();h.document.activeElement=h.control;
    h.control.dataset={control,target,index};h.control.value=value;h.event('dbcommit');await flush();
    assert.equal(h.writes.length,1,control);
    const changes=h.writes[0].form.get('changes');
    assert.ok(changes.startsWith(`${control}:${target}:${index}:`),changes);
    assert.ok(changes.endsWith(`:${value==='-91'?'-inf':value}`),changes);
    if(control==='master') assert.match(changes,/master:aux-1:1:/);
    h.writes[0].resolve({ok:false,status:409,json:async()=>({error:'test completed'})});await flush();
    h.reads.shift().resolve(fixture());await flush();
  }
});

test('typed monitor levels use persistent monitor sync and verified readback',async()=>{
  const h=harness();await h.ready();
  h.control.dataset={control:'monitor-level',target:'monitor',index:'0'};h.control.value='-42';
  h.event('dbcommit');await new Promise(resolve=>setTimeout(resolve,80));
  assert.equal(h.writes.length,1);
  assert.equal(h.writes[0].form.get('changes'),'monitor-level:monitor:0:1e:-42');
  h.writes[0].resolve({ok:true,json:async()=>({monitor:{records:[[0x1393,0,'2a']],revision:2}})});await flush();
  assert.equal(h.element('monitorLevelValue').value,'-42 dB');
  assert.equal(h.ui.busy(),false);
});
test('one failed background read stays editable, sustained failure pauses edits, success recovers',async()=> {
  const h=harness();await h.ready();
  h.tick();h.reads.shift().reject(Error('timeout'));await flush();assert.equal(h.control.disabled,false);
  h.tick();h.reads.shift().reject(Error('timeout'));await flush();assert.equal(h.control.disabled,false);
  h.tick();h.reads.shift().reject(Error('timeout'));await flush();assert.equal(h.control.disabled,true);
  h.tick();h.reads.shift().resolve(fixture());await flush();assert.equal(h.control.disabled,false);
});
test('failed write readback pauses edits immediately even with a recent snapshot',async()=> {
  const h=harness();await h.ready();h.event('change');await flush();
  h.writes[0].resolve({ok:true,json:async()=>({acknowledged:1})});await flush();
  h.reads.shift().reject(Error('timeout'));await flush();assert.equal(h.control.disabled,true);
  assert.match(h.status.textContent,/readback is unavailable/);
});
test('Outputs adopts the shared snapshot without requesting an extra inventory',async()=> {
  const h=harness();await h.ready();h.ui.activate('outputs');
  const snapshot=fixture();snapshot.records.find(r=>r[0]===0x13b6)[2]='03';
  h.ui.outputs(snapshot,'simulated-device',h.ui.outputRevision());
  assert.equal(h.reads.length,0);
  assert.equal(h.element('monitorControls').innerHTML,'Monitor 03');
});

test('an Outputs read started before a monitor write cannot overwrite its readback',async()=> {
  const h=harness();await h.ready();h.ui.activate('outputs');
  const revision=h.ui.outputRevision();
  h.control.dataset={control:'monitor-level',target:'monitor',index:'0'};h.control.value='-31';
  h.event('change');await new Promise(resolve=>setTimeout(resolve,80));
  assert.equal(h.writes[0].form.get('changes'),'monitor-level:monitor:0:1e:-31');
  h.writes[0].resolve({ok:true,json:async()=>({acknowledged:1,monitor:{records:[[0x1393,0,'1f'],[0x13b6,0,'01']],revision:1}})});await flush();
  h.ui.activate('outputs');
  h.ui.outputs(fixture(),'simulated-device',revision);
  assert.equal(h.element('monitorControls').innerHTML,'Monitor 01');
  h.ui.outputs(fixture(),'old-device',h.ui.outputRevision());
  assert.equal(h.element('monitorControls').innerHTML,'Monitor 01');
});

test('versioned Outputs polling recovers monitor state if SSE pauses without regressing newer events',async()=>{
  const h=harness();await h.ready();h.ui.activate('outputs');
  h.ui.monitorState({records:[[0x13b6,0,'01']],revision:1},true);
  h.ui.outputs(fixture(),'simulated-device',h.ui.outputRevision(),{records:[[0x13b6,0,'02']],revision:2});
  assert.equal(h.element('monitorControls').innerHTML,'Monitor 02');
  h.ui.monitorState({records:[[0x13b6,0,'04']],revision:3},true);
  h.ui.outputs(fixture(),'simulated-device',h.ui.outputRevision(),{records:[[0x13b6,0,'02']],revision:2});
  assert.equal(h.element('monitorControls').innerHTML,'Monitor 04');
});

test('monitor readback corrects a drag preview even if pointer release was lost',async()=>{
  const h=harness();await h.ready();h.ui.activate('outputs');
  const slider=h.element('monitorLevel');
  Object.assign(slider,{id:'monitorLevel',type:'range',dataset:{control:'monitor-level',target:'monitor',index:'0'},setAttribute(){}});
  h.document.activeElement=slider;
  h.event('pointerdown',slider);
  slider.value='-31';h.event('input',slider);
  await new Promise(resolve=>setTimeout(resolve,80));
  assert.equal(h.writes[0].form.get('changes'),'monitor-level:monitor:0:1e:-31');
  // Device readback is still -30. No pointerup is delivered (capture/focus loss).
  h.writes[0].resolve({ok:true,json:async()=>({acknowledged:1,monitor:{records:[[0x1393,0,'1e']],revision:2}})});
  await flush();
  assert.equal(h.ui.busy(),false);
  assert.equal(slider.value,'-30','settled device state must replace the stale -31 preview');
  assert.equal(h.element('monitorLevelValue').value,'-30 dB');
  h.ui.monitorState({records:[[0x1393,0,'20']],revision:3},true);
  assert.equal(slider.value,'-32','holding focus must not block future device values');
});

test('changing devices cancels an edit waiting for a background read',async()=> {
  const h=harness();await h.ready();h.tick();h.event('change');
  h.element('host').value='another-device';h.element('host').change();
  h.reads.shift().resolve(fixture());await flush();
  assert.equal(h.writes.length,0);
  h.reads.shift().resolve(fixture());await flush();assert.equal(h.control.disabled,false);
});

test('persistent monitoring follows device events and recovery snapshots outside Outputs', async()=> {
  const h=harness();await h.ready();
  for(const view of ['inputs','patchbay','routing','mixer','aux','diagnostics']) {
    h.ui.activate(view);
    h.ui.monitorState({records:[[0x1393,0,'20']],revision:2},true);
    assert.equal(h.element('monitorLevel').value,'-32',view);
  }
  h.ui.outputs(fixture(),'simulated-device',h.ui.outputRevision(),{records:[[0x1393,0,'28']],revision:3});
  assert.equal(h.element('monitorLevel').value,'-40','recovery poll works in Diagnostics');
  assert.equal(h.writes.length,0,'navigation and recovery never write hardware');
});
