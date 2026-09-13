// Exact-value editing commits through the slider's existing transport queue.
globalThis.CueMixDb = (() => {
  const infinity = input => input.dataset.dbInfinity === undefined ? null : Number(input.dataset.dbInfinity);
  function parse(text, slider, input) {
    const value = String(text).trim().toLowerCase().replaceAll('−', '-').replace(/\s*db$/, '').trim();
    const silent = infinity(input), min = Number(slider.min), max = Number(slider.max), step = Number(slider.step);
    if (silent !== null && ['-inf', '-infinity', '-∞'].includes(value)) return silent;
    const number = Number(value);
    if (silent !== null && value && number === silent) return silent;
    const finiteMin = Number(input.dataset.dbMin ?? (silent === min ? min + step : min));
    const hint = `Enter ${silent === null ? '' : '-inf or '}a ${step === 1 ? 'whole number' : 'number'} from ${finiteMin} to ${max} dB${step < 1 ? ` in ${step} dB steps` : ''}.`;
    if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)$/.test(value) || !Number.isFinite(number) || number < finiteMin || number > max || Math.abs((number - min) / step - Math.round((number - min) / step)) > 1e-7) throw Error(hint);
    return number;
  }
  function display(input, slider) {
    const value = Number(slider.value);
    const text = value === infinity(input) ? '−∞' : input.dataset.dbDecimals ? value.toFixed(Number(input.dataset.dbDecimals)) : String(value);
    return text + (input.dataset.dbUnit === 'none' ? '' : ' dB');
  }
  function sync(input, text) {
    if (!input) return;
    input.dataset.dbDisplay = text;
    if (globalThis.document?.activeElement !== input) input.value = text;
  }
  function editing(slider) { return !!slider.id && globalThis.document?.activeElement?.dataset?.dbFor === slider.id; }
  function mount(document, onError = () => {}) {
    const drafts = new WeakMap();
    const sliderFor = input => input.dataset?.dbFor ? document.getElementById(input.dataset.dbFor) : null;
    function finish(input, slider) {
      drafts.delete(input);
      input.setCustomValidity('');
      input.value = input.dataset.dbDisplay ?? display(input, slider);
    }
    function commit(input, slider, report) {
      const draft = drafts.get(input);
      if (!draft) return true;
      if (!draft.edited || input.disabled || slider.disabled || !slider.isConnected) { finish(input, slider); return true; }
      try {
        const value = parse(input.value, slider, input);
        const changed = Number(slider.value) !== value || input.dataset.dbMixed === 'true';
        drafts.delete(input);
        input.setCustomValidity('');
        slider.value = String(value);
        input.dataset.dbMixed = 'false';
        input.value = input.dataset.dbDisplay = display(input, slider);
        if (changed) slider.dispatchEvent(new Event('dbcommit', {bubbles:true}));
        return true;
      } catch (error) {
        onError(error.message);
        if (report) { input.setCustomValidity(error.message); input.reportValidity(); }
        else finish(input, slider);
        return false;
      }
    }
    document.addEventListener('focusin', event => {
      const input = event.target, slider = sliderFor(input);
      if (!slider || input.disabled || slider.disabled) return;
      input.dataset.dbDisplay = input.value;
      drafts.set(input, {edited:false});
      input.value = Number(slider.value) === infinity(input) ? '-inf' : String(Number(slider.value));
      input.select();
    });
    document.addEventListener('input', event => {
      const draft = drafts.get(event.target);
      if (draft) { draft.edited = true; event.target.setCustomValidity(''); }
    });
    document.addEventListener('focusout', event => {
      const slider = sliderFor(event.target);
      if (slider) commit(event.target, slider, false);
    });
    document.addEventListener('keydown', event => {
      const input = event.target, slider = sliderFor(input);
      if (!slider || event.isComposing) return;
      if (event.key === 'Escape') { event.preventDefault(); finish(input, slider); input.blur(); }
      if (event.key === 'Enter') { event.preventDefault(); if (commit(input, slider, true)) input.blur(); }
    });
  }
  return {parse, sync, editing, mount};
})();
