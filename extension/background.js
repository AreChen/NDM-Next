importScripts('rules.js');
const mediaByTab = new Map();
async function settings() { return { ...DEFAULTS, ...(await chrome.storage.local.get(DEFAULTS)) }; }
async function status() {
  return new Promise(resolve => {
    let done = false; const ws = new WebSocket('ws://127.0.0.1:10007/download', 'neatextension.v1');
    const finish = result => { if (done) return; done = true; clearTimeout(timer); try { ws.close(); } catch {} resolve(result); };
    const timer = setTimeout(() => finish(false), 1500); ws.onopen = () => finish(true); ws.onerror = () => finish(false); ws.onclose = () => finish(false);
  });
}
async function sendTask(item) {
  const store = await chrome.cookies.getAll({ url: item.url });
  const cookie = store.map(c => `${c.name}=${c.value}`).join('; ');
  const payload = encodeTask({ ...item, cookie });
  return new Promise(resolve => {
    let done = false; const ws = new WebSocket('ws://127.0.0.1:10007/download', 'neatextension.v1');
    const finish = value => { if (done) return; done = true; clearTimeout(timer); try { ws.close(); } catch {} resolve(value); };
    const timer = setTimeout(() => finish(false), 2000);
    ws.onopen = () => { try { ws.send(payload); setTimeout(() => finish(true), 100); } catch { finish(false); } };
    ws.onerror = () => finish(false); ws.onclose = () => finish(false);
  });
}
chrome.downloads.onCreated.addListener(async download => {
  try {
    const config = await settings();
    const item = { url: download.finalUrl || download.url, pageUrl: download.referrer, size: download.totalBytes >= 0 ? download.totalBytes : undefined, type: download.mime };
    if (shouldSkip(item, config) || shouldSkip({ ...item, url: download.url }, config)) return;
    if (item.size === undefined && config.minBytes > 0) {
      try {
        const response = await fetch(item.url, { method: 'HEAD', credentials: 'include', signal: AbortSignal.timeout(1500) });
        const length = response.headers.get('content-length');
        if (response.ok && length !== null && /^\d+$/.test(length)) item.size = Number(length);
      } catch {}
    }
    const [current] = await chrome.downloads.search({id: download.id});
    if (!current || current.state !== 'in_progress') return;
    if (current.totalBytes >= 0) item.size = current.totalBytes;
    if (shouldSkip(item, config)) return;
    if (await sendTask(item)) { await chrome.downloads.cancel(download.id); await chrome.downloads.erase({id: download.id}); }
  } catch (error) { console.warn('NDM Next: download left in browser', error.message); }
});
chrome.webRequest.onHeadersReceived.addListener(async details => {
  if (details.tabId < 0 || details.method !== 'GET') return;
  try {
    const headers = Object.fromEntries((details.responseHeaders || []).map(h => [h.name.toLowerCase(), h.value || '']));
    const type = (headers['content-type'] || '').split(';')[0].toLowerCase();
    if (!/^(audio|video)\//.test(type) && !/^(application\/(vnd\.apple\.mpegurl|x-mpegurl|dash\+xml))$/.test(type)) return;
    const config = await settings(); if (!config.media) return;
    const tab = await chrome.tabs.get(details.tabId); const size = Number(headers['content-length']);
    const item = {url: details.url, pageUrl: tab.url, size: headers['content-length'] && Number.isFinite(size) ? size : undefined, type};
    if (shouldSkip(item, config)) return;
    const entries = mediaByTab.get(details.tabId) || []; if (!entries.some(e => e.url === item.url)) entries.unshift(item);
    mediaByTab.set(details.tabId, entries.slice(0, 50));
  } catch {}
}, {urls:['http://*/*','https://*/*']}, ['responseHeaders']);
chrome.tabs.onUpdated.addListener((id, change) => { if (change.status === 'loading') mediaByTab.delete(id); });
chrome.tabs.onRemoved.addListener(id => mediaByTab.delete(id));
chrome.runtime.onMessage.addListener((message, sender, reply) => {
  (async () => {
    if (message.action === 'status') return { connected: await status() };
    if (message.action === 'media') return { items: mediaByTab.get(message.tabId) || [] };
    if (message.action === 'send') { const config = await settings(); if (shouldSkip(message.item, config)) return {ok: false}; return { ok: await sendTask(message.item) }; }
    return {};
  })().then(reply).catch(() => reply({ok:false})); return true;
});
