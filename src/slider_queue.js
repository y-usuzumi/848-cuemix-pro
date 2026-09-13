// One request at a time; retain only the latest unsent position per slider.
// An input stream must not postpone its own save as a debounce would.
(() => {
  'use strict';
  function createQueue({ send, ready = () => true, onError = () => {}, onIdle = () => {}, interval = 80 }) {
    const pending = new Map(), settled = new Map(), blocked = new Set();
    let timer = null, running = null, generation = 0;
    function schedule(delay = interval) {
      if (!running && timer === null && pending.size) timer = setTimeout(flush, delay);
    }
    async function flush() {
      clearTimeout(timer); timer = null;
      if (running || !pending.size) return;
      if (!ready()) { schedule(); return; }
      const [key, entry] = pending.entries().next().value;
      pending.delete(key);
      running = entry;
      const current = generation;
      try {
        await send(entry.payload);
        if (current === generation) settled.set(key, entry.value);
      } catch (error) {
        if (current === generation) {
          // A rejection or uncertain result ends this gesture. Mouseup and
          // already-buffered positions must not silently retry it.
          for (const item of [entry, ...pending.values()]) {
            blocked.add(item.key);
            onError(item.payload, error);
          }
          pending.clear();
        }
      } finally {
        running = null;
        if (pending.size) schedule([...pending.values()].some(item => item.final) ? 0 : interval);
        else if (current === generation) onIdle();
      }
    }
    return {
      begin(key) { blocked.delete(key); settled.delete(key); },
      enqueue(key, value, payload, final = false) {
        if (blocked.has(key)) return false;
        const prior = running?.key === key && running.generation === generation ? running.value : settled.get(key);
        if (prior === value) pending.delete(key);
        else pending.set(key, { key, value, payload, generation, final });
        if (final) { clearTimeout(timer); timer = null; flush(); }
        else schedule();
        if (!running && !pending.size) onIdle();
        return true;
      },
      has: key => running?.key === key || pending.has(key),
      busy: () => !!running || !!pending.size,
      reset() {
        generation++; clearTimeout(timer); timer = null;
        pending.clear(); settled.clear(); blocked.clear();
        // An old request may still be completing. Do not overlap it with
        // a new device's request; its completion cannot repopulate settled.
      },
    };
  }
  globalThis.CueMixSliders = { createQueue };
})();
