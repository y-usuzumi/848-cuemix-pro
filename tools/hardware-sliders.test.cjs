// Run the actual hardware slider bindings with an in-memory HTTP transport.
const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const flush = () => new Promise(resolve=>setImmediate(resolve));
function harness() {
  const elements = new Map(), requests = [];
  const element = id => {
    if (!elements.has(id)) elements.set(id,{
      id,value:id==='host'?'simulated-device':'0',dataset:{},disabled:false,hidden:true,
      addEventListener(type,fn){(this.events??={})[type]??=[];this.events[type].push(fn);},
      setAttribute(){},matches:()=>false,
      dispatchEvent(event){this.events[event.type]?.forEach(fn=>fn({target:this}));},
    });
    return elements.get(id);
  };
  const context = {
    document:{getElementById:element,querySelectorAll:()=>[],activeElement:null},
    URLSearchParams,setTimeout,clearTimeout,Event,
    fetch:(url,options)=>new Promise((resolve,reject)=>requests.push({url,form:new URLSearchParams(options?.body),resolve:value=>resolve({ok:true,json:async()=>value??{status:200,body:'saved'}}),reject})),
  };
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(require.resolve('../src/slider_queue.js'),'utf8'),context);
  vm.runInContext(fs.readFileSync(require.resolve('../src/db_entry.js'),'utf8'),context);
  const html=fs.readFileSync(require.resolve('../src/ui.html'),'utf8');
  const script=html.slice(html.indexOf('    const $ ='),html.indexOf("    $('get').addEventListener"));
  vm.runInContext(script,context);
  return {context,element,requests,
    bind(code){vm.runInContext(code,context);},
    event(id,type,value){const node=element(id);if(value!==undefined)node.value=value;node.events[type]?.forEach(fn=>fn({target:node}));},
    async settle(){requests.at(-1).resolve();await flush();},
    stop(){vm.runInContext('hardwareSliders.reset()',context);},
  };
}
test('mic, line, output and headphone input events save without waiting for change',async()=>{
  for(const [bind,id,url,field,value] of [
    ['bindPreamp(0)','mic-0-gain','/api/inputs/gain','gain_db','42'],
    ['bindLineInput(0)','line-in-0-gain','/api/inputs/gain','gain_db','12'],
    ['bindOutput(0, {})','out-0-gain','/api/outputs/line-trim','trim_db','-24'],
    ['bindHeadphone(0)','phone-0-gain','/api/outputs/headphone-trim','trim_db','-40'],
  ]) {
    const h=harness();h.bind(bind);h.event(id,'pointerdown');h.event(id,'input',value);
    await new Promise(resolve=>setTimeout(resolve,100));
    assert.equal(h.requests.length,1,id);assert.equal(h.requests[0].url,url);
    assert.equal(h.requests[0].form.get(field),value);
    assert.equal(h.requests[0].form.get('host'),'simulated-device');
    assert.equal(h.element(id).disabled,false,'keep dragging while the write is pending');
    h.event(id,'input',String(Number(value)+1));h.event(id,'input',String(Number(value)+2));
    await h.settle();await new Promise(resolve=>setTimeout(resolve,100));
    assert.equal(h.requests.length,2,'only the latest unsent position follows');
    assert.equal(h.requests[1].form.get(field),String(Number(value)+2));
    await h.settle();h.stop();
  }
});

test('input gain bindings send the bank and advertised channel through the persistent API',async()=>{
  for(const [bind,id,bank,index] of [
    ['bindPreamp(2)','mic-2-gain','mic','2'],
    ['lineInputChannels=[3,6];bindLineInput(1)','line-in-1-gain','line','6'],
  ]) {
    const h=harness();h.bind(bind);h.event(id,'input','12');
    await new Promise(resolve=>setTimeout(resolve,100));
    assert.equal(h.requests.length,1);
    assert.equal(h.requests[0].url,'/api/inputs/gain');
    assert.equal(h.requests[0].form.get('bank'),bank);
    assert.equal(h.requests[0].form.get('input'),index);
    await h.settle();h.stop();
  }
});
test('an acknowledged HTTP error ends a gain gesture without an automatic retry on release',async()=>{
  const h=harness();h.bind('bindPreamp(0)');h.event('mic-0-gain','input','42');
  await new Promise(resolve=>setTimeout(resolve,100));
  h.event('mic-0-gain','input','43');h.requests[0].resolve({status:500});await flush();
  h.event('mic-0-gain','change');await new Promise(resolve=>setTimeout(resolve,100));
  assert.equal(h.requests.length,1);assert.match(h.element('mic-0-status').textContent,/stopped.*HTTP 500/);
  h.stop();
});

test('typed levels use the same hardware queues and endpoints, including output silence',async()=>{
  for(const [bind,id,url,field,value,expected] of [
    ['bindPreamp(0)','mic-0-gain','/api/inputs/gain','gain_db','42','42'],
    ['bindLineInput(0)','line-in-0-gain','/api/inputs/gain','gain_db','12','12'],
    ['bindOutput(0, {})','out-0-gain','/api/outputs/line-trim','trim_db','-24','-24'],
    ['bindHeadphone(0)','phone-0-gain','/api/outputs/headphone-trim','trim_db','-100','-inf'],
  ]) {
    const h=harness();h.bind(bind);h.event(id,'dbcommit',value);await flush();
    assert.equal(h.requests.length,1,id);assert.equal(h.requests[0].url,url);
    assert.equal(h.requests[0].form.get(field),expected);
    assert.equal(h.requests[0].form.get('token'),'__SESSION_TOKEN__');
    h.stop();await h.settle();
  }
});
