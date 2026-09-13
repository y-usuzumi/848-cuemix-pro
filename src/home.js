// Device selection changes local session state; it never changes audio settings.
(() => {
  const historyKey = 'cuemix-848-recent-connections', historyLimit = 8;
  function mount(document, token, { request = fetch, navigate = url => location.assign(url), storage } = {}) {
    const byId = id => document.getElementById(id);
    const status = byId('connectStatus'), scan = byId('scanDevices');
    const connectButton = byId('connectDevice'), address = byId('deviceAddress');
    const recentList = byId('recentList'), clearRecent = byId('clearRecent');
    let connecting = false, scanning = false;
    let recent = [];
    const historyStorage = () => storage === undefined ? globalThis.localStorage : storage;
    try {
      const saved = JSON.parse(historyStorage().getItem(historyKey));
      if (Array.isArray(saved)) recent = [...new Set(saved.filter(host => typeof host === 'string' && host.trim() && host.length <= 256).map(host => host.trim()))].slice(0, historyLimit);
    } catch (_) { /* Storage may be unavailable or contain an older, invalid value. */ }
    function renderRecent() {
      recentList.replaceChildren();
      byId('recentConnections').hidden = recent.length === 0;
      for (const host of recent) {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'device-address';
        button.dataset.recentHost = host;
        button.disabled = connecting;
        button.setAttribute('aria-label', `Connect to ${host}`);
        const label = document.createElement('code'), action = document.createElement('span');
        label.textContent = host;
        action.textContent = 'Connect →';
        button.append(label, action);
        recentList.append(button);
      }
    }
    function remember(host) {
      recent = [host, ...recent.filter(previous => previous !== host)].slice(0, historyLimit);
      try { historyStorage().setItem(historyKey, JSON.stringify(recent)); } catch (_) {}
      renderRecent();
    }
    renderRecent();
    clearRecent.addEventListener('click', () => {
      if (connecting) return;
      recent = [];
      try { historyStorage().removeItem(historyKey); } catch (_) {}
      renderRecent();
    });
    async function post(path, values) {
      const response = await request(path, {
        method:'POST', headers:{'Content-Type':'application/x-www-form-urlencoded'},
        body:new URLSearchParams({ ...values, token }).toString()
      });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || `Server returned HTTP ${response.status}`);
      return result;
    }
    function busy(value) {
      connecting = value;
      connectButton.disabled = value;
      scan.disabled = value || scanning;
      clearRecent.disabled = value;
      recentList.querySelectorAll('button').forEach(button => { button.disabled = value; });
      connectButton.textContent = value ? 'Connecting…' : 'Connect';
      byId('deviceList').querySelectorAll('[data-device-host]').forEach(link => link.setAttribute('aria-disabled', String(value)));
    }
    async function connect(host, saveRecent = false) {
      if (connecting) return;
      busy(true);
      status.className = 'status';
      status.textContent = `Connecting to ${host}…`;
      try {
        const result = await post('/api/connect', {host});
        if (saveRecent) remember(result.host);
        navigate('/?' + new URLSearchParams({host:result.host}));
      } catch (error) {
        status.className = 'status error';
        status.textContent = error.message;
        busy(false);
      }
    }
    byId('manualConnect').addEventListener('submit', event => {
      event.preventDefault();
      connect(address.value.trim(), true);
    });
    recentList.addEventListener('click', event => {
      const button = event.target.closest('[data-recent-host]');
      if (!button || connecting) return;
      address.value = button.dataset.recentHost;
      connect(button.dataset.recentHost, true);
    });
    byId('deviceList').addEventListener('click', event => {
      const link = event.target.closest('[data-device-host]');
      if (!link) return;
      event.preventDefault();
      connect(link.dataset.deviceHost);
    });
    scan.addEventListener('click', async () => {
      if (scanning || connecting) return;
      scanning = true;
      scan.disabled = true;
      scan.textContent = 'Scanning…';
      byId('scanError').textContent = '';
      try {
        const result = await post('/api/discover', {});
        // The server builds this fragment with escaped discovery fields.
        byId('deviceList').innerHTML = result.html;
        byId('scanError').textContent = result.error ? `Discovery unavailable: ${result.error}. You can still connect by IP.` : '';
        if (connecting) busy(true);
      } catch (error) {
        byId('scanError').textContent = error.message;
      } finally {
        scanning = false;
        scan.disabled = connecting;
        scan.textContent = 'Scan again';
      }
    });
  }
  globalThis.CueMixHome = { mount };
})();
