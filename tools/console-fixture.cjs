// Local, synthetic device for browser verification. Never contacts hardware.
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
require('../src/console_model.js');
const M = globalThis.CueMixModel;
function fixture() {
  const records = [];
  const add = (p,i,v) => records.push([p,i,v]);
  const name = s => Buffer.from(s).toString('hex');
  for (let i=0;i<128;i++) {
    const banked=(Math.floor(i/8)<<8)|(i%8);
    add(0x8023,banked,i===127 ? name('Playback 128') : '');
    add(0x802b,banked,i===127 ? name('Record 128') : '');
    add(0x8024,i,''); add(0x802c,i,'');
    add(0x93b0,i,'00000000'); add(0x93af,banked,'00000000');
  }
  for(let i=0;i<16;i++) {
    const banked=(Math.floor(i/8)<<8)|(i%8);
    add(0x8022,banked,''); add(0x802a,banked,''); add(0x93ae,banked,'00000000');
    add(0x1b5b,i,'000177000800');
  }
  for(let i=0;i<4;i++) {
    add(0x8025,i,i===0 ? name('Vocal <1>') : '');
    add(0x03e8,i,i===0 ? '01' : '00');
    add(0x03fa,i,'00');add(0x03fb,i,'00');
    add(0x93ad,i,M.path(0x138c,0,i));
    for(const p of [0x841a,0x842e]) add(p,i<<8,M.encoded('level',-12));
    for(const p of [0x842b,0x843f]) add(p,i<<8,'00800000');
    for(let b=0;b<26;b++) { add(0x83f8,(i<<8)|b,'00000000'); add(0x83f9,(i<<8)|b,'00800000'); }
    add(0x93b1,(Math.floor(i/2)<<8)|(i%2),M.path(0x13ad,2,i%2));
  }
  for(let i=0;i<8;i++) add(0x8020,i,'');
  for(let i=0;i<12;i++) {add(0x93ac,i,M.path(0x13ad,1,i%2));add(0x1388,i,'00');}
  add(0x13b6,0,'00');add(0x1393,0,'1e');add(0x1394,0,'0003');add(0x139b,0,'00');
  add(0x13a3,0,'00');
  add(0x139a,0,'00');
  add(0x93b9,0,'00000000');add(0x93b9,1,'00000000');
  for(let i=0;i<26;i++) {add(0x0403,i,'01000000');add(0x0404,i,'00');add(0x0411,i,'01');add(0x03e9,i,i%2===0?'01':'00');}
  for(let i=0;i<2;i++) {
    for(const p of [0x0420,0x0434,0x0448]) add(p,i,'01000000');
    for(const p of [0x0421,0x0435,0x043c]) add(p,i,'00');
  }
  return {records};
}
module.exports = { fixture };
if(require.main===module) {
  const browserFixture=()=> {
    const state=fixture();
    for(let i=4;i<24;i++) {
      state.records.push([0x03e8,i,'00'],[0x03fa,i,'00'],[0x03fb,i,'00'],[0x93ad,i,M.path(i<12?0x13ac:0x13b0,0,i<12?i-4:i-12)]);
      for(const p of [0x841a,0x842e]) state.records.push([p,i<<8,M.encoded('level',-12-(i%4)*3)]);
      for(const p of [0x842b,0x843f]) state.records.push([p,i<<8,'00800000']);
      for(let b=0;b<26;b++) state.records.push([0x83f8,(i<<8)|b,M.encoded('level',-24)],[0x83f9,(i<<8)|b,'00800000']);
    }
    return state;
  };
  const banks={};
  for(const [bank,count] of [['/datastore/ext/ibank/0',4],['/datastore/ext/ibank/1',8],['/datastore/ext/obank/0',12]]) {
    const data=banks[bank]={maxCh:count};
    for(let i=0;i<count;i++) Object.assign(data,{[`ch/${i}/name`]:bank.endsWith('ibank/0')?['Vocal','Guitar','Room L','Room R'][i]:'', [`ch/${i}/trim`]:bank.endsWith('ibank/0')?28:0,[`ch/${i}/trimRange`]:'0:74',[`ch/${i}/48V`]:0,[`ch/${i}/pad`]:0,[`ch/${i}/phase`]:0});
  }
  let snapshot=browserFixture(), requests=[], failNext=false, monitorRevision=1;
  const phoneLevels=[[32,32],[40,40]];
  const monitor=()=>({records:snapshot.records.filter(r=>[0x1393,0x1394,0x139a,0x139b,0x13a3,0x13b6].includes(r[0])),revision:monitorRevision});
  const root=path.resolve(__dirname,'..');
  const page=()=> {
    let html=fs.readFileSync(path.join(root,'src/ui.html'),'utf8');
    for(const [marker,file] of [['CSS','console.css'],['PANELS','console_panels.html'],['MODEL','console_model.js'],['JS','console.js']]) html=html.replace('__CONSOLE_'+marker+'__',fs.readFileSync(path.join(root,'src',file),'utf8'));
    html=html.replace('__MONITOR_JS__',fs.readFileSync(path.join(root,'src/monitor.js'),'utf8'));
    html=html.replace('__SLIDER_QUEUE_JS__',fs.readFileSync(path.join(root,'src/slider_queue.js'),'utf8'));
    html=html.replace('__DB_ENTRY_JS__',fs.readFileSync(path.join(root,'src/db_entry.js'),'utf8'));
    html=html.replace('__CONSOLE_SHELL_CSS__',fs.readFileSync(path.join(root,'src/console_shell.css'),'utf8')).replace('__LAYOUT_JS__',fs.readFileSync(path.join(root,'src/ui_layout.js'),'utf8'));
    return html.replaceAll('__DEFAULT_HOST__','simulated-device').replaceAll('__SESSION_TOKEN__','fixture-token').replace('__DEVICE_HOME_LINK__','').replace('CONTROL CONSOLE','SIMULATED DEVICE');
  };
  http.createServer((req,res)=> {
    const url=new URL(req.url,'http://127.0.0.1');
    const json=(code,data)=>{res.writeHead(code,{'Content-Type':'application/json','Cache-Control':'no-store'});res.end(JSON.stringify(data));};
    if(req.method==='GET') {
      if(url.pathname==='/') {res.writeHead(200,{'Content-Type':'text/html'});return res.end(page());}
      if(url.pathname==='/api/console') return json(200,snapshot);
      if(url.pathname==='/api/outputs') return json(200,{console:snapshot,monitor:monitor(),line_outputs:Array.from({length:12},(_,i)=>({number:i+1,channel_index:i,attenuation:parseInt(snapshot.records.find(r=>r[0]===0x1388&&r[1]===i)[2],16),meter_path:{property_id:'13ad',record_index:1,channel_index:i%2}})),headphone_outputs:phoneLevels.map((attenuation,i)=>({number:i+1,channel_indices:[i*2,i*2+1],attenuation,meter_path:{property_id:'13ad',record_index:1,channel_index:0}}))});
      if(url.pathname==='/api/inputs/lines') return json(200,{inputs:Array.from({length:8},(_,i)=>({number:i+5,channel_index:i,gain_db:banks['/datastore/ext/ibank/1'][`ch/${i}/trim`],phase:Boolean(banks['/datastore/ext/ibank/1'][`ch/${i}/phase`])}))});
      if(url.pathname==='/fixture/requests') return json(200,requests);
      if(url.pathname==='/api/mixer/meters/events') {
        res.writeHead(200,{'Content-Type':'text/event-stream'});
        const record=(property_id,index,count)=>{const channels=Array.from({length:count},(_,i)=>Math.round(30+i%5*6+8*Math.sin(Date.now()/900+i)));return {property_id,index,channels,values:Array.from({length:Math.ceil(count/2)},(_,i)=>(channels[i*2]<<8)|(channels[i*2+1]??255))};};
        const tick=()=>res.write('event: meters\ndata: '+JSON.stringify({status:'ok',age_ms:0,error:null,monitor:monitor(),faders:{},records:[record('138c',0,4),record('13ac',0,8),record('13ad',0,24),record('13ad',1,2),record('13ad',3,26),record('13ad',4,2)]})+'\n\n');
        tick();const timer=setInterval(tick,60);req.on('close',()=>clearInterval(timer));return;
      }
      if(url.pathname==='/api/get') return json(200,{body:JSON.stringify(banks[url.searchParams.get('path')]||{})});
      return json(200,{});
    }
    let body='';req.on('data',c=>body+=c);req.on('end',()=> {
      if(url.pathname==='/fixture/reject-next') {failNext=true;return json(200,{});}
      if(url.pathname==='/fixture/reset') {snapshot=browserFixture();requests=[];return json(200,{});}
      if(url.pathname==='/fixture/monitor') {snapshot.records.find(r=>r[0]===0x1393)[2]='28';monitorRevision++;return json(200,monitor());}
      const form=new URLSearchParams(body);
      if(form.get('host')!=='simulated-device'||form.get('token')!=='fixture-token') return json(403,{error:'Invalid fixture identity'});
      if(failNext) {failNext=false;return json(409,{error:'Simulated device conflict; refresh and review.'});}
      if(url.pathname==='/api/set') {
        const field=form.get('path'), bank=Object.keys(banks).find(key=>field.startsWith(key+'/'));
        if(!bank) return json(400,{error:'Unknown fixture datastore path'});
        const key=field.slice(bank.length+1);banks[bank][key]=key.endsWith('/name')?form.get('value'):Number(form.get('value'));
        requests.push({path:field,value:form.get('value')});return json(200,{body:'ok'});
      }
      if(url.pathname==='/api/inputs/gain') {
        const bank=form.get('bank'), input=Number(form.get('input')), gain=Number(form.get('gain_db'));
        if(!['mic','line'].includes(bank)||!Number.isInteger(input)||input<0||input>=(bank==='mic'?4:8)||!Number.isInteger(gain)||gain<0||gain>(bank==='mic'?74:20)) return json(400,{error:'Invalid input gain'});
        banks[`/datastore/ext/ibank/${bank==='mic'?0:1}`][`ch/${input}/trim`]=gain;
        requests.push({path:url.pathname,bank,input,gain_db:gain});return json(200,{status:200,body:'Input gain verified'});
      }
      if(url.pathname==='/api/outputs/line-trim'||url.pathname==='/api/outputs/headphone-trim') {
        const i=Number(form.get('output')), attenuation=form.get('trim_db')==='-inf'?100:-Number(form.get('trim_db'));
        if(url.pathname.endsWith('line-trim')) snapshot.records.find(r=>r[0]===0x1388&&r[1]===i)[2]=attenuation.toString(16).padStart(2,'0');
        else phoneLevels[i]=[attenuation,attenuation];
        requests.push({path:url.pathname,output:i,attenuation});return json(200,{body:'ok'});
      }
      if(url.pathname==='/api/inputs/line-phase') {
        const i=Number(form.get('input'));banks['/datastore/ext/ibank/1'][`ch/${i}/phase`]=Number(form.get('phase'));
        requests.push({path:url.pathname,input:i,phase:form.get('phase')});return json(200,{});
      }
      if(url.pathname!=='/api/console/changes') return json(400,{error:'Not a simulated console operation'});
      const edits=form.get('changes').split(';').map(c=>c.split(':'));
      const model=M.create(snapshot);
      if(edits.some(([op,target,i,old])=>model.records.get(M.address(op,target,Number(i)))!==old)) return json(409,{error:'Simulated conflict'});
      for(const [op,target,i,old,value] of edits) {
        const id=M.address(op,target,Number(i)), record=snapshot.records.find(r=>M.key(r[0],r[1])===id);
        if(!record) return json(400,{error:'Unavailable simulated control'});
        record[2]=M.encoded(op,value);
      }
      requests.push(edits);monitorRevision++;json(200,{acknowledged:edits.length,monitor:monitor()});
    });
  }).listen(8482,'127.0.0.1',()=>console.log('Simulated console: http://127.0.0.1:8482/#patchbay'));
}
