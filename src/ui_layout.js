// Presentation only: banking never reconstructs a hardware control or writes
// device state. Hidden strips keep their bindings, pending saves and meters.
globalThis.CueMixLayout = (() => {
  const pages = new Map();
  const el = id => document.getElementById(id);
  let inputBank = 'mic', outputBank = 'line', scheduled = false;
  let phoneHighlightTimer;
  function size(id, minimum = 108, maximum = Infinity) {
    const grid = el(id);
    // Measure available workspace, not the capped strip row: a short final
    // bank or a filtered result must still expand again when space returns.
    const workspace = id === 'consoleStrips' ? el('panel-mixer') : grid?.parentElement;
    const width = id === 'consoleStrips'
      ? (workspace?.clientWidth || 0) - (el('consoleMaster')?.clientWidth || 112)
      : workspace?.clientWidth;
    return Math.max(1, Math.min(maximum, Math.floor((width || grid?.clientWidth || 640) / minimum)));
  }
  function page(items, requested, count) {
    const index = Math.max(0, Math.min(requested, Math.ceil(items.length / count) - 1));
    return { index, size: count, items: items.slice(index * count, (index + 1) * count) };
  }
  function controls(kind, index, total, count) {
    return `<span>${total ? index * count + 1 : 0}–${Math.min(total, (index + 1) * count)} of ${total}</span><button type="button" class="secondary" data-bank-page="${kind}" data-delta="-1" aria-label="Previous channel bank"${index === 0 ? ' disabled' : ''}>‹</button><button type="button" class="secondary" data-bank-page="${kind}" data-delta="1" aria-label="Next channel bank"${(index + 1) * count >= total ? ' disabled' : ''}>›</button>`;
  }
  function hardware() {
    for (const id of ['preampOut', 'lineInputOut', 'outputOut']) {
      const grid = el(id);
      if (!grid || !grid.clientWidth) continue;
      const strips = Array.from(grid.children).filter(node => node.classList.contains('strip'));
      for (const strip of strips) {
        const name = strip.querySelector?.('.channel-name-button')?.textContent;
        if (!name) continue;
        strip.querySelector('.gain-fader')?.setAttribute('aria-label', `${name} gain`);
        strip.querySelectorAll('.switch input').forEach(input => {
          input.dataset.controlLabel ||= input.getAttribute('aria-label');
          input.setAttribute('aria-label', `${name} ${input.dataset.controlLabel}`);
        });
      }
      const count = size(id, id === 'preampOut' ? 136 : 112);
      const current = page(strips, pages.get(id) || 0, count);
      // A narrower window must not bank away a focused control mid-edit.
      const focused = strips.findIndex(strip => strip.contains(document.activeElement));
      if (focused >= 0) current.index = Math.floor(focused / count);
      pages.set(id, current.index);
      strips.forEach((strip, i) => { strip.hidden = i < current.index * count || i >= (current.index + 1) * count; });
      grid.style.setProperty('--strip-count', Math.max(1, Math.min(count, strips.length - current.index * count)));
      const pager = el(id + 'Pages');
      if (pager) pager.innerHTML = controls(id, current.index, strips.length, count);
    }
  }
  function banks() {
    el('micBank').hidden = inputBank !== 'mic';
    el('lineBank').hidden = inputBank !== 'line';
    el('outputLineBank').hidden = outputBank !== 'line';
    el('outputSetupBank').hidden = outputBank !== 'setup';
    document.querySelectorAll('[data-input-bank]').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.inputBank === inputBank)));
    document.querySelectorAll('[data-output-bank]').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.outputBank === outputBank)));
    hardware();
  }
  function activate(view) {
    const group = view === 'aux' ? 'mixer' : view === 'routing' ? 'patchbay' : view;
    document.body.dataset.view = view;
    document.querySelectorAll('.workspace-nav [data-tab]').forEach(button => {
      if (button.dataset.tab === group) button.setAttribute('aria-current', 'page');
      else button.removeAttribute('aria-current');
    });
    el('mixModes').hidden = !['mixer', 'aux'].includes(view);
    el('connectionModes').hidden = !['patchbay', 'routing'].includes(view);
    document.querySelectorAll('.workspace-modes button').forEach(button => button.setAttribute('aria-pressed', String((button.dataset.tab || button.dataset.viewLink) === view)));
    banks();
  }
  function init() {
    document.addEventListener('click', event => {
      const button = event.target.closest('button');
      if (!button) return;
      if (button.dataset.bankPage) {
        pages.set(button.dataset.bankPage, (pages.get(button.dataset.bankPage) || 0) + Number(button.dataset.delta));
        hardware();
      }
      if (button.dataset.inputBank) { inputBank = button.dataset.inputBank; banks(); }
      if (button.dataset.outputBank) { outputBank = button.dataset.outputBank; banks(); }
      if (button.dataset.viewLink) activateTab(button.dataset.viewLink);
      if (button.id === 'openMonitorSetup') { outputBank = 'setup'; activateTab('outputs'); banks(); }
      if (button.id === 'focusPhones') {
        const panel = el('headphonePanel');
        el('headphoneTitle').focus({ preventScroll: true });
        panel.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
        panel.classList.add('is-highlighted');
        clearTimeout(phoneHighlightTimer);
        phoneHighlightTimer = setTimeout(() => panel.classList.remove('is-highlighted'), 2000);
      }
    });
    const observer = new ResizeObserver(() => {
      if (scheduled) return;
      scheduled = true;
      requestAnimationFrame(() => { scheduled = false; hardware(); globalThis.consoleUi?.resize(); });
    });
    observer.observe(el('workspace'));
    banks();
  }
  return { size, page, controls, hardware, activate, init };
})();
