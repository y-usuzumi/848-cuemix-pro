const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const flush = () => new Promise(resolve => setImmediate(resolve));

const historyKey = 'cuemix-848-recent-connections';
function memoryStorage(saved = '[]') {
  const values = new Map([[historyKey, saved]]);
  return {getItem:key=>values.get(key) ?? null, setItem:(key,value)=>values.set(key,value), removeItem:key=>values.delete(key)};
}
function harness(storage = memoryStorage()) {
  const nodes = new Map(), requests = [], destinations = [];
  let created = 0;
  const node = id => {
    if (!nodes.has(id)) nodes.set(id, {
      value:'', textContent:'', innerHTML:'', disabled:false, hidden:false, children:[], dataset:{}, events:{}, attributes:{},
      addEventListener(event, fn) { this.events[event] = fn; },
      querySelectorAll() { return this.links || this.children; },
      replaceChildren(...children) { this.children = children; },
      append(...children) { this.children.push(...children); },
      closest() { return this; },
      setAttribute(name, value) { this.attributes[name] = value; },
      emit(event, target=this) { return this.events[event]({preventDefault(){},target}); }
    });
    return nodes.get(id);
  };
  const context = {URLSearchParams};
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(require.resolve('../src/home.js'),'utf8'),context);
  context.CueMixHome.mount({getElementById:node,createElement:()=>node(`created-${++created}`)},'test-token',{
    storage,
    request:(url, options)=>new Promise((resolve,reject)=>requests.push({url,form:new URLSearchParams(options.body),method:options.method,
      resolve(body,status=200){resolve({ok:status<400,status,json:async()=>body});},reject})),
    navigate:url=>destinations.push(url)
  });
  return {node,requests,destinations,storage};
}

test('manual connect sends explicit intent once and navigates only after success', async()=>{
  const h=harness();
  assert.equal(h.requests.length,0,'loading the home page does not connect to a device');
  h.node('deviceAddress').value=' 192.168.1.50 ';
  h.node('manualConnect').emit('submit');
  h.node('manualConnect').emit('submit');
  assert.equal(h.requests.length,1);
  assert.equal(h.requests[0].url,'/api/connect');
  assert.equal(h.requests[0].method,'POST');
  assert.equal(h.requests[0].form.get('token'),'test-token');
  assert.equal(h.requests[0].form.get('host'),'192.168.1.50');
  assert.equal(h.node('connectDevice').disabled,true);
  assert.equal(h.node('scanDevices').disabled,true);
  assert.deepEqual(h.destinations,[]);
  h.requests[0].resolve({host:'192.168.1.50'});await flush();
  assert.deepEqual(h.destinations,['/?host=192.168.1.50']);
});

test('failed connections keep the typed address, show the error, and allow a fresh attempt',async()=>{
  const h=harness();h.node('deviceAddress').value='192.168.1.50';
  h.node('manualConnect').emit('submit');
  h.requests[0].resolve({error:'Could not connect: timed out'},502);await flush();
  assert.deepEqual(h.destinations,[]);
  assert.match(h.node('connectStatus').textContent,/timed out/);
  assert.equal(h.node('deviceAddress').value,'192.168.1.50');
  assert.equal(h.node('connectDevice').disabled,false);
  h.node('manualConnect').emit('submit');
  assert.equal(h.requests.length,2);
  h.requests[1].reject(new Error('Network unavailable'));await flush();
  assert.match(h.node('connectStatus').textContent,/Network unavailable/);
  assert.equal(h.node('connectDevice').textContent,'Connect');
});

test('discovered device selection handles nested clicks and scoped IPv6 links',async()=>{
  const h=harness(),link=h.node('discovered');
  link.dataset.deviceHost='[fe80::1%eth2]';h.node('deviceList').links=[link];
  h.node('deviceList').emit('click',{closest:()=>link});
  assert.equal(h.requests[0].form.get('host'),'[fe80::1%eth2]');
  assert.equal(link.attributes['aria-disabled'],'true');
  h.requests[0].resolve({host:'[fe80::1%eth2]'});await flush();
  assert.equal(h.destinations[0],'/?host=%5Bfe80%3A%3A1%25eth2%5D');
});

test('rescanning replaces the device list without clearing a manual address',async()=>{
  const h=harness();h.node('deviceAddress').value='192.168.1.50';
  const first=h.node('scanDevices').emit('click');
  h.node('scanDevices').emit('click');
  assert.equal(h.requests.length,1);
  assert.equal(h.requests[0].url,'/api/discover');
  assert.equal(h.requests[0].form.get('token'),'test-token');
  assert.equal(h.node('scanDevices').textContent,'Scanning…');
  h.requests[0].resolve({html:'<article>848</article>',error:null});await first;
  assert.equal(h.node('deviceList').innerHTML,'<article>848</article>');
  assert.equal(h.node('deviceAddress').value,'192.168.1.50');
  assert.equal(h.node('scanDevices').disabled,false);
  const second=h.node('scanDevices').emit('click');
  h.requests[1].resolve({html:'<article>848</article>',error:'No multicast interface'});await second;
  assert.match(h.node('scanError').textContent,/still connect by IP/);
  assert.equal(h.node('connectDevice').disabled,false);
});

test('finishing a scan during a connection keeps selection disabled',async()=>{
  const h=harness(),link=h.node('discovered');h.node('deviceList').links=[link];
  const scan=h.node('scanDevices').emit('click');
  h.node('deviceAddress').value='192.168.1.50';h.node('manualConnect').emit('submit');
  h.requests[0].resolve({html:'new devices',error:null});await scan;
  assert.equal(h.node('scanDevices').disabled,true);
  assert.equal(link.attributes['aria-disabled'],'true');
  h.requests[1].resolve({error:'Offline'},502);await flush();
  assert.equal(h.node('scanDevices').disabled,false);
  assert.equal(link.attributes['aria-disabled'],'false');
});

test('successful manual connections persist the canonical address and reload without connecting',async()=>{
  const h=harness();
  assert.equal(h.node('recentConnections').hidden,true);
  h.node('deviceAddress').value='192.168.1.50:80';h.node('manualConnect').emit('submit');
  assert.equal(h.storage.getItem(historyKey),'[]','pending connections are not saved');
  h.requests[0].resolve({host:'192.168.1.50'});await flush();
  const restored=harness(h.storage);
  assert.equal(restored.requests.length,0);
  assert.equal(restored.node('recentConnections').hidden,false);
  const button=restored.node('recentList').children[0];
  assert.equal(button.dataset.recentHost,'192.168.1.50');
  assert.equal(button.attributes['aria-label'],'Connect to 192.168.1.50');
  assert.equal(button.children[0].textContent,'192.168.1.50');
});

test('history reconnects are validated again and move to the front only after success',async()=>{
  const storage=memoryStorage(JSON.stringify(['192.168.1.51','[fe80::1%eth2]'])), h=harness(storage);
  const link=h.node('recentList').children[1];
  h.node('recentList').emit('click',{closest:()=>link});
  h.node('recentList').emit('click',{closest:()=>link});
  assert.equal(h.requests.length,1);
  assert.equal(h.requests[0].url,'/api/connect');
  assert.equal(h.requests[0].form.get('host'),'[fe80::1%eth2]');
  assert.equal(h.requests[0].form.get('token'),'test-token');
  assert.equal(h.node('deviceAddress').value,'[fe80::1%eth2]');
  assert.equal(link.disabled,true);
  h.requests[0].resolve({error:'Offline'},502);await flush();
  assert.deepEqual(JSON.parse(storage.getItem(historyKey)),['192.168.1.51','[fe80::1%eth2]']);
  assert.equal(link.disabled,false);
  assert.deepEqual(h.destinations,[]);
  h.node('recentList').emit('click',{closest:()=>link});
  h.requests[1].resolve({host:'[fe80::1%eth2]'});await flush();
  assert.deepEqual(JSON.parse(storage.getItem(historyKey)),['[fe80::1%eth2]','192.168.1.51']);
  assert.equal(h.destinations[0],'/?host=%5Bfe80%3A%3A1%25eth2%5D');
});

test('manual history deduplicates canonical addresses and keeps the latest eight successes',async()=>{
  const storage=memoryStorage(JSON.stringify(Array.from({length:8},(_,i)=>`192.168.1.${i+1}`)));
  let h=harness(storage);
  h.node('deviceAddress').value='192.168.1.4:80';h.node('manualConnect').emit('submit');
  h.requests[0].resolve({host:'192.168.1.4'});await flush();
  assert.deepEqual(JSON.parse(storage.getItem(historyKey)),['192.168.1.4','192.168.1.1','192.168.1.2','192.168.1.3','192.168.1.5','192.168.1.6','192.168.1.7','192.168.1.8']);
  h=harness(storage);
  h.node('deviceAddress').value='192.168.1.9';h.node('manualConnect').emit('submit');
  h.requests[0].resolve({host:'192.168.1.9'});await flush();
  assert.deepEqual(JSON.parse(storage.getItem(historyKey)),['192.168.1.9','192.168.1.4','192.168.1.1','192.168.1.2','192.168.1.3','192.168.1.5','192.168.1.6','192.168.1.7']);
});

test('failed manual and successful discovered connections leave manual history unchanged',async()=>{
  const h=harness(memoryStorage('["192.168.1.50"]'));
  h.node('deviceAddress').value='192.168.1.51';h.node('manualConnect').emit('submit');
  h.requests[0].resolve({error:'Offline'},502);await flush();
  assert.equal(h.storage.getItem(historyKey),'["192.168.1.50"]');
  const link=h.node('discovered');link.dataset.deviceHost='192.168.1.52';
  h.node('deviceList').emit('click',{closest:()=>link});
  h.requests[1].resolve({host:'192.168.1.52'});await flush();
  assert.equal(h.storage.getItem(historyKey),'["192.168.1.50"]');
});

test('clear removes persisted history without connecting or clearing the manual input',()=>{
  const h=harness(memoryStorage('["192.168.1.50"]'));
  h.node('deviceAddress').value='192.168.1.51';h.node('clearRecent').emit('click');
  assert.equal(h.storage.getItem(historyKey),null);
  assert.equal(h.node('recentConnections').hidden,true);
  assert.equal(h.node('recentList').children.length,0);
  assert.equal(h.node('deviceAddress').value,'192.168.1.51');
  assert.equal(h.requests.length,0);
  assert.equal(harness(h.storage).node('recentConnections').hidden,true);
});

test('unavailable or corrupt browser storage does not block successful connections',async()=>{
  const blocked={getItem(){throw Error('blocked');},setItem(){throw Error('blocked');},removeItem(){throw Error('blocked');}};
  for(const storage of [blocked,memoryStorage('broken json'),memoryStorage('{"host":"192.168.1.50"}'),null]) {
    const h=harness(storage);
    assert.equal(h.node('recentConnections').hidden,true);
    h.node('deviceAddress').value='192.168.1.50';h.node('manualConnect').emit('submit');
    h.requests[0].resolve({host:'192.168.1.50'});await flush();
    assert.deepEqual(h.destinations,['/?host=192.168.1.50']);
    const restored=harness(storage);restored.node('clearRecent').emit('click');
    assert.equal(restored.node('recentConnections').hidden,true);
  }
});

test('stored values are bounded, filtered, deduplicated, and rendered as text',()=>{
  const h=harness(memoryStorage(JSON.stringify([null,{},42,'','  ','192.168.1.50','192.168.1.50','<img src=x onerror=alert(1)>','x'.repeat(257),...Array.from({length:12},(_,i)=>`192.168.2.${i+1}`)])));
  const buttons=h.node('recentList').children;
  assert.equal(buttons.length,8);
  assert.equal(buttons[0].children[0].textContent,'192.168.1.50');
  assert.equal(buttons[1].children[0].textContent,'<img src=x onerror=alert(1)>');
  assert.equal(buttons[1].children[0].innerHTML,'');
  assert.equal(h.requests.length,0);
});
