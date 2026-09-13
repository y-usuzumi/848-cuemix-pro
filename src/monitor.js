/* Monitoring presentation and routing helpers; transport belongs to console.js. */
globalThis.CueMixMonitor = (() => {
  const M = globalThis.CueMixModel;
  const esc = value => escapeHtml(String(value));
  const effective = (model, drafts, d) => drafts.get(M.key(d.property, d.index))?.value ?? d.source;
  const physical = model => model.destinations.filter(d => ['line', 'optical', 'phones', 'network'].includes(d.group));
  function pairs(model) {
    const dests = physical(model);
    return dests.filter(d => d.index % 2 === 0).flatMap(left => {
      const right = dests.find(d => d.group === left.group && d.index === left.index + 1);
      return right ? [{ id: `${left.id},${right.id}`, left, right, name: `${left.name} / ${right.name}` }] : [];
    });
  }
  const selectedNames = mask => ['A', 'B', 'C'].filter((_, i) => mask & (1 << i)).join(' + ') || 'Off';
  const volumeText = value => value === -100 ? '−∞ dB' : `${value} dB`;
  const frontButtonText = (label, state) => `<span class="monitor-action-label">${label}</span><span class="monitor-action-state">${state}</span>`;
  function render(model, drafts, setupOpen = false, section = 'all') {
    if (!model.monitorAvailable) return '<p class="empty">This device does not advertise the mapped ABC monitor controls.</p>';
    const mask = M.number(model.get(0x13b6, 0));
    const attenuation = M.number(model.get(0x1393, 0));
    const validLevel = model.get(0x1393, 0)?.length === 2 && attenuation <= 100;
    const level = validLevel ? -attenuation : -100;
    const members = M.number(model.get(0x1394, 0));
    const lines = model.destinations.filter(d => d.group === 'line' && d.index < 12 && model.get(0x1388, d.index)?.length === 2);
    const allowed = lines.reduce((bits, d) => bits | (1 << d.index), 0);
    const validMembers = model.get(0x1394, 0)?.length === 4 && allowed && !(members & ~allowed);
    const attrs = (op, unavailable = false) => `data-control="${op}" data-target="monitor" data-index="0"${unavailable ? ' data-unavailable="true" disabled' : ''}`;
    const frontButtons = [
      ['monitor-mute', 0x139b, 'Mute', 'Monitor mute', 'Silences the active ABC speakers or Monitor Group. Click again to unmute.'],
      ['monitor-mono', 0x139a, 'Mono', 'Monitor mono', 'Sums the main output pair or active ABC pairs to mono. Click again for stereo.'],
      ['monitor-talk', 0x13a3, 'Talk', 'Talkback', 'Click to talk; click again to stop. Uses the talkback setup in CueMix Pro → Home.'],
    ].map(([operation, property, label, name, help]) => {
      const value = model.get(property, 0), available = /^0[01]$/.test(value), active = value === '01';
      return `<div><button class="secondary" ${attrs(operation, !available)} data-value="${active ? 0 : 1}" aria-label="${name}" aria-describedby="${operation}-help" aria-pressed="${active}">${frontButtonText(label, available ? (active ? 'On' : 'Off') : 'Unavailable')}</button><small id="${operation}-help">${help}</small></div>`;
    }).join('');
    const selectionButton = ([value, name]) => `<button class="secondary" ${attrs('monitor-select')} data-value="${value}" aria-label="${value ? `Select speakers ${name}` : 'Turn ABC off'}" aria-pressed="${mask === value}">${name}</button>`;
    const speakerButtons = [1, 2, 4].map((bit, i) => `<button class="secondary" ${attrs('monitor-select')} data-monitor-toggle="${bit}" data-value="${mask ^ bit}" aria-label="Toggle speakers ${'ABC'[i]}" aria-pressed="${!!(mask & bit)}">${'ABC'[i]}</button>`).join('');
    const quickSelections = [[0, 'Off'], [7, 'All']].map(selectionButton).join('');
    const combinations = [[0, 'Off'], [1, 'A only'], [2, 'B only'], [4, 'C only'], [3, 'A + B'], [5, 'A + C'], [6, 'B + C'], [7, 'All']];
    const combinedSelections = `<select id="monitorCombination" data-monitor-combination ${attrs('monitor-select')} aria-label="Speaker combination">${combinations.map(([value, name]) => `<option value="${value}"${value === mask ? ' selected' : ''}>${name}</option>`).join('')}</select>`;
    const outputs = physical(model), choices = pairs(model);
    const cards = [0, 1, 2].map(bank => {
      const connected = [0, 1].map(channel => outputs.filter(d => effective(model, drafts, d) === M.path(0x13b9, bank, channel)));
      const currentPair = choices.find(p => connected[0].length === 1 && connected[1].length === 1 && connected[0][0].id === p.left.id && connected[1][0].id === p.right.id);
      const selection = currentPair?.id ?? (connected.flat().length ? 'custom' : 'none');
      const summary = connected.map((ds, i) => `<span>${i ? 'R' : 'L'} → ${esc(ds.map(d => d.name).join(', ') || 'Unassigned')}</span>`).join('');
      return `<article class="monitor-speaker${mask & (1 << bank) ? ' is-selected' : ''}"><h3>Speakers ${'ABC'[bank]}${mask & (1 << bank) ? ' · Active' : ''}</h3><div class="monitor-connections">${summary}</div><label>Output pair <select data-monitor-pair="${bank}" data-monitor-edit aria-label="Speakers ${'ABC'[bank]} output pair"><option value="none"${selection === 'none' ? ' selected' : ''}>Unassigned</option>${selection === 'custom' ? '<option value="custom" selected>Custom connections (see Patchbay)</option>' : ''}${choices.map(p => `<option value="${esc(p.id)}"${selection === p.id ? ' selected' : ''}>${esc(p.name)}</option>`).join('')}</select></label></article>`;
    }).join('');
    const sources = model.sources.filter(s => !s.id.startsWith('13b9'));
    const inputSelect = channel => {
      const d = model.destinations.find(d => d.group === 'monitor' && d.index === channel);
      const value = effective(model, drafts, d);
      return `<label>${channel ? 'Right' : 'Left'} input <select data-monitor-input="${channel}" data-monitor-edit aria-label="ABC ${channel ? 'right' : 'left'} input">${!model.sourceMap.has(value) ? `<option value="${esc(value)}" selected>${esc(model.sourceName(value))}</option>` : ''}${[...new Set(sources.map(s => s.group))].map(group => `<optgroup label="${esc(group)}">${sources.filter(s => s.group === group).map(s => `<option value="${s.id}"${s.id === value ? ' selected' : ''}>${esc(s.name)}</option>`).join('')}</optgroup>`).join('')}</select></label>`;
    };
    const disconnected = [0, 1].some(i => model.get(0x93b9, i) === '00000000');
    const controls = `<div class="monitor-top"><div class="monitor-volume console-strip"><label for="monitorLevel">Monitor level</label><input id="monitorLevelValue" class="db-input" type="text" inputmode="decimal" data-db-for="monitorLevel" data-db-infinity="-100" value="${validLevel ? volumeText(level) : 'Unavailable'}" aria-label="Monitor level in decibels" title="Type a level or -inf. Enter to apply; Escape to cancel." ${attrs('monitor-level', !validLevel)}><input id="monitorLevel" type="range" min="-100" max="0" step="1" value="${level}" aria-label="Monitor level" aria-valuetext="${volumeText(level)}" ${attrs('monitor-level', !validLevel)}><small>Same level as the front-panel monitor knob.</small></div><div class="monitor-switch"><strong>Speaker selection · ${selectedNames(mask)}</strong><div class="monitor-buttons" role="group" aria-label="ABC speaker selection">${speakerButtons}</div><div class="monitor-shortcuts">${quickSelections}${combinedSelections}</div><p>${mask ? 'ABC speakers share the monitor level.' : 'ABC off · Monitor Group active.'}</p><strong class="monitor-muted"${model.get(0x139b, 0) === '01' ? '' : ' hidden'}>Muted on the device</strong></div></div>
      <div class="monitor-actions" role="group" aria-label="Front-panel controls">${frontButtons}</div>
      ${disconnected ? '<p class="monitor-notice">ABC has an unassigned input. Open Monitor setup to choose its source.</p>' : ''}`;
    const setup = `
      <details id="monitorSetup"${setupOpen ? ' open' : ''}><summary>ABC source &amp; speaker connections</summary><p>Choose the shared stereo signal, then the outputs for each speaker pair. Review and apply the staged connections below.</p><div class="monitor-inputs">${inputSelect(0)}${inputSelect(1)}</div><div class="monitor-speakers">${cards}</div><p class="muted">For individual channels or additional destinations, use <a href="#patchbay">Patchbay</a>. Line-output trims below adjust the relative speaker levels.</p></details>
      <div class="monitor-members"><h3>Monitor Group <span class="muted">· when ABC is off</span></h3><p>Choose the line outputs controlled together by the monitor knob. Each selection saves immediately.</p><div role="group" aria-label="Monitor Group line outputs">${lines.map(d => `<label><input type="checkbox" data-monitor-member="${d.index}" data-monitor-edit${members & (1 << d.index) ? ' checked' : ''}${!validMembers ? ' data-unavailable="true" disabled' : ''}>${esc(d.name)}</label>`).join('')}</div>${!validMembers ? '<p class="muted">Group membership is unavailable or contains unmapped outputs.</p>' : ''}</div>`;
    return section === 'controls' ? controls : section === 'setup' ? setup : controls + setup;
  }
  function pairChanges(model, drafts, bank, pairId) {
    if (!Number.isInteger(bank) || bank < 0 || bank > 2) throw Error('Invalid speaker pair');
    if (pairId === 'custom') return [];
    const pair = pairs(model).find(p => p.id === pairId);
    if (!pair && pairId !== 'none') throw Error('Choose an advertised output pair');
    const edits = new Map();
    for (const d of physical(model)) if ([0, 1].some(i => effective(model, drafts, d) === M.path(0x13b9, bank, i))) {
      edits.set(d.id, { destination: d, source: '00000000' });
    }
    if (pair) [pair.left, pair.right].forEach((d, i) => edits.set(d.id, { destination: d, source: M.path(0x13b9, bank, i) }));
    return [...edits.values()];
  }
  // A single in-flight request with one latest desired value per control.
  // Incoming device state never erases a pending drag or rebases its conflict
  // check. Only a verified response to our own write advances that expectation.
  function createSync({send, onState, onStatus, schedule = setTimeout, cancel = clearTimeout}) {
    let confirmed = new Map(), pending = new Map(), inFlight = null, timer = null, generation = 0, revision = -1;
    const snapshot = () => ({records:[...confirmed].map(([p,v])=>[p,0,v]), revision});
    const display = () => {
      const state = new Map(confirmed);
      for (const c of inFlight || []) state.set(Number(c.id.split(':')[0]),c.encoded);
      for (const c of pending.values()) state.set(Number(c.id.split(':')[0]),c.encoded);
      return {records:[...state].map(([p,v])=>[p,0,v]),revision};
    };
    function receive(state) {
      if (!state || !Array.isArray(state.records) || (state.revision !== undefined && state.revision < revision)) return;
      const valid = state.records.filter(record => {
        if (!Array.isArray(record) || record.length !== 3) return false;
        const [p,i,v]=record;
        return i===0 && typeof v==='string' && ((p===0x1393 && /^[0-9a-f]{2}$/i.test(v) && parseInt(v,16)<=100) || (p===0x1394 && /^[0-9a-f]{4}$/i.test(v)) || (p===0x13b6 && /^0[0-7]$/i.test(v)) || ([0x139a,0x139b,0x13a3].includes(p) && /^0[01]$/i.test(v)));
      });
      for (const [p,,v] of valid) confirmed.set(p,v);
      revision = state.revision ?? revision;
      onState(snapshot(),display());
    }
    function queue(operation, value) {
      const id = M.address(operation,'monitor',0), property = Number(id.split(':')[0]);
      const active = inFlight?.find(c=>c.id===id);
      const expected = pending.get(id)?.expected ?? active?.encoded ?? confirmed.get(property);
      if (expected === undefined) throw Error('Wait for monitor state before editing.');
      pending.set(id,{id,operation,target:'monitor',index:0,expected,value:String(value),encoded:M.encoded(operation,value)});
      onState(snapshot(),display());
      onStatus('Saving monitor changes…');
      if (!timer && !inFlight) timer=schedule(flush,60);
    }
    async function flush() {
      timer=null;
      if (inFlight || !pending.size) return;
      const changes=[...pending.values()].filter(c=>c.encoded!==confirmed.get(Number(c.id.split(':')[0])));
      pending.clear();
      if (!changes.length) {onState(snapshot(),display());onStatus('Monitor is up to date.');return;}
      inFlight=changes;
      const epoch=generation;
      try {
        const result=await send(changes);
        if (epoch!==generation) return;
        receive(result.monitor);
        if (!result.monitor || changes.some(c=>confirmed.get(Number(c.id.split(':')[0]))!==c.encoded)) throw Error('Monitor readback differs; showing device state.');
        onStatus('Monitor updated.');
      } catch(error) {
        if(epoch!==generation)return;
        pending.clear();onStatus(error.message,'error');
      }
      if(epoch!==generation)return;
      inFlight=null;onState(snapshot(),display());
      if(pending.size) timer=schedule(flush,0);
    }
    return {receive,queue,snapshot,display,seed:state=>{if(revision<0)receive(state);},busy:()=>!!inFlight||pending.size>0,
      reset() {generation++;if(timer)cancel(timer);timer=null;pending.clear();inFlight=null;confirmed.clear();revision=-1;},
    };
  }
  function syncDom(state) {
    const get=p=>state.records.find(r=>r[0]===p)?.[2];
    const level=document.getElementById('monitorLevel');
    if(!level)return;
    if(get(0x1393)!==undefined) {
      const value=String(-parseInt(get(0x1393),16));
      if(level.value!==value)level.value=value;
      const text=volumeText(Number(level.value));
      CueMixDb.sync(document.getElementById('monitorLevelValue'), text);level.setAttribute('aria-valuetext',text);
    }
    const mask=parseInt(get(0x13b6),16);
    if(Number.isInteger(mask)) {
      document.querySelector('.monitor-switch strong').textContent=`Speaker selection · ${selectedNames(mask)}`;
      document.querySelectorAll('button[data-control="monitor-select"]').forEach(button=>{
        const bit = Number(button.dataset.monitorToggle);
        button.setAttribute('aria-pressed', String(bit ? !!(mask & bit) : Number(button.dataset.value) === mask));
        if (bit) button.dataset.value = String(mask ^ bit);
      });
      const combination = document.getElementById('monitorCombination');
      if (combination) combination.value = String(mask);
      document.querySelector('.monitor-switch p').textContent=mask ? 'ABC speakers share the monitor level.' : 'ABC off · Monitor Group active.';
      document.querySelectorAll('.monitor-speaker').forEach((card,bank)=>{
        const active=!!(mask&(1<<bank));
        card.classList.toggle('is-selected',active);
        card.querySelector('h3').textContent=`Speakers ${'ABC'[bank]}${active ? ' · Active' : ''}`;
      });
    }
    const members=parseInt(get(0x1394),16);
    if(Number.isInteger(members)) document.querySelectorAll('[data-monitor-member]').forEach(node=>{node.checked=!!(members&(1<<Number(node.dataset.monitorMember)));});
    for(const [operation,property] of [['monitor-mute',0x139b],['monitor-mono',0x139a],['monitor-talk',0x13a3]]) {
      const button=document.querySelector(`[data-control="${operation}"]`), value=get(property);
      if(button && /^0[01]$/.test(value)) {
        const active=value==='01';
        button.setAttribute('aria-pressed',String(active));
        button.dataset.value=active?'0':'1';
        // Meter events arrive while a pointer may be held over the label.
        // Replacing the children here can remove its pending click target.
        const stateNode=button.querySelector('.monitor-action-state'), text=active?'On':'Off';
        if(stateNode && stateNode.textContent!==text)stateNode.textContent=text;
      }
    }
    const muted=document.querySelector('.monitor-muted');
    if(muted)muted.hidden=get(0x139b)!=='01';
  }
  return { render, pairChanges, volumeText, createSync, syncDom };
})();
