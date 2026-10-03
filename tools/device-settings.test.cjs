const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
function harness() {
  const nodes = new Map(), calls = [], timers = [];
  const records = new Map([[10,'00017700'],[11,'00'],[12,'00'],[0x1b5f,'00'],[0x1b5d,'0011']]);
  const $ = id => {
    if (!nodes.has(id)) nodes.set(id, { value: '', disabled: true, options: ['', '44100', '48000', '96000', '192000'].map(value => ({value})),
      events: {}, addEventListener(type, fn) { this.events[type] = fn; } });
    return nodes.get(id);
  };
  $('host').value = '192.168.4.166';
  const entity = 'avb/0001f2fffefeb9e2';
  const data = {uid: '0001f2fffefeb9e2', [`${entity}/current_configuration`]: 0,
    [`${entity}/entity_name`]: '848', [`${entity}/serial_number`]: 'Serial', [`${entity}/firmware_version`]: '2.3',
    [`${entity}/cfg/0/current_sampling_rate`]: 96000, [`${entity}/cfg/0/sample_rates`]: '48000:96000'};
  let apply = true;
  const context = {$, document: {hidden: false, activeElement: null}, activeView: 'device', sessionToken: 'token',
    qs: value => new URLSearchParams(value), setStripStatus: (node, message) => { node.textContent = message; },
    setInterval: fn => timers.push(fn),
    fetchJson: async (url, options) => {
      calls.push({url, options});
      if (url.startsWith('/api/console?')) return {records: [...records].map(([p, v]) => [p, 0, v]).concat([[0x1b5b, 0, '000177000800'], [0x1b5b, 15, '000177000800'], [0x1b5b, 16, '000177000000']])};
      if (options) {
        const form = new URLSearchParams(options.body);
        if (apply && form.has('path')) data[form.get('path').slice('/datastore/'.length)] = form.get('value');
        if (apply && form.has('changes')) for (const item of form.get('changes').split(';')) {
          const [op,,,expected,value] = item.split(':');
          const property = {'device-rate':10,'device-clock':11,'device-word-clock':12,'device-clock-stream':0x1b5f}[op];
          if (records.get(property) !== expected) throw new Error('conflict');
          records.set(property, Number(value).toString(16).padStart(op === 'device-rate' ? 8 : 2, '0'));
        }
        return {status: 200};
      }
      return {status: 200, body: JSON.stringify(data)};
    }};
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(require.resolve('../src/device_settings.js'), 'utf8') + '\nglobalThis.settings = DeviceSettings;', context);
  context.settings.init();
  return {$, calls, data, records, entity, context, noApply() {apply = false;}};
}
test('opening reads identity and advertised rates without writes; polling preserves focused edits', async () => {
  const h = harness(); await h.context.settings.load();
  assert.equal(h.calls.length, 2); assert.equal(h.calls[0].options, undefined);
  assert.equal(h.$('deviceSerial').value, 'Serial'); assert.equal(h.$('deviceRate').value, '96000');
  assert.equal(h.$('deviceRate').options.find(o => o.value === '192000').disabled, true);
  h.$('deviceName').value = 'Draft'; h.context.document.activeElement = h.$('deviceName');
  await h.context.settings.load({quiet:true}); assert.equal(h.$('deviceName').value, 'Draft');
});
test('name save targets dynamic root key, includes token and verifies readback', async () => {
  const h = harness(); await h.context.settings.load(); h.$('deviceName').value = 'Studio';
  await h.$('saveDeviceName').events.click();
  const writes = h.calls.filter(c => c.options); assert.equal(writes.length, 1);
  const form = new URLSearchParams(writes[0].options.body);
  assert.equal(form.get('path'), `/datastore/${h.entity}/entity_name`); assert.equal(form.get('token'), 'token');
  assert.equal(h.$('deviceStatus').textContent, 'Saved and verified.');
});
test('stale state and unsupported rates never write; ignored setters report failure and retain name', async () => {
  const h = harness(); await h.context.settings.load(); h.$('deviceRate').value = '192000';
  await h.$('deviceRate').events.change(); assert.equal(h.calls.filter(c => c.options).length, 0);
  h.data[`${h.entity}/entity_name`] = 'External'; h.$('deviceName').value = 'Draft';
  await h.$('saveDeviceName').events.click(); assert.equal(h.calls.filter(c => c.options).length, 0);
  await h.context.settings.load(); h.$('deviceName').value = 'Draft'; h.noApply();
  await h.$('saveDeviceName').events.click(); assert.equal(h.$('deviceStatus').textContent, 'The device did not apply this setting.');
  assert.equal(h.$('deviceName').value, 'Draft'); assert.equal(h.$('saveDeviceName').disabled, false);
});

test('native clock selection writes stream first with both expected bytes and handles media clock', async () => {
  const h = harness(); await h.context.settings.load();
  h.$('deviceClock').value = 'stream-16'; await h.$('deviceClock').events.change();
  const write = h.calls.find(c => c.options); const form = new URLSearchParams(write.options.body);
  assert.equal(write.url, '/api/console/changes');
  assert.equal(form.get('changes'), 'device-clock-stream:device:0:00:16;device-clock:device:0:00:4');
  assert.equal(h.records.get(0x1b5f), '10'); assert.equal(h.records.get(11), '04');
  assert.equal(h.$('deviceClock').value, 'stream-16'); assert.equal(h.$('deviceStatus').textContent, 'Saved and verified.');
});
test('native rate is a big-endian Hz value; Word Clock Out is true', async () => {
  const h = harness(); await h.context.settings.load();
  h.$('deviceRate').value = '48000'; await h.$('deviceRate').events.change();
  assert.equal(h.records.get(10), '0000bb80'); assert.equal(h.$('deviceRate').value, '48000');
  h.$('deviceWordClock').value = '1'; await h.$('deviceWordClock').events.change();
  assert.equal(h.records.get(12), '01');
});

test('hostnames with ports are not presented as IPv6; scoped literal addresses are retained', async () => {
  const h = harness(); h.$('host').value = 'motu.local:80'; await h.context.settings.load();
  assert.equal(h.$('deviceIpv6').value, '');
  h.$('host').value = '[fe80::1%eth2]:80'; await h.context.settings.load();
  assert.equal(h.$('deviceIpv6').value, 'fe80::1%eth2');
});
