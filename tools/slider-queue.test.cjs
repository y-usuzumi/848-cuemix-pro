const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const flush = () => new Promise(resolve => setImmediate(resolve));
function harness() {
  const timers = new Map(), writes = [], errors = [];
  let id = 0, ready = true;
  const context = { setTimeout:fn=>{timers.set(++id,fn);return id;}, clearTimeout:id=>timers.delete(id) };
  vm.runInNewContext(fs.readFileSync(require.resolve('../src/slider_queue.js'),'utf8'), context);
  const queue = context.CueMixSliders.createQueue({
    ready:()=>ready,
    send:payload=>new Promise((resolve,reject)=>writes.push({payload,resolve,reject})),
    onError:(_,error)=>errors.push(error.message),
  });
  return {queue,writes,errors,timers,setReady:value=>{ready=value;},
    input(value,final=false,key='gain'){queue.enqueue(key,value,{key,value},final);},
    tick(){const callbacks=[...timers.values()];timers.clear();callbacks.forEach(fn=>fn());},
  };
}
test('continuous input saves before release, coalesces a slow write, and flushes the final position',async()=>{
  const h=harness();h.input(1);h.input(2);h.input(3);
  assert.equal(h.timers.size,1);h.tick();assert.equal(h.writes[0].payload.value,3);
  for(let value=4;value<=100;value++)h.input(value);
  h.input(101,true);assert.equal(h.writes.length,1,'only one in flight');
  h.writes[0].resolve();await flush();h.tick();
  assert.equal(h.writes.length,2);assert.equal(h.writes[1].payload.value,101);
  h.writes[1].resolve();await flush();h.input(101,true);h.tick();
  assert.equal(h.writes.length,2,'mouseup must not duplicate the settled value');
  assert.equal(h.queue.busy(),false);
});
test('reversing to the in-flight value discards an obsolete buffered position',async()=>{
  const h=harness();h.input(1,true);h.input(2);h.input(1,true);
  h.writes[0].resolve();await flush();h.tick();assert.equal(h.writes.length,1);
});
test('waits for competing work and keeps separate sliders bounded',async()=>{
  const h=harness();h.setReady(false);h.input(1);h.tick();h.input(2);h.input(3,false,'phones');h.tick();
  assert.equal(h.writes.length,0);h.setReady(true);h.tick();
  assert.deepEqual(h.writes[0].payload,{key:'gain',value:2});
  h.writes[0].resolve();await flush();h.tick();
  assert.deepEqual(h.writes[1].payload,{key:'phones',value:3});
  h.writes[1].resolve();await flush();
});
test('failure cancels buffered positions and release; only a fresh gesture can retry',async()=>{
  const h=harness();h.input(1,true);h.input(2);h.writes[0].reject(Error('conflict'));await flush();
  h.input(3);h.input(3,true);h.tick();assert.equal(h.writes.length,1);
  assert.ok(h.errors.includes('conflict'));
  h.queue.begin('gain');h.input(3,true);assert.equal(h.writes.length,2);
  h.writes[1].resolve();await flush();
});
test('device reset cancels unsent work and ignores old completion when deduplicating',async()=>{
  const h=harness();h.input(1,true);h.input(2);h.queue.reset();h.input(1,true);
  assert.equal(h.writes.length,1);h.writes[0].resolve();await flush();h.tick();
  assert.equal(h.writes.length,2,'the new device still needs its requested value');
  h.writes[1].resolve();await flush();h.input(2);h.queue.reset();h.tick();
  assert.equal(h.writes.length,2,'unsent old-device value is cancelled');
});
