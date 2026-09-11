const assert = require('node:assert/strict');
const test = require('node:test');
const { fixture } = require('./console-fixture.cjs');
const M = globalThis.CueMixModel;
test('banked source names expose all host and optical channels',()=> {
  const model=M.create(fixture());
  assert.equal(model.sources.filter(s=>s.group==='Computer playback').length,128);
  assert.equal(model.sourceName('13b0007f'),'Playback 128');
  assert.equal(model.sourceName('13ae0107'),'Optical B 8');
  assert.equal(model.sourceName('13af0f07'),'Network 16 · 8');
  assert.equal(new Set(model.sources.map(s=>s.id)).size,model.sources.length);
});
test('destinations preserve sparse indices and join each correct name inventory',()=> {
  const model=M.create(fixture());
  assert.equal(model.destinations.find(d=>d.id==='host:127').name,'Record 128');
  assert.equal(model.destinations.find(d=>d.id==='network:3847').name,'Network Out 16 · 8');
  assert.equal(model.destinations.find(d=>d.id==='optical:263').name,'Optical Out B 8');
  assert.equal(model.destinations.find(d=>d.id==='phones:257').name,'Phones 2 R');
  assert.equal(M.edits(model,'route','network',[3847],'13b0007f')[0].id,'37807:3847');
});
test('linked inputs and auxes use the leading index without dropping mono strips',()=> {
  const model=M.create(fixture());
  assert.deepEqual(model.channels.map(c=>c.members),[[0,1],[2],[3]]);
  assert.deepEqual(model.buses.find(b=>b.id==='aux-24').members,[24,25]);
  assert.equal(model.buses.length,15);
});
test('readback and write encodings agree with captured gains and native pan converter',()=> {
  for(const [db,raw] of [[-12,'00404de6'],[-60,'00004189'],[0,'01000000'],['-inf','00000000']]) assert.equal(M.encoded('level',db),raw);
  assert.ok(Math.abs(M.level('00404de6')+12)<0.00001);
  assert.equal(M.encoded('pan',0),'00800000');
  assert.equal(M.encoded('pan',1),'01000000');
  assert.equal(M.level('00000000'),-Infinity);
  assert.equal(M.level(undefined),null);
});
test('edits keep expected bytes, distinct bus addresses, and correct mute/solo IDs',()=> {
  const model=M.create(fixture());
  assert.equal(M.serialize(M.edits(model,'level','aux-24',[2],-12)),'level:aux-24:2:00000000:-12');
  assert.equal(M.address('level','aux-24',2),M.key(0x83f8,0x0218));
  assert.equal(M.address('mute','input',2),M.key(0x03fb,2));
  assert.equal(M.address('solo','input',2),M.key(0x03fa,2));
  assert.deepEqual(M.edits(model,'level','main',[2],-12),[]);
  assert.throws(()=>M.edits(model,'level','main',[63],0),/unavailable/);
});
test('network media clock and invalid or missing sources cannot be selected',()=> {
  const snapshot=fixture(); snapshot.records.find(r=>r[0]===0x1b5b&&r[1]===15)[2]='000177000000';
  const model=M.create(snapshot);
  assert.ok(!model.sourceMap.has('13af0f00'));
  assert.ok(model.sourceName('deadbeef').startsWith('Unavailable'));
});
test('snapshot validation and name fallback tolerate malformed device labels',()=> {
  assert.throws(()=>M.create(null));
  assert.throws(()=>M.create({records:[[1,0,'00'],[1,0,'00']]}),/Duplicate/);
  assert.throws(()=>M.create({records:[[1,-1,'00']]}),/Invalid/);
  const snapshot=fixture();snapshot.records.find(r=>r[0]===0x8025&&r[1]===0)[2]='fffe';
  assert.equal(M.create(snapshot).sourceName('138c0000'),'Mic / Inst 1');
});
