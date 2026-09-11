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
  const control = { dataset:{control:'level',target:'main',index:'0'},type:'range',value:'-18',disabled:true,getAttribute:()=> 'Input level',closest:()=> ({}) };
  const element = id => {
    if (!elements.has(id)) elements.set(id,{ value: id==='host' ? 'simulated-device' : '',dataset:{},innerHTML:'',textContent:'',checked:false,addEventListener(type,fn){this[type]=fn;} });
    return elements.get(id);
  };
  const status = element('status');
  const panel = {dataset:{},querySelectorAll:()=> [control]};
  const document = {hidden:false,activeElement:null,getElementById:element,
    querySelector:()=> ({dataset:{tab:'mixer'}}),
    querySelectorAll:selector=>selector==='.console-panel'?[panel]:selector==='.console-status'?[status]:[],
    addEventListener(type,fn){(events[type]??=[]).push(fn);}};
  let now = 100000;
  const context = { CueMixModel:M,document,localStorage:{getItem:()=>null,setItem(){}},
    Date:class extends Date {static now(){return now;}},
    setInterval:fn=>ticks.push(fn),setTimeout,
    escapeHtml:String,meterScaleMarkup:()=>'',lastMeterRecords:[],loadMeters(){},
    qs:o=>new URLSearchParams(o).toString(),sessionToken:'test-token',
    fetchJson:()=>new Promise((resolve,reject)=>reads.push({resolve,reject})),
    fetch:(url,options)=>new Promise(resolve=>writes.push({form:new URLSearchParams(options.body),resolve})),
  };
  vm.runInNewContext(fs.readFileSync(require.resolve('../src/console.js'),'utf8'),context);
  return {control,document,element,status,reads,writes,
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
  assert.equal(h.control.disabled,true,'remain locked through write readback');
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
test('changing devices cancels an edit waiting for a background read',async()=> {
  const h=harness();await h.ready();h.tick();h.event('change');
  h.element('host').value='another-device';h.element('host').change();
  h.reads.shift().resolve(fixture());await flush();
  assert.equal(h.writes.length,0);
  h.reads.shift().resolve(fixture());await flush();assert.equal(h.control.disabled,false);
});
