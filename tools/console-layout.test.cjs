const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

function harness() {
  const elements = new Map();
  const document = { activeElement: null, getElementById: id => elements.get(id) };
  const context = { document };
  vm.runInNewContext(fs.readFileSync(require.resolve('../src/ui_layout.js'), 'utf8'), context);
  return { layout: context.CueMixLayout, document, elements };
}

test('banking preserves every sparse channel exactly once, including the final partial bank', () => {
  const { layout } = harness();
  const channels = [0, 2, 5, 9, 12, 15, 28].map(index => ({ index }));
  const visited = [];
  for (let i = 0; i < 3; i++) visited.push(...layout.page(channels, i, 3).items);
  assert.deepEqual(visited, channels);
  assert.equal(layout.page(channels, 99, 3).index, 2);
  assert.equal(layout.page([], 4, 3).index, 0);
});

test('hardware banking uses wide workspaces and keeps a focused edit visible when resized', () => {
  const { layout, elements, document } = harness();
  const active = { value: 34 };
  const strips = Array.from({ length: 12 }, (_, i) => ({
    classList: { contains: name => name === 'strip' },
    contains: node => i === 5 && node === active,
    binding: { channel: i, pendingSave: i === 5 },
    hidden: false,
  }));
  const workspace = { clientWidth: 2000 };
  elements.set('outputOut', { clientWidth: 164, parentElement: workspace, children: strips, style: { setProperty() {} } });
  elements.set('outputOutPages', { innerHTML: '' });
  document.activeElement = active;
  layout.hardware();
  assert.equal(strips.filter(strip => !strip.hidden).length, 12, 'all outputs fit despite the previously capped row width');
  workspace.clientWidth = 240;
  layout.hardware();
  assert.deepEqual(strips.filter(strip => !strip.hidden).map(strip => strip.binding.channel), [4, 5]);
  assert.equal(strips[5].binding.pendingSave, true);
  assert.equal(active.value, 34);
  assert.match(elements.get('outputOutPages').innerHTML, /5–6 of 12/);
});
