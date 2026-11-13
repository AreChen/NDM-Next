importScripts('rules.js', 'capture.js', 'handoff.js');

const mediaByTab = new Map();
const requests = new Map();
const pendingApprovals = new Map();
const HOST = 'com.ndmnext.host';
const RECOVERY_ALARM = 'ndm-next-recover';
let captureEnabled = false;
let expiryTimer;
let recoveryTimer;
let mediaSequence = 0;

async function settings() { return { ...DEFAULTS, ...(await chrome.storage.local.get(DEFAULTS)) }; }
function setCaptureEnabled(config) {
  captureEnabled = config.backend === 'next' && config.enabled;
  if (!captureEnabled) { requests.clear(); mediaByTab.clear(); }
}
settings().then(setCaptureEnabled).catch(() => {});
chrome.storage.onChanged?.addListener((changes,area) => {
  if (area === 'local' && (changes.backend || changes.enabled)) settings().then(setCaptureEnabled).catch(() => {});
});
function native(message) { return chrome.runtime.sendNativeMessage(HOST, message); }
function scheduleRecovery() {
  try { chrome.alarms?.create(RECOVERY_ALARM, {delayInMinutes:1}); } catch {}
  if (!recoveryTimer) {
    recoveryTimer = setTimeout(() => { recoveryTimer = undefined; handoff.recover().catch(scheduleRecovery); },3000);
    recoveryTimer?.unref?.();
  }
}
const handoff = new Handoff({native,storage:chrome.storage.local,downloads:chrome.downloads,uuid:()=>crypto.randomUUID(),onPending:scheduleRecovery});

// Existing installations retain the legacy backend. Brand-new installations
// default to the NDM Next desktop app.
chrome.runtime.onInstalled?.addListener(async ({reason}) => {
  const saved = await chrome.storage.local.get('backend');
  if (!saved.backend) await chrome.storage.local.set({backend:reason === 'install' ? 'next' : 'neatdm'});
});
chrome.runtime.onStartup?.addListener(() => { handoff.recover().catch(scheduleRecovery); });
chrome.alarms?.onAlarm.addListener(alarm => {
  if (alarm.name === RECOVERY_ALARM) handoff.recover().catch(scheduleRecovery);
});
handoff.recover().catch(() => {});

chrome.webRequest.onBeforeSendHeaders?.addListener(details => {
  if (!captureEnabled) return;
  const now = Date.now();
  for (const [id,entry] of requests) if (now-entry.when > 120000) requests.delete(id);
  // Keep unsafe requests as markers so a simultaneous safe request for the
  // same URL cannot be mistaken for a POST or a request with custom headers.
  const entry = captureRequest(details) || {requestId:details.requestId,url:details.url,headers:null};
  requests.set(details.requestId,{...entry,when:now});
  if (requests.size > 256) requests.delete(requests.keys().next().value);
  if (!expiryTimer) {
    expiryTimer = setTimeout(() => { requests.clear(); expiryTimer = undefined; },120000);
    expiryTimer?.unref?.();
  }
}, {urls:['http://*/*','https://*/*']}, ['requestHeaders','extraHeaders']);

function reliablePage(value) {
  try {
    const url = new URL(value);
    return ['http:','https:'].includes(url.protocol) && !url.username && !url.password ? url.href : null;
  } catch { return null; }
}

function browserTask(item, download, mediaCapture) {
  const capture = mediaCapture || findCapture([...requests.values()],item.url);
  if (!capture) return null;
  const filename = typeof download?.filename === 'string' ? download.filename.split(/[\\/]/).pop() : undefined;
  return {
    url:item.url, source_url:reliablePage(item.pageUrl), suggested_filename:filename || null,
    expected_size:!isHls(item) && Number.isSafeInteger(item.size) && item.size >= 0 ? item.size : null,
    media_type:item.type || null,
    headers:capture.headers,
  };
}

async function legacyStatus() {
  return new Promise(resolve => {
    let done = false; const ws = new WebSocket('ws://127.0.0.1:10007/download', 'neatextension.v1');
    const finish = result => { if (done) return; done = true; clearTimeout(timer); try { ws.close(); } catch {} resolve(result); };
    const timer = setTimeout(() => finish(false), 1500); ws.onopen = () => finish(true); ws.onerror = () => finish(false); ws.onclose = () => finish(false);
  });
}
async function status(config) {
  if (config.backend !== 'next') return legacyStatus();
  try { return !!(await native({type:'status'}))?.ok; } catch { return false; }
}
async function sendLegacy(item) {
  const store = await chrome.cookies.getAll({ url:item.url });
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

async function askToTakeOver(download, task, config) {
  if (!chrome.notifications) return;
  const id = `ndm-next-confirm:${download.id}`;
  const chinese = config.language === 'zh' || (config.language === 'auto' && chrome.i18n.getUILanguage().toLowerCase().startsWith('zh'));
  const pending = {downloadId:download.id,task,policy:config.transferPolicy,expires:Date.now()+300000};
  pendingApprovals.set(id,pending);
  // Session storage is memory-only and survives a service-worker restart. Never
  // persist captured authorization headers or cookies in storage.local.
  try { await chrome.storage.session?.set({[id]:pending}); } catch {}
  try {
    await chrome.notifications.create(id, {
      type:'basic',iconUrl:'icons/icon-128.png',title:'NDM Next',
      message:chinese ? '使用 NDM Next 下载此文件？' : 'Send this download to NDM Next?',
      buttons:[{title:chinese ? '接管下载' : 'Send download'},{title:chinese ? '留在浏览器' : 'Keep in browser'}],
    });
  } catch { await forgetApproval(id); }
}
async function forgetApproval(id) {
  pendingApprovals.delete(id);
  try { await chrome.storage.session?.remove(id); } catch {}
}
chrome.notifications?.onButtonClicked.addListener((id,button) => {
  (async () => {
    let pending = pendingApprovals.get(id);
    if (!pending) { try { pending = (await chrome.storage.session?.get(id))?.[id]; } catch {} }
    await forgetApproval(id);
    chrome.notifications.clear(id).catch(() => {});
    if (pending && pending.expires >= Date.now() && button === 0) await handoff.transfer(pending.downloadId,pending.task,pending.policy);
  })().catch(() => {});
});
chrome.notifications?.onClosed.addListener(id => { forgetApproval(id).catch(() => {}); });

chrome.downloads.onCreated.addListener(async download => {
  try {
    const config = await settings();
    const item = { url:download.finalUrl || download.url, pageUrl:download.referrer, size:download.totalBytes >= 0 ? download.totalBytes : undefined, type:download.mime };
    if ((config.blockedDomains || []).length && !reliablePage(item.pageUrl)) return;
    if (shouldSkip(item,config) || shouldSkip({ ...item, url:download.url },config)) return;
    // Do not issue an extra probe for the new backend: it might consume a
    // one-time URL. Unknown size remains eligible as documented.
    if (config.backend !== 'next' && item.size === undefined && config.minBytes > 0) {
      try {
        const response = await fetch(item.url, { method:'HEAD', credentials:'include', signal:AbortSignal.timeout(1500) });
        const length = response.headers.get('content-length');
        if (response.ok && length !== null && /^\d+$/.test(length)) item.size = Number(length);
      } catch {}
    }
    const [current] = await chrome.downloads.search({id:download.id});
    if (!current || current.state !== 'in_progress') return;
    if (current.totalBytes >= 0) item.size = current.totalBytes;
    item.url = current.finalUrl || item.url;
    if (shouldSkip(item,config)) return;
    if (config.backend === 'next') {
      const task = browserTask(item,current);
      if (!task) return;
      if (config.captureMode === 'confirm') await askToTakeOver(current,task,config);
      else await handoff.transfer(download.id,task,config.transferPolicy);
    } else if (await sendLegacy(item)) {
      await chrome.downloads.cancel(download.id);
      await chrome.downloads.erase({id:download.id});
    }
  } catch { console.warn('NDM Next: download left in browser'); }
});
chrome.downloads.onChanged?.addListener(delta => { if (delta.state || delta.error) handoff.recover().catch(scheduleRecovery); });

chrome.webRequest.onHeadersReceived.addListener(async details => {
  if (details.tabId < 0 || details.method !== 'GET') return;
  const sequence = ++mediaSequence;
  const captured = requests.get(details.requestId);
  const capture = captured?.url === details.url && captured?.tabId === details.tabId && captured?.headers ? captured : null;
  try {
    const headers = Object.fromEntries((details.responseHeaders || []).map(h => [h.name.toLowerCase(), h.value || '']));
    const type = (headers['content-type'] || '').split(';')[0].toLowerCase();
    if (!/^(audio|video)\//.test(type) && !/^(application\/(vnd\.apple\.mpegurl|x-mpegurl|dash\+xml))$/.test(type)) return;
    const config = await settings(); if (!config.media) return;
    const tab = await chrome.tabs.get(details.tabId); const size = Number(headers['content-length']);
    const item = {url:details.url,pageUrl:tab.url,size:headers['content-length'] && Number.isFinite(size) ? size : undefined,type};
    if (isHls(item)) item.size = undefined; // This is the playlist's size, not the media size.
    if (shouldSkip(item,config) || ((config.blockedDomains || []).length && !reliablePage(item.pageUrl))) return;
    const previous = mediaByTab.get(details.tabId) || [];
    if (previous.some(entry => entry.item.url === item.url && entry.sequence > sequence)) return;
    const entries = previous.filter(entry => entry.item.url !== item.url);
    entries.unshift({item,capture,sequence});
    mediaByTab.set(details.tabId,entries.slice(0,50));
  } catch {}
}, {urls:['http://*/*','https://*/*']}, ['responseHeaders']);
chrome.tabs.onUpdated.addListener((id,change) => { if (change.status === 'loading') mediaByTab.delete(id); });
chrome.tabs.onRemoved.addListener(id => mediaByTab.delete(id));
chrome.runtime.onMessage.addListener((message,sender,reply) => {
  (async () => {
    const config = await settings();
    if (message.action === 'status') return {connected:await status(config)};
    if (message.action === 'media') return {items:(mediaByTab.get(message.tabId) || []).map(entry => entry.item)};
    if (message.action === 'send') {
      const entry = (mediaByTab.get(message.tabId) || []).find(entry => entry.item.url === message.item?.url);
      if (!entry || shouldSkip(entry.item,config) || ((config.blockedDomains || []).length && !reliablePage(entry.item.pageUrl))) return {ok:false};
      if (config.backend !== 'next') return {ok:await sendLegacy(entry.item)};
      if (!entry.capture || Date.now()-entry.capture.when > 1800000) return {ok:false};
      const task = browserTask(entry.item,undefined,entry.capture);
      return {ok:!!task && await handoff.send(task)};
    }
    return {};
  })().then(reply).catch(() => reply({ok:false})); return true;
});
