/* Shared, DOM-free console model. The browser and offline tests use this file. */
globalThis.CueMixModel = (() => {
  const Q24 = 16777216;
  const hex = (number, width = 8) => number.toString(16).padStart(width, '0');
  const number = value => value === undefined ? null : parseInt(value, 16);
  const level = value => number(value) === null ? null : number(value) === 0 ? -Infinity : 20 * Math.log10(number(value) / Q24);
  const encodeLevel = db => db === '-inf' ? '00000000' : hex(Math.trunc(10 ** (Number(db) / 20) * Q24));
  const path = (property, bank, channel) => hex(property, 4) + hex(bank, 2) + hex(channel, 2);
  const key = (property, index) => `${property}:${index}`;
  const label = (value, fallback) => {
    if (!value) return fallback;
    const bytes = value.match(/../g).map(byte => parseInt(byte, 16));
    const end = bytes.indexOf(0);
    try {
      const name = new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array(end < 0 ? bytes : bytes.slice(0, end))).trim();
      return name && !/[\x00-\x1f\x7f]/.test(name) ? name : fallback;
    } catch { return fallback; }
  };
  function create(snapshot) {
    if (!snapshot || !Array.isArray(snapshot.records) || snapshot.records.length > 10000) throw Error('Invalid console snapshot');
    const records = new Map();
    for (const record of snapshot.records) {
      if (!Array.isArray(record) || record.length !== 3 || ![record[0], record[1]].every(n => Number.isInteger(n) && n >= 0 && n <= 65535) || typeof record[2] !== 'string' || !/^(?:[0-9a-f]{2}){0,255}$/i.test(record[2])) throw Error('Invalid console record');
      const id = key(record[0], record[1]);
      if (records.has(id)) throw Error('Duplicate console record');
      records.set(id, record[2].toLowerCase());
    }
    const get = (p, i) => records.get(key(p, i));
    const indices = p => snapshot.records.filter(r => r[0] === p).map(r => r[1]).sort((a, b) => a - b);
    const sources = [{ id: '00000000', name: 'Disconnected', group: 'Disconnected' }];
    function addSources(names, property, group, fallback, bank = 0, offset = 0, selectedIndices = indices(names)) {
      for (const i of selectedIndices.filter(i => i < 256)) sources.push({ id: path(property, bank, i - offset), name: label(get(names, i), fallback(i)), group });
    }
    addSources(0x8025, 0x138c, 'Mic / instrument', i => `Mic / Inst ${i + 1}`);
    addSources(0x8020, 0x13ac, 'Line inputs', i => `Line In ${i + 5}`);
    for (const i of indices(0x8023).filter(i => (i >> 8) < 16 && (i & 255) < 8)) {
      const channel = (i >> 8) * 8 + (i & 255);
      sources.push({ id: path(0x13b0, 0, channel), name: label(get(0x8023, i), `Host Out ${channel + 1}`), group: 'Computer playback' });
    }
    for (const i of indices(0x8022).filter(i => (i >> 8) < 2 && (i & 255) < 8)) {
      const bank = i >> 8, channel = i & 255, group = `Optical ${bank ? 'B' : 'A'}`;
      sources.push({ id: path(0x13ae, bank, channel), name: label(get(0x8022, i), `${group} ${channel + 1}`), group });
    }
    for (let b = 0; b < 16; b++) {
      const format = get(0x1b5b, b);
      const count = format?.length === 12 ? parseInt(format.slice(8, 10), 16) : 0;
      addSources(0x8024, 0x13af, `Network ${b + 1}`, i => `Network ${b + 1} · ${i % 8 + 1}`, b, b * 8, indices(0x8024).filter(i => Math.floor(i / 8) === b && i % 8 < count));
    }
    for (const [property, bank, group, prefix] of [[0x03e8, 0, 'Mixer direct outs', 'Post FX'], [0x0420, 1, 'Main mix', 'Main'], [0x0448, 2, 'Monitor mix', 'Mix Monitor'], [0x0403, 3, 'Aux mixes', 'Aux'], [0x0434, 4, 'Reverb', 'Reverb']]) {
      for (const i of indices(property).filter(i => i < 256)) sources.push({ id: path(0x13ad, bank, i), name: `${prefix} ${i + 1}`, group });
    }
    const sourceMap = new Map(sources.map(source => [source.id, source]));
    const groups = [
      { id: 'line', property: 0x93ac, names: 0x8028, name: 'Line outputs', fallback: i => `Line Out ${i + 1}` },
      { id: 'phones', property: 0x93b1, names: 0x802d, name: 'Headphones', fallback: i => `Phones ${Math.floor(i / 256) + 1} ${i % 256 ? 'R' : 'L'}` },
      { id: 'optical', property: 0x93ae, names: 0x802a, name: 'Optical outputs', fallback: i => `Optical Out ${i >> 8 ? 'B' : 'A'} ${(i & 255) + 1}` },
      { id: 'host', property: 0x93b0, names: 0x802b, nameIndex: i => (Math.floor(i / 8) << 8) | (i % 8), name: 'Computer recording', fallback: i => `Host In ${i + 1}` },
      { id: 'mixer', property: 0x93ad, names: 0x8029, name: 'Mixer inputs', fallback: i => `Mixer In ${i + 1}` },
      { id: 'network', property: 0x93af, names: 0x802c, nameIndex: i => (i >> 8) * 8 + (i & 255), name: 'Network outputs', fallback: i => `Network Out ${(i >> 8) + 1} · ${(i & 255) + 1}` },
    ];
    const destinations = groups.flatMap(group => indices(group.property).filter(i => get(group.property, i)?.length === 8).map(i => ({ id: `${group.id}:${i}`, group: group.id, groupName: group.name, property: group.property, index: i, name: label(get(group.names, group.nameIndex ? group.nameIndex(i) : i), group.fallback(i)), source: get(group.property, i) })));
    // Unknown current routes stay visible, but are never offered as selectable sources.
    const sourceName = id => sourceMap.get(id)?.name || `Unavailable source (${id})`;
    const channels = [];
    const inputIndices = indices(0x03e8).filter(i => i < 256);
    for (let pos = 0; pos < inputIndices.length; pos++) {
      const i = inputIndices[pos];
      const stereo = get(0x03e8, i) === '01' && inputIndices[pos + 1] === i + 1;
      const route = get(0x93ad, i) || '00000000';
      const rightRoute = stereo ? get(0x93ad, i + 1) : undefined;
      const leftName = sourceName(route);
      const name = route === '00000000' ? `Mixer ${i + 1}${stereo ? `–${i + 2}` : ''}` : leftName + (stereo && rightRoute !== route ? ` / ${sourceName(rightRoute)}` : '');
      channels.push({ index: i, members: stereo ? [i, i + 1] : [i], stereo, name, source: route, group: sourceMap.get(route)?.group || 'Unassigned', route, rightRoute });
      if (stereo) pos++;
    }
    const buses = [];
    function addBus(id, name, members, gain, mute, pre, fader, pan, meterBank) {
      if (!members.length || !members.every(i => get(gain, i)?.length === 8)) return;
      buses.push({ id, name, members, gain, mute, pre, fader, pan, meterBank, index: members[0], stereo: members.length === 2 });
    }
    addBus('main', 'Main 1–2', indices(0x0420).filter(i => i < 2), 0x0420, 0x0421, null, 0x841a, 0x842b, 1);
    const aux = indices(0x0403).filter(i => i < 256);
    for (let pos = 0; pos < aux.length; pos++) {
      const i = aux[pos];
      const stereo = get(0x03e9, i) === '01' && aux[pos + 1] === i + 1;
      addBus(`aux-${i}`, `Aux ${i + 1}${stereo ? `–${i + 2}` : ''}`, stereo ? [i, i + 1] : [i], 0x0403, 0x0404, 0x0411, 0x83f8, 0x83f9, 3);
      if (stereo) pos++;
    }
    addBus('reverb', 'Reverb 1–2', indices(0x0434).filter(i => i < 2), 0x0434, 0x0435, 0x043c, 0x842e, 0x843f, 4);
    return { records, get, indices, sources, sourceMap, sourceName, destinations, groups: groups.filter(g => destinations.some(d => d.group === g.id)), channels, buses };
  }
  function address(operation, target, index) {
    if (operation === 'route') return key({line:0x93ac,mixer:0x93ad,optical:0x93ae,network:0x93af,host:0x93b0,phones:0x93b1}[target], index);
    if (operation === 'mute' || operation === 'solo') return key(operation === 'mute' ? 0x03fb : 0x03fa, index);
    const aux = target.startsWith('aux-');
    const bus = aux ? Number(target.slice(4)) : 0;
    if (operation === 'level' || operation === 'pan') return key(operation === 'level' ? (aux ? 0x83f8 : target === 'main' ? 0x841a : 0x842e) : (aux ? 0x83f9 : target === 'main' ? 0x842b : 0x843f), (index << 8) | bus);
    return key(operation === 'master' ? (aux ? 0x0403 : target === 'main' ? 0x0420 : 0x0434) : operation === 'master-mute' ? (aux ? 0x0404 : target === 'main' ? 0x0421 : 0x0435) : (aux ? 0x0411 : 0x043c), index);
  }
  function encoded(operation, value) {
    if (['level', 'master'].includes(operation)) return encodeLevel(value);
    if (operation === 'pan') return hex(Math.trunc((Number(value) + 1) * 0.5 * Q24));
    if (operation === 'route') return value;
    return hex(Number(value), 2);
  }
  function edits(model, operation, target, indices, value) {
    return indices.map(index => {
      const id = address(operation, target, index);
      const expected = model.records.get(id);
      if (expected === undefined) throw Error('This control is unavailable on the device');
      return { id, operation, target, index, expected, value: String(value), encoded: encoded(operation, value) };
    }).filter(edit => edit.expected !== edit.encoded);
  }
  const serialize = changes => changes.map(c => [c.operation, c.target, c.index, c.expected, c.value].join(':')).join(';');
  return { create, level, number, path, key, address, encoded, edits, serialize };
})();
