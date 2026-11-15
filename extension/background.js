importScripts('rules.js', 'capture.js', 'handoff.js');

const mediaByTab = new Map();
const requests = new Map();
const pendingApprovals = new Map();
const tabEpochs = new Map();
const mediaNotices = new Map();
const SETTING_KEYS = new Set(Object.keys(DEFAULTS));
const HOST = 'com.ndmnext.host';
const RECOVERY_ALARM = 'ndm-next-recover';
let captureEnabled = false;
let captureInitialized = false;
let mediaEnabled = true;
let expiryTimer;
let recoveryTimer;
let mediaSequence = 0;
const MEDIA_SESSION_KEY = 'ndm-next-media-v1';
const MEDIA_TTL = 1800000;
const invalidatedMediaTabs = new Set();
let mediaSessionTrusted = false;
let mediaDirty = false;
let mediaPersistTimer;
let mediaPersistTail = Promise.resolve();

async function settings() { return { ...DEFAULTS, ...(await chrome.storage.local.get(DEFAULTS)) }; }
function setCaptureEnabled(config) {
  captureEnabled = config.backend === 'next' && config.enabled;
  mediaEnabled = config.enabled && config.media;
  captureInitialized = true;
  if (!captureEnabled && !mediaEnabled) requests.clear();
  if (!mediaEnabled) { mediaByTab.clear(); scheduleMediaPersist(); }
}
const captureReady = settings().then(setCaptureEnabled).catch(() => {});
const mediaReady = restoreMedia();

function currentMedia(entry, now = Date.now()) {
  return Number.isFinite(entry.when) && entry.when <= now && now-entry.when <= MEDIA_TTL
    && (!entry.capture || (entry.capture.when <= now && now-entry.capture.when <= MEDIA_TTL));
}
function pruneMedia() {
  for (const [tabId,entries] of mediaByTab) {
    const current=entries.filter(entry=>currentMedia(entry));
    if (current.length !== entries.length) {
      if (current.length) mediaByTab.set(tabId,current); else mediaByTab.delete(tabId);
      mediaDirty=true;
    }
  }
}
async function restoreMedia() {
  const session=chrome.storage.session;
  if (!session?.get || !session?.set) return;
  try {
    // session is trusted-only by default; explicitly retain that boundary on
    // browsers supporting access levels. Never expose replay headers to pages.
    await session.setAccessLevel?.({accessLevel:'TRUSTED_CONTEXTS'});
    mediaSessionTrusted=true;
    const saved=(await session.get(MEDIA_SESSION_KEY))?.[MEDIA_SESSION_KEY];
    if (saved) scheduleMediaPersist(); // Rewrite away expired or invalid entries.
    await captureReady;
    if (!mediaEnabled || saved?.version !== 1 || !Array.isArray(saved.entries)) return;
    const tabs=new Map();
    for (const entry of saved.entries.slice(0,500)) {
      const {tabId,item,capture}=entry || {};
      if (!Number.isInteger(tabId) || tabId < 0 || !item || !currentMedia(entry)
        || !Number.isSafeInteger(entry.sequence) || entry.sequence < 1
        || typeof item.id !== 'string' || !Number.isInteger(item.frameId) || item.frameId < 0
        || !reliablePage(item.url) || !reliablePage(item.pageUrl) || invalidatedMediaTabs.has(tabId)) continue;
      if (!tabs.has(tabId)) tabs.set(tabId,await chrome.tabs.get(tabId).catch(()=>null));
      if (!mediaEnabled || invalidatedMediaTabs.has(tabId) || tabs.get(tabId)?.url !== item.pageUrl) continue;
      let replay=null;
      if (capture) {
        if (capture.tabId !== tabId || capture.url !== item.url || capture.frameId !== item.frameId
          || (capture.documentId && item.documentId && capture.documentId !== item.documentId) || !capture.headers) continue;
        replay=captureRequest({...capture,method:'GET',requestHeaders:Object.entries(capture.headers).map(([name,value])=>({name,value}))});
        if (!replay) continue;
        replay.when=capture.when;
      }
      const entries=mediaByTab.get(tabId) || [];
      if (entries.length >= 50) continue;
      entries.push({...entry,capture:replay});mediaByTab.set(tabId,entries);
      mediaSequence=Math.max(mediaSequence,entry.sequence);
    }
  } catch {}
}
function scheduleMediaPersist() {
  mediaDirty=true;
  if (!mediaPersistTimer) {
    mediaPersistTimer=setTimeout(()=>{mediaPersistTimer=undefined;persistMedia().catch(()=>{});},150);
    mediaPersistTimer?.unref?.();
  }
}
function persistMedia() {
  clearTimeout(mediaPersistTimer);mediaPersistTimer=undefined;
  // Serialize writes and build the snapshot when each write starts, so a
  // navigation racing a pending write cannot restore the previous document.
  mediaPersistTail=mediaPersistTail.catch(()=>{}).then(async()=>{
    await mediaReady;
    pruneMedia();
    if (!mediaDirty || !mediaSessionTrusted) return;
    mediaDirty=false;
    const candidates=[...mediaByTab].flatMap(([tabId,entries])=>entries.map(entry=>({tabId,...entry})))
      .sort((a,b)=>b.sequence-a.sequence);
    const entries=[];let bytes=0;
    // Keep well below session's quota, including large authorization headers.
    for (const entry of candidates) {
      const size=JSON.stringify(entry).length*3;
      if (entries.length >= 500 || bytes+size > 4*1024*1024) continue;
      entries.push(entry);bytes+=size;
    }
    try { await chrome.storage.session.set({[MEDIA_SESSION_KEY]:{version:1,entries}}); } catch { mediaDirty=true; }
  });
  return mediaPersistTail;
}
chrome.storage.onChanged?.addListener((changes,area) => {
  if (area === 'local' && Object.keys(changes).some(key=>SETTING_KEYS.has(key))) settings().then(config => {
    const tabIds=[...mediaByTab.keys()];
    setCaptureEnabled(config);
    for (const id of tabIds) notifyMedia(id);
  }).catch(() => {});
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
    // Store synchronously: response observers must not race storage initialization.
    if (captureInitialized && !captureEnabled && !mediaEnabled) return;
    const now = Date.now();
    for (const [id,entry] of requests) if (now-entry.when > 120000) requests.delete(id);
    // Keep unsafe requests as markers so a simultaneous safe request for the
    // same URL cannot be mistaken for a POST or a request with custom headers.
    const entry = captureRequest(details) || {requestId:details.requestId,url:details.url,tabId:details.tabId,frameId:details.frameId ?? 0,documentId:details.documentId,headers:null};
    requests.set(details.requestId,{...entry,mediaCapture:captureRequest(details,{media:true}),when:now});
    if (requests.size > 256) requests.delete(requests.keys().next().value);
    if (!expiryTimer) {
      expiryTimer = setTimeout(() => { requests.clear(); expiryTimer = undefined; },120000);
      expiryTimer?.unref?.();
    }
}, {urls:['http://*/*','https://*/*']}, ['requestHeaders','extraHeaders']);

function notifyMedia(tabId) {
  if (mediaNotices.has(tabId)) return;
  const timer=setTimeout(() => {
    mediaNotices.delete(tabId);
    try { chrome.tabs.sendMessage?.(tabId,{action:'mediaChanged'})?.catch(() => {}); } catch {}
    // Popup listeners are not tabs.
    try { chrome.runtime.sendMessage?.({action:'mediaChanged',tabId})?.catch(() => {}); } catch {}
  },150);
  timer?.unref?.();mediaNotices.set(tabId,timer);
}

function reliablePage(value) {
  try {
    const url = new URL(value);
    return ['http:','https:'].includes(url.protocol) && !url.username && !url.password ? url.href : null;
  } catch { return null; }
}

function browserTask(item, download, mediaCapture, originalUrl) {
  const captures = [...requests.values()];
  let capture = mediaCapture || findCapture(captures,item.url);
  // Redirect targets can lack an observable request capture. Replay the
  // original URL only when there is no target capture (including unsafe ones).
  // The desktop strips credentials on cross-origin redirects.
  const now = Date.now();
  if (!capture && !mediaCapture && originalUrl && originalUrl !== item.url && !captures.some(entry => entry.url === item.url && now >= entry.when && now-entry.when <= 120000)) {
    capture = findCapture(captures,originalUrl);
  }
  if (!capture) return null;
  const filename = typeof download?.filename === 'string' ? download.filename.split(/[\\/]/).pop() : undefined;
  return {
    url:capture.url, source_url:reliablePage(item.pageUrl), suggested_filename:filename || null,
    expected_size:!isMediaManifest(item) && Number.isSafeInteger(item.size) && item.size > 0 ? item.size : null,
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
async function sendLegacy(item,capture) {
  let cookie;
  if (!capture) {const store = await chrome.cookies.getAll({ url:item.url });cookie=store.map(c => `${c.name}=${c.value}`).join('; ');}
  const payload = encodeTask({ ...item, cookie, replayHeaders:capture?.headers });
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

function knownDownloadSize(value) {
  // Chromium may expose zero before the actual length is known.
  return Number.isSafeInteger(value) && value > 0 ? value : undefined;
}

chrome.downloads.onCreated.addListener(async download => {
  try {
    await captureReady;
    const config = await settings();
    const item = { url:download.finalUrl || download.url, pageUrl:download.referrer, size:knownDownloadSize(download.totalBytes), type:download.mime };
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
    const currentSize = knownDownloadSize(current.totalBytes);
    if (currentSize !== undefined) item.size = currentSize;
    item.url = current.finalUrl || item.url;
    if (shouldSkip(item,config)) return;
    if (config.backend === 'next') {
      const task = browserTask(item,current,undefined,download.url);
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
  const epoch = tabEpochs.get(details.tabId);
  const captured = requests.get(details.requestId);
  const replay = captured?.mediaCapture;
  const capture = replay?.url === details.url && replay?.tabId === details.tabId && replay?.frameId === (details.frameId ?? 0)
    && (!details.documentId || !replay.documentId || details.documentId === replay.documentId)
    ? {...replay,when:captured.when} : null;
  await mediaReady;
  await captureReady;
  const sequence = ++mediaSequence;
  try {
    const headers = Object.fromEntries((details.responseHeaders || []).map(h => [h.name.toLowerCase(), h.value || '']));
    const type = (headers['content-type'] || '').split(';')[0].trim().toLowerCase();
    if (!/^(audio|video)\//.test(type) && !isMediaManifest({url:details.url,type})) return;
    const config = await settings(); if (!config.media) return;
    if (details.statusCode && (details.statusCode < 200 || details.statusCode >= 300)) return;
    const tab = await chrome.tabs.get(details.tabId);
    const item = {id:`media-${sequence}`,url:details.url,pageUrl:tab.url,size:mediaSize(details,headers),type,frameId:details.frameId ?? 0,documentId:details.documentId || captured?.documentId};
    if (isMediaManifest(item)) item.size = undefined; // Manifest length is not the resulting media length.
    if (isMediaSegment(item)) item.size = undefined;
    if (shouldSkip(item,config) || ((config.blockedDomains || []).length && !reliablePage(item.pageUrl))) return;
    if (tabEpochs.get(details.tabId) !== epoch) return;
    pruneMedia();
    const previous = mediaByTab.get(details.tabId) || [];
    // A stream may request thousands of segments. Keep its manifest selectable
    // instead of letting fragments replace the actual download choice.
    const sameDocument=entry=>entry.item.frameId === item.frameId && entry.item.documentId === item.documentId;
    if (isMediaSegment(item) && previous.some(entry=>sameDocument(entry) && isMediaManifest(entry.item))) return;
    const same=entry=>entry.item.url === item.url && sameDocument(entry);
    if (previous.some(entry => same(entry) && entry.sequence > sequence)) return;
    item.id=previous.find(same)?.item.id || item.id;
    const entries = previous.filter(entry => !same(entry) && !(isMediaManifest(item) && sameDocument(entry) && isMediaSegment(entry.item)));
    entries.unshift({item,capture,sequence,when:captured?.when ?? Date.now()});
    mediaByTab.set(details.tabId,entries.slice(0,50));
    scheduleMediaPersist();
    notifyMedia(details.tabId);
  } catch {}
}, {urls:['http://*/*','https://*/*']}, ['responseHeaders']);
function mediaSize(details, headers) {
  if (headers['content-encoding'] && headers['content-encoding'].toLowerCase() !== 'identity') return undefined;
  if (details.statusCode === 206 || headers['content-range']) {
    const match = /^bytes (\d+)-(\d+)\/(\d+)$/i.exec(headers['content-range'] || '');
    if (!match) return undefined;
    const [start,end,total] = match.slice(1).map(Number);
    const length=headers['content-length'];
    const matchesLength=length === undefined || (/^\d+$/.test(length) && Number(length) === end-start+1);
    return matchesLength && [start,end,total].every(Number.isSafeInteger) && start <= end && end < total ? total : undefined;
  }
  const length = headers['content-length'];
  const size = /^\d+$/.test(length || '') ? Number(length) : NaN;
  return Number.isSafeInteger(size) && size > 0 ? size : undefined;
}
chrome.tabs.onUpdated.addListener((id,change) => {
  if (change.status === 'loading') { invalidatedMediaTabs.add(id);mediaByTab.delete(id); tabEpochs.set(id,(tabEpochs.get(id) || 0)+1);mediaDirty=true;persistMedia().catch(()=>{}); }
});
chrome.tabs.onRemoved.addListener(id => { invalidatedMediaTabs.add(id);mediaByTab.delete(id); tabEpochs.set(id,(tabEpochs.get(id) || 0)+1);clearTimeout(mediaNotices.get(id));mediaNotices.delete(id);mediaDirty=true;persistMedia().catch(()=>{}); });
function mediaTab(message,sender) {
  if (sender.id !== chrome.runtime.id) return null;
  if (sender.tab) {
    if (!Number.isInteger(sender.tab.id) || (message.tabId !== undefined && message.tabId !== sender.tab.id)) return null;
    return sender.tab.id;
  }
  return sender.id === chrome.runtime.id && sender.url === chrome.runtime.getURL?.('popup.html') && Number.isInteger(message.tabId) ? message.tabId : null;
}
function visibleMedia(tabId,sender,config) {
  return (mediaByTab.get(tabId) || []).filter(({item}) => config.media && !shouldSkip(item,config)
    && (!sender.tab || (item.frameId === (sender.frameId ?? 0)
      && (!item.documentId || !sender.documentId || item.documentId === sender.documentId))));
}
chrome.runtime.onMessage.addListener((message,sender,reply) => {
  (async () => {
    let config = await settings();
    if (message.action === 'status') return {connected:await status(config)};
    const tabId = mediaTab(message,sender);
    if ((message.action === 'media' || message.action === 'send') && tabId !== null) {
      await mediaReady;
      await persistMedia();
      config = await settings();
    }
    if (message.action === 'media') {
      if (tabId === null) return {ok:false,error:'invalid_sender'};
      return {items:visibleMedia(tabId,sender,config).map(entry => entry.item),inlineMedia:config.inlineMedia,language:config.language};
    }
    if (message.action === 'send') {
      if (tabId === null) return {ok:false,error:'invalid_sender'};
      const entry = visibleMedia(tabId,sender,config).find(entry => message.id ? entry.item.id === message.id : entry.item.url === message.item?.url);
      if (!entry || ((config.blockedDomains || []).length && !reliablePage(entry.item.pageUrl))) return {ok:false,error:'media_unavailable'};
      if (!entry.capture) return {ok:false,error:'unsupported_request'};
      if (!currentMedia(entry)) return {ok:false,error:'capture_expired'};
      if (config.backend !== 'next') { const ok=await sendLegacy(entry.item,entry.capture); return {ok,error:ok?undefined:'manager_unavailable'}; }
      const task = browserTask(entry.item,undefined,entry.capture);
      const ok=!!task && await handoff.send(task);
      return {ok,error:ok?undefined:'manager_unavailable'};
    }
    return {};
  })().then(reply).catch(() => reply({ok:false})); return true;
});
