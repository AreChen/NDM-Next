const $ = id => document.getElementById(id);
let config, tab, language = 'en', messageTimer;
let contextSequence = 0;

function text(key, ...values) { return extensionMessage(key, language, ...values); }

function notice(message, error = false) {
  clearTimeout(messageTimer);
  $('message').textContent = message;
  $('message').classList.toggle('error', error);
  messageTimer = setTimeout(() => { $('message').textContent = ''; }, error ? 6000 : 2400);
}

async function translate() {
  language = extensionLocale(config.language, chrome.i18n.getUILanguage());
  await loadCatalog(language);
  localizeDocument(language);
  $('language').replaceChildren(...[
    ['auto', text('followBrowser')],
    ...Object.entries(EXTENSION_LANGUAGES).map(([id, entry]) => [id, entry.label]),
  ].map(([value, label]) => {
    const option = document.createElement('option');
    option.value = value;
    option.textContent = label;
    return option;
  }));
  $('language').value = Object.hasOwn(EXTENSION_LANGUAGES, config.language) ? config.language : 'auto';
}

function renderSite() {
  const domain = /^https?:/i.test(tab?.url || '') ? normalizeDomain(tab.url) : '';
  const blocked = config.blockedDomains.some(rule => domainMatches(domain, rule));
  $('domain').textContent = domain || '—';
  $('siteState').textContent = text(!domain ? 'noSite' : blocked ? 'siteBlocked' : 'siteAllowed');
  $('blockSite').textContent = text(blocked ? 'unblockSite' : 'blockSite');
  $('blockSite').disabled = !domain;
  $('openMedia').disabled = !domain;
}

async function renderResources() {
  if (!Number.isInteger(tab?.id)) return;
  const result = await chrome.runtime.sendMessage({action: 'media', tabId: tab.id});
  const count = result?.items?.length ?? 0;
  $('mediaCount').textContent = new Intl.NumberFormat(language).format(count);
  $('mediaCount').title = text('resourceCount', count);
  const list = $('mediaList');
  list.replaceChildren();
  const items = (result?.items || []).sort((a, b) => Number(mediaKind(a) === 'image') - Number(mediaKind(b) === 'image'));
  list.hidden = !count;
  for (const item of items.slice(0, 3)) {
    const row = document.createElement('div');
    const info = document.createElement('div');
    info.className = 'stack';
    const title = document.createElement('strong');
    title.textContent = mediaName(item, language);
    title.title = title.textContent;
    const detail = document.createElement('span');
    detail.className = 'small';
    detail.textContent = mediaFormat(item) + ' · ' + mediaSizeLabel(item, language);
    const button = document.createElement('button');
    button.className = 'quiet';
    button.textContent = text('download');
    button.onclick = () => sendResource(item.id, button);
    info.append(title, detail);
    row.append(info, button);
    list.append(row);
  }
}

async function sendResource(id, button) {
  button.disabled = true;
  try {
    const result = await chrome.runtime.sendMessage({action: 'send', tabId: tab.id, id});
    notice(mediaMessage(result?.ok ? 'sent' : result?.error, language), !result?.ok);
  } catch {
    notice(mediaMessage('manager_unavailable', language), true);
  } finally { button.disabled = false; }
}

async function renderContext(refresh = false) {
  const sequence = ++contextSequence;
  const result = await chrome.runtime.sendMessage({action:'context', tabId:tab?.id, refresh}).catch(() => null);
  if (sequence !== contextSequence) return;
  const status = $('status');
  const state = result?.connected ? 'online' : 'offline';
  status.dataset.state = state;
  status.textContent = text(state);
  status.title = status.textContent;
  $('desktopVersion').textContent = result?.desktopVersion ? 'v' + result.desktopVersion : '—';
  const plugins = $('plugins');
  plugins.replaceChildren();
  for (const plugin of result?.plugins || []) {
    const row = document.createElement('div');
    const name = document.createElement('span');
    name.textContent = pluginName(plugin, language);
    const version = document.createElement('code');
    version.textContent = 'v' + plugin.version;
    row.append(name, version);
    plugins.append(row);
  }
  if (!plugins.childElementCount) {
    plugins.textContent = text(result?.desktopVersion ? 'noPlugins' : 'pluginInfoUnavailable');
    plugins.classList.add('muted', 'small');
  } else {
    plugins.classList.remove('muted', 'small');
  }
  renderPageActions($('pageActions'), result?.pages || [], language, sendResource);
}

async function start() {
  config = {...DEFAULTS, ...await chrome.storage.local.get(DEFAULTS)};
  [tab] = await chrome.tabs.query({active: true, currentWindow: true});
  await translate();
  const version = chrome.runtime.getManifest?.().version;
  $('extensionVersion').textContent = version ? 'v' + version : '—';
  $('browserVersion').textContent = $('extensionVersion').textContent;
  for (const key of ['enabled', 'media', 'inlineMedia']) $(key).checked = config[key];
  for (const key of ['backend', 'captureMode', 'transferPolicy']) $(key).value = config[key];
  $('minSize').value = config.minBytes / 1048576;
  $('extensions').value = config.excludedExtensions.join(', ');
  $('domains').value = config.blockedDomains.join('\n');
  renderSite();
  await Promise.all([renderResources(), renderContext(true)]);
}

async function refreshSettings() {
  renderSite();
  await Promise.all([renderResources(), renderContext(true)]);
}

for (const key of ['enabled', 'media', 'inlineMedia']) {
  $(key).onchange = async event => {
    config[key] = event.target.checked;
    await chrome.storage.local.set({[key]: config[key]});
    if (key !== 'inlineMedia') await refreshSettings();
  };
}

$('openMedia').onclick = async () => {
  if (!Number.isInteger(tab?.id)) return;
  $('openMedia').disabled = true;
  try {
    const reply = await chrome.runtime.sendMessage({action: 'openMediaWindow', tabId: tab.id});
    if (reply?.ok) window.close();
    else notice(mediaMessage('media_unavailable', language), true);
  } catch { notice(mediaMessage('media_unavailable', language), true); }
  finally { renderSite(); }
};

$('blockSite').onclick = async () => {
  const domain = normalizeDomain(tab?.url || '');
  if (!domain) return;
  const matched = config.blockedDomains.filter(rule => domainMatches(domain, rule));
  config.blockedDomains = matched.length
    ? config.blockedDomains.filter(rule => !matched.includes(rule))
    : [...config.blockedDomains, domain];
  await chrome.storage.local.set({blockedDomains: config.blockedDomains});
  $('domains').value = config.blockedDomains.join('\n');
  await refreshSettings();
};

$('save').onclick = async () => {
  const domains = $('domains').value.split(/\n/).map(value => value.trim()).filter(Boolean).map(normalizeDomain);
  if (domains.some(domain => !domain)) {
    notice(text('invalid'), true);
    return;
  }
  config = {
    ...config,
    backend: $('backend').value,
    captureMode: $('captureMode').value,
    transferPolicy: $('transferPolicy').value,
    minBytes: Math.max(0, Number($('minSize').value) || 0) * 1048576,
    excludedExtensions: $('extensions').value.split(',').map(value => value.trim().toLowerCase().replace(/^\./, '')).filter(Boolean),
    blockedDomains: [...new Set(domains)],
    language: $('language').value,
  };
  await chrome.storage.local.set(config);
  await translate();
  await refreshSettings();
  notice(text('saved'));
};

chrome.runtime.onMessage.addListener(message => {
  if (message.action === 'mediaChanged' && tab && message.tabId === tab.id) {
    renderResources().catch(() => {});
  }
});

start().catch(() => notice(mediaMessage('manager_unavailable', language), true));
