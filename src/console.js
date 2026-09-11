(() => {
  'use strict';
  const M = globalThis.CueMixModel;
  const el = id => document.getElementById(id);
  const escape = escapeHtml;
  const views = ['patchbay', 'routing', 'mixer', 'aux'];
  let model = null, loading = false, writing = false, online = false, epoch = 0;
  let selectedView = '', loadedHost = '', lastRead = 0;
  let refreshTask = null, deferredSnapshot = null;
  let patchPage = 0, matrixRowPage = 0, matrixColPage = 0;
  let drafts = new Map();
  const dbText = db => db === null ? 'Unavailable' : db === -Infinity ? '−∞ dB' : `${db > 0 ? '+' : ''}${db.toFixed(1)} dB`;
  const selectedBus = () => model?.buses.find(bus => bus.id === el('mixBus').value);
  const selectedChannel = () => model?.channels.find(channel => channel.index === Number(el('auxInput').value));
  const stateKey = 'cuemix-console-views';
  let preferences = {};
  try { preferences = JSON.parse(localStorage.getItem(stateKey) || '{}'); } catch { /* Optional view preferences. */ }
  function status(message, state = '') {
    document.querySelectorAll('.console-status').forEach(node => { node.textContent = message; node.dataset.state = state; });
  }
  function availability() {
    document.querySelectorAll('.console-panel').forEach(panel => {
      panel.dataset.offline = String(!online);
      panel.querySelectorAll('[data-control], [data-route], #stagePatch').forEach(control => { control.disabled = writing || !online || control.dataset.unavailable === 'true'; });
    });
    el('applyRoutes').disabled = writing || !online || !drafts.size || hasConflict();
    el('discardRoutes').disabled = writing;
  }
  const hasConflict = () => model && [...drafts.values()].some(d => model.records.get(d.id) !== d.expected);
  function option(value, text) { return `<option value="${escape(value)}">${escape(text)}</option>`; }
  function options(id, entries, preferred) {
    const node = el(id), value = preferred ?? node.value;
    node.innerHTML = entries.map(([v, text]) => option(v, text)).join('');
    if (entries.some(([v]) => String(v) === value)) node.value = value;
  }
  function refreshChoices() {
    const groups = model.groups.map(g => [g.id, g.name]);
    options('patchGroup', groups, el('patchGroup').value || preferences.patchGroup);
    options('matrixDestGroup', groups, el('matrixDestGroup').value || preferences.matrixDestGroup);
    options('matrixSourceGroup', [...new Set(model.sources.filter(s => s.id !== '00000000').map(s => s.group))].map(g => [g, g]), el('matrixSourceGroup').value || preferences.matrixSourceGroup);
    options('mixBus', model.buses.map(bus => [bus.id, bus.name]), el('mixBus').value || preferences.mixBus);
    options('auxInput', model.channels.map(channel => [String(channel.index), channel.name]), el('auxInput').value || preferences.auxInput);
    refreshBuilder();
  }
  function refreshBuilder() {
    if (!model) return;
    options('patchDestination', model.destinations.filter(d => d.group === el('patchGroup').value).map(d => [d.id, d.name]));
    const search = el('sourceSearch').value.toLowerCase().trim();
    const sources = model.sources.filter(source => `${source.name} ${source.group}`.toLowerCase().includes(search));
    const select = el('patchSource'), previous = select.value;
    select.innerHTML = [...new Set(sources.map(s => s.group))].map(group => `<optgroup label="${escape(group)}">${sources.filter(s => s.group === group).map(s => option(s.id, s.name)).join('')}</optgroup>`).join('');
    if (sources.some(s => s.id === previous)) select.value = previous;
  }
  function acceptSnapshot(snapshot) {
    model = M.create(snapshot);
    refreshChoices(); render();
  }
  function editingStrip() { return document.activeElement?.closest('.console-strip'); }
  function refresh({ quiet = false, force = false, afterWrite = false } = {}) {
    if (loading) return refreshTask;
    if ((writing && !afterWrite) || (!views.includes(selectedView) && !force) || (document.hidden && !force)) return Promise.resolve(false);
    if (!force && model && Date.now() - lastRead < 4000) { render(); return; }
    const host = el('host').value, generation = epoch;
    loading = true;
    availability();
    if (!quiet) status('Reading connections and mixes…');
    refreshTask = (async () => { try {
      const snapshot = await fetchJson('/api/console?' + qs({ host }));
      if (generation !== epoch || host !== el('host').value || (writing && !afterWrite)) return false;
      // A user may start editing AFTER a background request begins. Preserve
      // both the DOM and its expected values until that edit is committed.
      if (editingStrip() && model && !afterWrite) deferredSnapshot = snapshot;
      else { deferredSnapshot = null; acceptSnapshot(snapshot); }
      loadedHost = host; online = true; lastRead = Date.now();
      status(`${model.destinations.length} destinations · ${model.channels.length} input strips · ${model.buses.length} mixes · Updated ${new Date().toLocaleTimeString()}`);
      return true;
    } catch (error) {
      if (generation !== epoch || (writing && !afterWrite)) return false;
      online = !afterWrite && !!model && loadedHost === host && Date.now() - lastRead < 15000;
      status(`Cannot refresh the device: ${error.message}. ${online ? 'Showing the last confirmed values; retrying automatically.' : model ? 'Showing the last snapshot; editing is paused.' : 'Use Refresh to try again.'}`, 'error');
      return false;
    } finally {
      if (generation === epoch) { loading = false; refreshTask = null; availability(); loadMeters(); }
    } })();
    return refreshTask;
  }
  function render() {
    if (!model) return;
    if (selectedView === 'patchbay') renderPatchbay();
    if (selectedView === 'routing') renderMatrix();
    if (selectedView === 'mixer') renderMixer();
    if (selectedView === 'aux') renderAux();
    renderDrafts(); availability();
    if (typeof lastMeterRecords !== 'undefined') meters(lastMeterRecords);
  }
  function pageControls(kind, page, count, size) {
    const max = Math.max(1, Math.ceil(count / size));
    return `<div><button class="secondary" data-page="${kind}" data-delta="-1" ${page <= 0 ? 'disabled' : ''} aria-label="Previous ${kind}">←</button><span>${count ? page * size + 1 : 0}–${Math.min(count, (page + 1) * size)} of ${count}</span><button class="secondary" data-page="${kind}" data-delta="1" ${page >= max - 1 ? 'disabled' : ''} aria-label="Next ${kind}">→</button></div>`;
  }
  function renderPatchbay() {
    const search = el('patchSearch').value.toLowerCase().trim();
    const rows = model.destinations.filter(d => d.group === el('patchGroup').value && `${d.name} ${model.sourceName(d.source)}`.toLowerCase().includes(search));
    patchPage = Math.min(patchPage, Math.max(0, Math.ceil(rows.length / 24) - 1));
    el('patchList').innerHTML = rows.slice(patchPage * 24, patchPage * 24 + 24).map(d => {
      const draft = drafts.get(M.key(d.property, d.index));
      const source = draft?.value ?? d.source;
      return `<div class="patch-row${draft ? ' is-planned' : ''}"><strong>${escape(d.name)}</strong><span class="route-arrow" aria-hidden="true">←</span><div class="route-source">${escape(model.sourceName(source))}${draft ? `<small>Was ${escape(model.sourceName(d.source))}</small>` : ''}</div><div class="route-row-actions"><button class="secondary" data-edit-route="${d.id}" aria-label="Change source for ${escape(d.name)}">Change</button><button class="secondary" data-route="${d.id}" data-source="00000000" aria-label="Disconnect ${escape(d.name)}">Clear</button></div></div>`;
    }).join('') || '<p class="empty">No connections match your search.</p>';
    el('patchPages').innerHTML = pageControls('connections', patchPage, rows.length, 24);
  }
  function renderMatrix() {
    const rows = model.destinations.filter(d => d.group === el('matrixDestGroup').value);
    const sources = model.sources.filter(s => s.group === el('matrixSourceGroup').value);
    matrixRowPage = Math.min(matrixRowPage, Math.max(0, Math.ceil(rows.length / 16) - 1));
    matrixColPage = Math.min(matrixColPage, Math.max(0, Math.ceil(sources.length / 16) - 1));
    const visibleSources = sources.slice(matrixColPage * 16, matrixColPage * 16 + 16);
    el('matrixPages').innerHTML = `<div>Destinations ${pageControls('destinations', matrixRowPage, rows.length, 16)}</div><div>Sources ${pageControls('sources', matrixColPage, sources.length, 16)}</div>`;
    el('routingMatrix').innerHTML = `<table aria-label="Audio routing matrix"><thead><tr><th scope="col">Destination ← source</th>${visibleSources.map(s => `<th scope="col">${escape(s.name)}</th>`).join('')}</tr></thead><tbody>${rows.slice(matrixRowPage * 16, matrixRowPage * 16 + 16).map(d => {
      const draft = drafts.get(M.key(d.property, d.index)), effective = draft?.value ?? d.source;
      return `<tr><td><strong>${escape(d.name)}</strong><small>${escape(model.sourceName(effective))}${draft ? ' · Staged' : ''}</small></td>${visibleSources.map(s => `<td><button class="matrix-cell${d.source === s.id ? ' connected' : ''}${draft && effective === s.id ? ' planned' : ''}" data-route="${d.id}" data-source="${effective === s.id ? '00000000' : s.id}" aria-label="${escape(d.name)} receives ${escape(s.name)}${draft && effective === s.id ? ' (staged)' : ''}" aria-pressed="${effective === s.id}">${effective === s.id ? (draft ? '◉' : '●') : d.source === s.id ? '○' : '·'}</button></td>`).join('')}</tr>`;
    }).join('')}</tbody></table>`;
  }
  function stage(destination, source) {
    if (!online || writing || !model.sourceMap.has(source)) return;
    const id = M.key(destination.property, destination.index);
    const prior = drafts.get(id);
    if (source === destination.source) drafts.delete(id);
    else {
      if (!prior && drafts.size >= 32) throw Error('Apply or discard the current 32 connections before adding more.');
      const [edit] = M.edits(model, 'route', destination.group, [destination.index], source);
      drafts.set(id, { ...edit, expected: prior?.expected ?? edit.expected, label: destination.name });
    }
  }
  function stageBuilder() {
    const destination = model?.destinations.find(d => d.id === el('patchDestination').value);
    const source = model?.sourceMap.get(el('patchSource').value);
    const count = Number(el('patchCount').value);
    if (!destination || !source || !Number.isInteger(count) || count < 1 || count > 32) throw Error('Choose a destination, a source, and 1–32 channels.');
    const dests = model.destinations.filter(d => d.group === destination.group);
    const destStart = dests.indexOf(destination);
    const sources = model.sources.filter(s => s.group === source.group);
    const sourceStart = sources.indexOf(source);
    const repeated = el('patchMode').value === 'duplicate' || source.id === '00000000';
    if (destStart + count > dests.length || (!repeated && sourceStart + count > sources.length)) throw Error('The channel range extends past the selected group. Reduce the channel count.');
    const previous = new Map(drafts);
    try { for (let i = 0; i < count; i++) stage(dests[destStart + i], repeated ? source.id : sources[sourceStart + i].id); }
    catch (error) { drafts = previous; throw error; }
    render(); status(`${count} connection${count === 1 ? '' : 's'} staged. Review and apply below.`);
  }
  function renderDrafts() {
    const tray = el('consoleDraftTray');
    tray.hidden = !drafts.size || !['patchbay', 'routing'].includes(selectedView);
    el('draftCount').textContent = `${drafts.size} staged connection${drafts.size === 1 ? '' : 's'}`;
    el('draftHint').textContent = hasConflict() ? 'Some device routes changed. Remove those edits and stage them again.' : 'Your hardware changes only when you apply.';
    el('draftList').innerHTML = [...drafts.values()].map(d => `<div class="draft-row${model.records.get(d.id) !== d.expected ? ' draft-conflict' : ''}"><span><strong>${escape(d.label)}</strong>: ${escape(model.sourceName(d.expected))} → ${escape(model.sourceName(d.value))}${model.records.get(d.id) !== d.expected ? ' · Device changed' : ''}</span><button class="secondary" data-remove-draft="${d.id}" aria-label="Remove staged connection for ${escape(d.label)}">Remove</button></div>`).join('');
  }
  function controlAttrs(operation, bus, channel, unavailable = false) {
    return `data-control="${operation}" data-target="${escape(bus)}" data-index="${channel}"${unavailable ? ' data-unavailable="true" disabled' : ''}`;
  }
  function levelMarkup(operation, target, index, value, name, meterBank, members) {
    const db = M.level(value), available = db !== null;
    const position = db === -Infinity ? -91 : Math.max(-90, Math.min(12, db ?? -91));
    const entry = db === -Infinity ? '-inf' : db === null ? '' : db.toFixed(1);
    return `<output class="console-value">${dbText(db)}</output><div class="strip-fader-body"><input type="range" class="console-fader" min="-91" max="12" step="0.1" value="${position}" aria-label="${escape(name)} level" aria-valuetext="${dbText(db)}" ${controlAttrs(operation, target, index, !available)}>${members.map(member => `<span class="console-meter" aria-label="${escape(name)} input meter"><i class="meter-fill" data-console-meter="${meterBank}:${member}"></i></span>`).join('')}${meterScaleMarkup()}</div><label class="console-level-entry"><input inputmode="decimal" value="${entry}" data-initial="${entry}" aria-label="${escape(name)} level in decibels" ${controlAttrs(operation, target, index, !available)}><span>dB</span></label>`;
  }
  function panMarkup(bus, channel, name) {
    const raw = model.get(bus.pan, (channel.index << 8) | (bus.id.startsWith('aux-') ? bus.index : 0));
    if (raw === undefined) return '';
    const value = M.number(raw) / 16777216 * 2 - 1;
    return `<label class="console-pan"><span><span>L</span><span class="pan-value">${panText(value)}</span><span>R</span></span><input type="range" min="-1" max="1" step="0.01" value="${value}" aria-label="${escape(name)} pan" ${controlAttrs('pan', bus.id, channel.index)}></label>`;
  }
  const panText = value => Math.abs(value) < .01 ? 'Center' : `${Math.round(Math.abs(value) * 100)}${value < 0 ? 'L' : 'R'}`;
  function toggle(operation, target, index, name, property, channelName) {
    const raw = model.get(property, index), pressed = raw === '01';
    return `<button class="secondary" ${controlAttrs(operation, target, index, !['00','01'].includes(raw))} data-value="${pressed ? '0' : '1'}" aria-pressed="${pressed}" aria-label="${escape(channelName + ' ' + name)}">${escape(name)}</button>`;
  }
  function strip(bus, channel, aux = false) {
    const raw = model.get(bus.fader, (channel.index << 8) | (bus.id.startsWith('aux-') ? bus.index : 0));
    const title = aux ? bus.name : channel.name;
    return `<article class="console-strip" data-strip-channel="${channel.index}"><h3>${escape(title)}</h3><div class="channel-subtitle">${aux ? (bus.stereo ? 'Stereo send' : 'Mono send') : `Input ${channel.members.map(i => i + 1).join(' / ')} · ${channel.stereo ? 'Stereo' : 'Mono'}`}</div>${levelMarkup('level', bus.id, channel.index, raw, title, 0, channel.members)}${panMarkup(bus, channel, title)}${aux ? preButton(bus) : `<div class="strip-buttons">${toggle('mute','input',channel.index,'Mute',0x03fb,title)}${toggle('solo','input',channel.index,'Solo',0x03fa,title)}</div>`}</article>`;
  }
  function preButton(bus) {
    if (!bus.pre) return '';
    const raw = model.get(bus.pre, bus.index);
    if (!['00', '01'].includes(raw)) return '';
    return `<button class="secondary console-pre" ${controlAttrs('pre', bus.id, bus.index)} data-value="${raw === '01' ? '0' : '1'}" aria-label="${escape(bus.name)} send position">${raw === '01' ? 'Pre-fader' : 'Post-fader'}</button>`;
  }
  function renderMixer() {
    const bus = selectedBus();
    if (!bus) { el('consoleStrips').innerHTML = '<p class="empty">No mixer buses were advertised by this device.</p>'; el('consoleMaster').innerHTML = ''; return; }
    const search = el('mixSearch').value.toLowerCase();
    const channels = model.channels.filter(channel => channel.name.toLowerCase().includes(search) && (!el('mixActiveOnly').checked || M.number(model.get(bus.fader, (channel.index << 8) | (bus.id.startsWith('aux-') ? bus.index : 0))) > 0));
    el('consoleStrips').innerHTML = channels.map(channel => strip(bus, channel)).join('') || '<p class="empty">No inputs match this filter.</p>';
    el('consoleMaster').innerHTML = `<article class="console-strip"><h3>${escape(bus.name)}</h3><div class="channel-subtitle">Bus master · ${bus.stereo ? 'Stereo' : 'Mono'}</div>${levelMarkup('master', bus.id, bus.index, model.get(bus.gain, bus.index), bus.name + ' master', bus.meterBank, bus.members)}<div class="strip-buttons">${toggle('master-mute',bus.id,bus.index,'Mute bus',bus.mute,bus.name)}</div>${preButton(bus)}</article>`;
  }
  function renderAux() {
    const channel = selectedChannel();
    el('auxSends').innerHTML = channel ? model.buses.filter(bus => bus.id !== 'main').map(bus => strip(bus, channel, true)).join('') : '<p class="empty">No mixer inputs were advertised by this device.</p>';
  }
  async function apply(changes, description, routing = false) {
    if (!changes.length || writing || !online || loadedHost !== el('host').value) return;
    const host = loadedHost, generation = epoch;
    writing = true; deferredSnapshot = null; availability(); status(`Saving ${description}…`);
    let failure = null;
    try {
      // Let an in-flight read finish, without allowing it to replace the
      // user's edit or its original expected values. Never silently drop it.
      await refreshTask;
      if (generation !== epoch) return;
      const response = await fetch('/api/console/changes', { method:'POST', headers:{'Content-Type':'application/x-www-form-urlencoded'}, body:qs({host, token:sessionToken, changes:M.serialize(changes)}) });
      const result = await response.json();
      if (!response.ok || result.error) throw Error(result.error || `HTTP ${response.status}`);
    } catch (error) { failure = error; }
    if (generation !== epoch) return;
    await refresh({ force:true, quiet:true, afterWrite:true });
    if (generation !== epoch) return;
    writing = false;
    const matched = online && changes.every(c => model.records.get(c.id) === c.encoded);
    if (routing && online) {
      for (const c of changes) if (model.records.get(c.id) === c.encoded) drafts.delete(c.id);
    }
    if (failure) status(`${description}: ${failure.message}`, 'error');
    else if (!online) status('The write was acknowledged, but readback is unavailable. Refresh before making another change.', 'error');
    else if (!matched) status('The write was acknowledged, but the device reports a different value. The current device values are shown; review before retrying.', 'error');
    else status(`Saved ${description}. Verified against the device.`, 'saved');
    render();
  }
  function controlChanges(node) {
    const { control: operation, target, index: rawIndex } = node.dataset;
    const index = Number(rawIndex);
    let value = node.dataset.value ?? node.value;
    if (operation === 'level' || operation === 'master') {
      value = String(value).trim().replace('−','-');
      if (value === '−∞' || value === '-∞' || (node.type === 'range' && Number(value) <= -91)) value = '-inf';
      if (value !== '-inf' && (!value || !Number.isFinite(Number(value)) || Number(value) < -90 || Number(value) > 12)) throw Error('Enter -inf or a level from -90 to +12 dB.');
    }
    if (['master','master-mute','pre'].includes(operation)) {
      const bus = model.buses.find(b => b.id === target);
      return bus.members.flatMap(member => M.edits(model, operation, target.startsWith('aux-') ? `aux-${member}` : target, [member], value));
    }
    if (['mute','solo'].includes(operation)) {
      const channel = model.channels.find(c => c.index === index);
      return M.edits(model, operation, target, channel.members, value);
    }
    return M.edits(model, operation, target, [index], value);
  }
  function meters(records) {
    const samples = new Map(records.filter(r => r.property_id === '13ad').map(r => [Number(r.index), r]));
    document.querySelectorAll('[data-console-meter]').forEach(node => {
      const [bank, channel] = node.dataset.consoleMeter.split(':').map(Number);
      const record = samples.get(bank);
      const raw = record?.channels?.[channel] ?? (record?.values?.[Math.floor(channel / 2)] === undefined ? null : meterWordChannels(record.values[Math.floor(channel / 2)])[channel % 2]);
      node.style.clipPath = `inset(${100 - meterPercent(raw)}% 0 0)`;
      node.parentElement.title = meterText(raw);
    });
  }
  function run(action) { try { action(); } catch (error) { status(error.message, 'error'); } }
  document.addEventListener('click', event => {
    const button = event.target.closest('button');
    if (!button) return;
    if (button.classList.contains('console-refresh')) { refresh({force:true}); return; }
    if (button.dataset.page) {
      const delta = Number(button.dataset.delta);
      if (button.dataset.page === 'connections') patchPage += delta;
      if (button.dataset.page === 'destinations') matrixRowPage += delta;
      if (button.dataset.page === 'sources') matrixColPage += delta;
      render(); return;
    }
    if (button.dataset.editRoute) {
      const destination = model.destinations.find(d => d.id === button.dataset.editRoute);
      el('patchDestination').value = destination.id;
      el('sourceSearch').value = ''; refreshBuilder();
      el('patchSource').value = drafts.get(M.key(destination.property,destination.index))?.value ?? destination.source;
      el('patchCount').value = '1'; el('patchSource').focus(); return;
    }
    if (button.dataset.route) run(() => { stage(model.destinations.find(d => d.id === button.dataset.route), button.dataset.source); render(); });
    if (button.dataset.removeDraft && !writing) { drafts.delete(button.dataset.removeDraft); render(); }
    if (button.dataset.control) run(() => { apply(controlChanges(button), button.getAttribute('aria-label') || 'mix setting'); });
  });
  document.addEventListener('input', event => {
    const node = event.target;
    if (!node.dataset.control || node.type !== 'range') return;
    if (['level','master'].includes(node.dataset.control)) {
      const db = Number(node.value) <= -91 ? -Infinity : Number(node.value);
      const strip = node.closest('.console-strip');
      strip.querySelector('.console-value').textContent = dbText(db);
      strip.querySelector('.console-level-entry input').value = db === -Infinity ? '-inf' : db.toFixed(1);
      node.setAttribute('aria-valuetext',dbText(db));
    } else if (node.dataset.control === 'pan') node.closest('.console-pan').querySelector('.pan-value').textContent = panText(Number(node.value));
  });
  document.addEventListener('change', event => {
    const node = event.target;
    if (node.dataset.control && node.type === 'range') run(() => { apply(controlChanges(node), node.getAttribute('aria-label') || 'mix setting'); });
  });
  document.addEventListener('focusout', event => {
    const node = event.target;
    if (node.dataset.initial !== undefined && node.value !== node.dataset.initial) run(() => { apply(controlChanges(node), node.getAttribute('aria-label') || 'mix setting'); });
    setTimeout(() => {
      if (deferredSnapshot && !writing && !editingStrip()) {
        const snapshot = deferredSnapshot; deferredSnapshot = null; acceptSnapshot(snapshot);
      }
    }, 0);
  });
  document.addEventListener('keydown', event => {
    if (event.key === 'Enter' && event.target.dataset.control && event.target.type !== 'range' && event.target.tagName === 'INPUT') event.target.blur();
  });
  el('stagePatch').addEventListener('click', () => run(stageBuilder));
  el('discardRoutes').addEventListener('click', () => { if (!writing) { drafts.clear(); render(); } });
  el('applyRoutes').addEventListener('click', () => { if (!hasConflict()) apply([...drafts.values()], 'routing connections', true); });
  for (const id of ['patchGroup','matrixDestGroup','matrixSourceGroup','mixBus','auxInput']) el(id).addEventListener('change', () => {
    preferences[id] = el(id).value;
    try { localStorage.setItem(stateKey, JSON.stringify(preferences)); } catch { /* View still works without storage. */ }
    patchPage = matrixRowPage = matrixColPage = 0; refreshBuilder(); render();
  });
  for (const id of ['patchSearch','mixSearch']) el(id).addEventListener('input', () => { patchPage = 0; render(); });
  el('mixActiveOnly').addEventListener('change', render);
  el('sourceSearch').addEventListener('input', refreshBuilder);
  el('host').addEventListener('change', () => { epoch++; loading = writing = online = false; refreshTask = deferredSnapshot = null; model = null; drafts.clear(); loadedHost = ''; lastRead = 0; refresh({force:true}); });
  // Poll even during edits; defer adopting results until focus leaves the strip.
  setInterval(() => { if (!document.hidden && views.includes(selectedView)) refresh({quiet:true}); }, 5000);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) refresh({force:true,quiet:true}); });
  globalThis.consoleUi = {
    activate(view) { selectedView = view; el('consoleDraftTray').hidden = !drafts.size || !['patchbay','routing'].includes(view); if (views.includes(view)) refresh(); },
    meters,
  };
  consoleUi.activate(document.querySelector('[role="tab"][aria-selected="true"]')?.dataset.tab || 'inputs');
})();
