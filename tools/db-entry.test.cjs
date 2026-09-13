const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
function harness({min='-100',max='0',step='1',value='-30',silent='-100',finiteMin,unit}={}) {
  const events={},commits=[],errors=[];
  const slider={id:'gain',min,max,step,value,disabled:false,isConnected:true,dispatchEvent(){commits.push(this.value);}};
  const input={value:`${value} dB`,dataset:{dbFor:'gain',...(silent===null?{}:{dbInfinity:silent}),...(finiteMin?{dbMin:finiteMin}:{}),...(unit?{dbUnit:unit}:{})},
    select(){},setCustomValidity(message){this.validity=message;},reportValidity(){this.reported=true;},blur(){document.activeElement=null;emit('focusout');}};
  const document={activeElement:null,getElementById:id=>id==='gain'?slider:null,addEventListener(type,fn){events[type]=fn;}};
  const context={document,Event};vm.createContext(context);
  vm.runInContext(fs.readFileSync(require.resolve('../src/db_entry.js'),'utf8'),context);
  const db=context.CueMixDb;db.mount(document,message=>errors.push(message));
  const emit=(type,key)=>events[type]?.({target:input,key,preventDefault(){}});
  return {db,input,slider,commits,errors,document,emit,
    focus(){document.activeElement=input;emit('focusin');},
    type(value){input.value=value;emit('input');},
    blur(){input.blur();},key:key=>emit('keydown',key)};
}

test('typing waits for Enter or blur, and Enter followed by blur commits once',()=>{
  for(const finish of [h=>h.key('Enter'),h=>h.blur()]) {
    const h=harness();h.focus();assert.equal(h.input.value,'-30');h.type('-24 dB');
    assert.deepEqual(h.commits,[]);assert.equal(h.slider.value,'-30');
    finish(h);assert.deepEqual(h.commits,['-24']);assert.equal(h.input.value,'-24 dB');
    h.blur();assert.deepEqual(h.commits,['-24']);
  }
});
test('Escape and untouched or unchanged fields never send a value',()=>{
  const h=harness();h.focus();h.type('-6');h.key('Escape');
  assert.equal(h.input.value,'-30 dB');assert.equal(h.slider.value,'-30');
  h.focus();h.blur();h.focus();h.type('-30');h.key('Enter');
  assert.deepEqual(h.commits,[]);
});
test('polling preserves a draft and cancellation restores the latest device readout',()=>{
  const h=harness();h.focus();h.type('-2');h.slider.value='-40';h.db.sync(h.input,'-40 dB');
  assert.equal(h.input.value,'-2');assert.equal(h.db.editing(h.slider),true);
  h.key('Escape');assert.equal(h.input.value,'-40 dB');assert.equal(h.db.editing(h.slider),false);
});
test('whole-dB gain and attenuation limits reject invalid values without clamping or sending',()=>{
  for(const config of [{min:'0',max:'74',silent:null},{min:'0',max:'20',silent:null},{}]) {
    for(const value of ['', ' ', 'garbage', 'Infinity', 'NaN', '0x10', '-1e1', '1.5', '75', '-101']) {
      const h=harness(config);h.focus();h.type(value);h.key('Enter');
      assert.equal(h.commits.length,0,value);assert.equal(h.errors.length,1,value);assert.equal(h.input.reported,true,value);
      h.blur();assert.equal(h.commits.length,0);assert.equal(h.input.value,'-30 dB');
    }
  }
});
test('valid decimal mix levels and explicit silence follow the range contract',()=>{
  for(const [value,expected] of [['−12.5 dB','-12.5'],['+12','12'],['-90','-90'],['-inf','-91'],['−∞','-91']]) {
    const h=harness({min:'-91',max:'12',step:'0.1',silent:'-91',finiteMin:'-90',unit:'none'});
    h.focus();h.type(value);h.key('Enter');assert.deepEqual(h.commits,[expected]);
  }
  for(const value of ['-90.5','-12.55','12.1']) {
    const h=harness({min:'-91',max:'12',step:'0.1',silent:'-91',finiteMin:'-90'});
    h.focus();h.type(value);h.key('Enter');assert.equal(h.commits.length,0,value);
  }
  for(const value of ['-inf','−∞','-100']) {
    const h=harness();h.focus();h.type(value);h.key('Enter');assert.deepEqual(h.commits,['-100']);assert.equal(h.input.value,'−∞ dB');
  }
  const gain=harness({min:'0',max:'74',silent:null});gain.focus();gain.type('-inf');gain.key('Enter');assert.equal(gain.commits.length,0);
});
test('a disabled or removed slider cannot be edited through its number',()=>{
  for(const state of [{disabled:true},{isConnected:false}]) {
    const h=harness();h.focus();h.type('-12');Object.assign(h.slider,state);h.key('Enter');assert.equal(h.commits.length,0);
  }
});
test('mixed headphone levels remain untouched on focus but accept explicitly setting both channels to the left value',()=>{
  const h=harness();h.input.value='L -30 dB / R -40 dB';h.input.dataset.dbMixed='true';
  h.focus();h.blur();assert.equal(h.input.value,'L -30 dB / R -40 dB');assert.equal(h.commits.length,0);
  h.focus();h.type('-30');h.key('Enter');assert.deepEqual(h.commits,['-30']);assert.equal(h.input.value,'-30 dB');
});
