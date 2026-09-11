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
  let snapshot=fixture(), requests=[], failNext=false, monitorRevision=1;
  const monitor=()=>({records:snapshot.records.filter(r=>[0x1393,0x1394,0x139a,0x139b,0x13a3,0x13b6].includes(r[0])),revision:monitorRevision});
  const root=path.resolve(__dirname,'..');
  const page=()=> {
    let html=fs.readFileSync(path.join(root,'src/ui.html'),'utf8');
    for(const [marker,file] of [['CSS','console.css'],['PANELS','console_panels.html'],['MODEL','console_model.js'],['JS','console.js']]) html=html.replace('__CONSOLE_'+marker+'__',fs.readFileSync(path.join(root,'src',file),'utf8'));
    html=html.replace('__MONITOR_JS__',fs.readFileSync(path.join(root,'src/monitor.js'),'utf8'));
    return html.replaceAll('__DEFAULT_HOST__','simulated-device').replaceAll('__SESSION_TOKEN__','fixture-token').replace('<body>','<body><p style="text-align:center">SIMULATED DEVICE · No hardware connection</p>');
  };
  http.createServer((req,res)=> {
    const url=new URL(req.url,'http://127.0.0.1');
    const json=(code,data)=>{res.writeHead(code,{'Content-Type':'application/json','Cache-Control':'no-store'});res.end(JSON.stringify(data));};
    if(req.method==='GET') {
      if(url.pathname==='/') {res.writeHead(200,{'Content-Type':'text/html'});return res.end(page());}
      if(url.pathname==='/api/console') return json(200,snapshot);
      if(url.pathname==='/api/outputs') return json(200,{console:snapshot,monitor:monitor(),line_outputs:[],headphone_outputs:[]});
      if(url.pathname==='/fixture/requests') return json(200,requests);
      if(url.pathname==='/api/mixer/meters/events') {
        res.writeHead(200,{'Content-Type':'text/event-stream'});
        const tick=()=>res.write('event: meters\ndata: '+JSON.stringify({status:'ok',age_ms:0,error:null,monitor:monitor(),faders:{},records:[{property_id:'13ad',index:0,channels:[48,48,96,96]},{property_id:'13ad',index:1,channels:[24,24]}]})+'\n\n');
        tick();const timer=setInterval(tick,60);req.on('close',()=>clearInterval(timer));return;
      }
      if(url.pathname==='/api/get') return json(200,{data:{}});
      return json(200,{});
    }
    let body='';req.on('data',c=>body+=c);req.on('end',()=> {
      if(url.pathname==='/fixture/reject-next') {failNext=true;return json(200,{});}
      if(url.pathname==='/fixture/reset') {snapshot=fixture();requests=[];return json(200,{});}
      if(url.pathname==='/fixture/monitor') {snapshot.records.find(r=>r[0]===0x1393)[2]='28';monitorRevision++;return json(200,monitor());}
      if(url.pathname!=='/api/console/changes') return json(400,{error:'Not a simulated console operation'});
      const form=new URLSearchParams(body);
      if(form.get('host')!=='simulated-device'||form.get('token')!=='fixture-token') return json(403,{error:'Invalid fixture identity'});
      if(failNext) {failNext=false;return json(409,{error:'Simulated device conflict; refresh and review.'});}
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
