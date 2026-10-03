const DeviceSettings = (() => {
  let snapshot = null, loadedHost = '', busy = false;
  const rates = [44100, 48000, 88200, 96000, 176400, 192000];
  const controls = () => ['deviceName', 'saveDeviceName', 'deviceRate', 'deviceClock', 'deviceWordClock', 'loadDevice'].map($);
  const status = (message, state = '') => setStripStatus($('deviceStatus'), message, state);
  function keys(data) {
    if (!/^[a-fA-F0-9]{16}$/.test(data.uid || '')) throw new Error('Device identity unavailable.');
    const entity = `avb/${data.uid}`;
    const configuration = data[`${entity}/current_configuration`];
    if (!Number.isInteger(configuration) || configuration < 0) throw new Error('Device configuration unavailable.');
    return { name: `${entity}/entity_name`, rate: `${entity}/cfg/${configuration}/current_sampling_rate`,
      rates: `${entity}/cfg/${configuration}/sample_rates`, entity };
  }
  async function read(host) {
    const [response, vendor] = await Promise.all([
      fetchJson('/api/get?' + qs({ host, path: '/datastore' })),
      fetchJson('/api/console?' + qs({ host })).catch(error => ({ error: error.message }))
    ]);
    if (response.status !== 200) throw new Error(`Device returned HTTP ${response.status}`);
    const data = JSON.parse(response.body);
    keys(data);
    data.deviceRecords = new Map((vendor.records || []).map(([property, index, value]) => [`${property}:${index}`, value]));
    data.deviceError = vendor.error;
    return data;
  }
  const record = (data, property, index = 0) => data.deviceRecords?.get(`${property}:${index}`);
  const byte = (data, property) => /^[0-9a-f]{2}$/.test(record(data, property) || '') ? parseInt(record(data, property), 16) : null;
  function streamAvailable(data, index) {
    const count = record(data, 0x1b5d);
    return /^[0-9a-f]{4}$/.test(count || '') && parseInt(count, 16) <= 17 && index < parseInt(count, 16)
      && /^[0-9a-f]{12}$/.test(record(data, 0x1b5b, index) || '');
  }
  function changesFor(field, data) {
    const edit = (operation, property, value) => `${operation}:device:0:${record(data, property)}:${value}`;
    if (field === 'rate') return [edit('device-rate', 0x000a, Number($('deviceRate').value))];
    if (field === 'word') return [edit('device-word-clock', 0x000c, $('deviceWordClock').value)];
    const source = $('deviceClock').value;
    if (source.startsWith('stream-')) {
      const stream = Number(source.slice(7));
      if (!streamAvailable(data, stream)) throw new Error('Clock stream unavailable.');
      return [edit('device-clock-stream', 0x1b5f, stream), edit('device-clock', 0x000b, 4)];
    }
    if (!['0', '5', '12', '13'].includes(source)) throw new Error('Clock source unavailable.');
    return [edit('device-clock', 0x000b, source)];
  }
  function render(data, host, preserveName = false) {
    const k = keys(data);
    if (!preserveName && document.activeElement !== $('deviceName')) $('deviceName').value = data[k.name] ?? '';
    $('deviceSerial').value = data[`${k.entity}/serial_number`] ?? '';
    $('deviceFirmware').value = data[`${k.entity}/firmware_version`] ?? '';
    const supported = String(data[k.rates] ?? '').split(':').map(Number);
    Array.from($('deviceRate').options).forEach(option => {
      if (option.value) option.disabled = !supported.includes(Number(option.value));
    });
    const rawRate = record(data, 0x000a);
    const rate = /^[0-9a-f]{8}$/.test(rawRate || '') ? parseInt(rawRate, 16) : Number(data[k.rate]);
    $('deviceRate').value = rates.includes(rate) ? String(rate) : '';
    $('deviceName').disabled = $('saveDeviceName').disabled = typeof data[k.name] !== 'string';
    $('deviceRate').disabled = !rates.includes(rate) || !/^[0-9a-f]{8}$/.test(rawRate || '');
    const source = byte(data, 0x000b), stream = byte(data, 0x1b5f);
    const validSource = [0, 4, 5, 12, 13].includes(source);
    $('deviceClock').disabled = !validSource;
    $('deviceClock').value = source === 4 && streamAvailable(data, stream) ? `stream-${stream}` : validSource ? String(source) : '';
    Array.from($('deviceClock').options).forEach(option => {
      if (option.value.startsWith('stream-')) option.disabled = stream === null || !streamAvailable(data, Number(option.value.slice(7)));
    });
    const word = byte(data, 0x000c);
    $('deviceWordClock').disabled = ![0, 1].includes(word);
    $('deviceWordClock').value = [0, 1].includes(word) ? String(word) : '';
    const ipv4 = record(data, 0x0005);
    let reportedIpv4 = '';
    if (/^[0-9a-f]{32}$/.test(ipv4 || '')) {
      const bytes = ipv4.match(/../g).map(value => parseInt(value, 16));
      const end = bytes.indexOf(0);
      if (end >= 0) reportedIpv4 = String.fromCharCode(...bytes.slice(0, end));
    }
    // Prefer the vendor-reported IPv4 field. IPv6 currently comes from a
    // literal connected address; never display a hostname as an IP address.
    const address = host.replace(/^\[([^\]]+)\](?::\d+)?$/, '$1');
    $('deviceIpv4').value = /^\d{1,3}(?:\.\d{1,3}){3}$/.test(reportedIpv4) ? reportedIpv4 : /^\d{1,3}(?:\.\d{1,3}){3}(?::\d+)?$/.test(address) ? address.split(':')[0] : '';
    $('deviceIpv6').value = address.split(':').length >= 3 && /^[0-9a-f:.]+(?:%[a-z0-9_.-]+)?$/i.test(address) ? address : '';
  }
  async function load({ quiet = false } = {}) {
    if (busy) return;
    busy = true;
    const host = $('host').value;
    try {
      const data = await read(host);
      if (host !== $('host').value) return;
      snapshot = data; loadedHost = host;
      render(data, host);
      if (!quiet) status(data.deviceError ? `Device information refreshed; clock controls unavailable: ${data.deviceError}` : 'Device information refreshed.');
    } catch (error) {
      snapshot = null;
      controls().slice(0, -1).forEach(node => { node.disabled = true; });
      if (loadedHost !== host) {
        ['deviceName', 'deviceSerial', 'deviceFirmware', 'deviceRate', 'deviceClock', 'deviceWordClock', 'deviceIpv4', 'deviceIpv6'].forEach(id => { $(id).value = ''; });
      }
      status(error.message, 'error');
    } finally { busy = false; }
  }
  async function save(field) {
    if (busy || !snapshot || loadedHost !== $('host').value) return;
    const host = loadedHost, k = keys(snapshot);
    const value = field === 'name' ? $('deviceName').value : Number($('deviceRate').value);
    if (field === 'rate' && (!rates.includes(value) || !String(snapshot[k.rates]).split(':').map(Number).includes(value))) return;
    if (field === 'name' && snapshot[k.name] === value) return;
    let changes;
    try { changes = field === 'name' ? [] : changesFor(field, snapshot); }
    catch (error) { status(error.message, 'error'); return; }
    busy = true;
    controls().forEach(node => { node.disabled = true; });
    status('Saving…', 'pending');
    try {
      if (field === 'name') {
        const current = await read(host);
        if (host !== $('host').value || keys(current).name !== k.name || current[k.name] !== snapshot[k.name]) throw new Error('Device changed. Refresh before saving.');
        const response = await fetchJson('/api/set', {
          method: 'POST', headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
          body: qs({ host, path: `/datastore/${k.name}`, value, method: 'POST', token: sessionToken })
        });
        if (response.status >= 400) throw new Error(`Device returned HTTP ${response.status}`);
      } else {
        // The shared session validates every expected byte before any setter,
        // then checks readback. It never retries an uncertain write outcome.
        await fetchJson('/api/console/changes', {
          method: 'POST', headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
          body: qs({ host, changes: changes.join(';'), token: sessionToken })
        });
      }
      const updated = await read(host);
      if (field === 'name' && updated[k.name] !== value) throw new Error('The device did not apply this setting.');
      if (field !== 'name') {
        const properties = { 'device-rate': 0x000a, 'device-clock': 0x000b, 'device-clock-stream': 0x1b5f, 'device-word-clock': 0x000c };
        for (const change of changes) {
          const [operation, , , , expected] = change.split(':');
          if (parseInt(record(updated, properties[operation]), 16) !== Number(expected)) throw new Error('The device did not apply this setting.');
        }
      }
      if (host !== $('host').value) return;
      snapshot = updated;
      render(updated, host);
      status('Saved and verified.', 'saved');
    } catch (error) { status(error.message, 'error'); }
    finally {
      busy = false;
      $('loadDevice').disabled = false;
      if (snapshot && host === $('host').value) render(snapshot, host, true);
    }
  }
  function init() {
    $('loadDevice').addEventListener('click', () => load());
    $('saveDeviceName').addEventListener('click', () => save('name'));
    $('deviceName').addEventListener('keydown', event => { if (event.key === 'Enter') { event.preventDefault(); save('name'); } });
    $('deviceRate').addEventListener('change', () => save('rate'));
    $('deviceClock').addEventListener('change', () => save('clock'));
    $('deviceWordClock').addEventListener('change', () => save('word'));
    setInterval(() => { if (!document.hidden && activeView === 'device') load({ quiet: true }); }, 5000);
  }
  return { init, load };
})();
