(() => {
  const $ = id => document.getElementById(id);
  const raw = new URL(location.href).searchParams.get('tab');
  const tabId = raw !== null && /^\d+$/.test(raw) ? Number(raw) : NaN;
  const selected = new Set();
  const results = new Map();
  const posters = new Map();
  const pageSize = 24;
  let items = [], pages = [], language = 'en', sourceClosed = false, detecting = true;
  let limit = pageSize, busy = false, refreshing = false, queued = false, queuedForce = false;
  let lastPosterScan = 0, pageUrl, noticeTimer;
  const text = (key, ...values) => extensionMessage(key, language, ...values);
  const filtered = () => filterMedia(items, $('search').value, $('kind').value, language);
  const visible = () => filtered().slice(0, limit);

  function notice(message, error = false) {
    clearTimeout(noticeTimer);
    $('feedback').textContent = message;
    $('feedback').classList.toggle('error', error);
    noticeTimer = setTimeout(() => { $('feedback').textContent = ''; }, error ? 6500 : 2500);
  }

  function renderSelection(shown) {
    const selectedCount = shown.filter(item => selected.has(item.id)).length;
    $('count').textContent = text('count', shown.length, items.length, selectedCount);
    $('selectAll').checked = shown.length > 0 && selectedCount === shown.length;
    $('selectAll').indeterminate = selectedCount > 0 && selectedCount < shown.length;
    $('selectAll').disabled = busy || shown.length === 0;
    $('download').disabled = busy || selectedCount === 0;
    $('download').textContent = text('downloadSelected') + (selectedCount ? ' (' + selectedCount + ')' : '');
  }

  function render() {
    const matching = filtered();
    const shown = matching.slice(0, limit);
    renderSelection(shown);
    $('empty').hidden = shown.length > 0;
    $('empty').textContent = text(sourceClosed ? 'closed' : !detecting ? 'pausedDetection' : items.length ? 'noMatch' : 'empty');
    $('showMore').hidden = matching.length <= shown.length;
    resetPreviews();
    const list = $('list');
    list.replaceChildren();
    for (const item of shown) {
      const status = results.get(item.id);
      const card = document.createElement('article');
      card.className = selected.has(item.id) ? 'surface resource selected' : 'surface resource';
      const check = document.createElement('input');
      check.type = 'checkbox';
      check.checked = selected.has(item.id);
      check.disabled = busy;
      check.setAttribute('aria-label', mediaName(item, language));
      check.onchange = () => {
        if (check.checked) selected.add(item.id); else selected.delete(item.id);
        card.classList.toggle('selected', check.checked);
        renderSelection(shown);
      };
      const info = document.createElement('div');
      info.className = 'resource-info';
      const name = document.createElement('div');
      name.className = 'resource-name';
      const title = document.createElement('strong');
      title.textContent = mediaName(item, language);
      title.title = title.textContent;
      const format = document.createElement('span');
      format.className = 'badge';
      format.textContent = mediaFormat(item);
      name.append(title, format);
      const detail = document.createElement('div');
      detail.className = 'resource-detail';
      detail.textContent = mediaHost(item) + ' · ' + mediaSizeLabel(item, language);
      const ops = document.createElement('div');
      ops.className = 'resource-ops';
      const button = document.createElement('button');
      button.className = 'quiet';
      button.type = 'button';
      button.disabled = busy || !!status?.sent;
      button.textContent = text(status?.pending ? 'sending' : status?.sent ? 'sentShort' : 'download');
      button.onclick = () => downloadItems([item.id]);
      ops.append(button);
      if (status?.error) {
        const error = document.createElement('span');
        error.className = 'message error';
        error.textContent = mediaMessage(status.error, language);
        ops.append(error);
      }
      info.append(name, detail, ops);
      card.append(check, mediaThumbnail({...item, poster: posters.get(item.id)}, language), info);
      list.append(card);
    }
  }

  async function downloadItems(ids) {
    if (busy || ids.length === 0) return;
    busy = true;
    let sent = 0;
    try {
      for (const id of ids) {
        if (!items.some(item => item.id === id)) continue;
        results.set(id, {pending: true});
        render();
        const result = await chrome.runtime.sendMessage({action: 'send', tabId, id}).catch(() => null);
        if (result?.ok) {
          sent++;
          selected.delete(id);
          results.set(id, {sent: true});
        } else {
          results.set(id, {error: result?.error || 'manager_unavailable'});
        }
      }
      notice(text('done', sent, ids.length), sent !== ids.length);
    } finally { busy = false; render(); }
  }

  async function sendPage(id, button) {
    button.disabled = true;
    const result = await chrome.runtime.sendMessage({action: 'send', tabId, id}).catch(() => null);
    notice(mediaMessage(result?.ok ? 'sent' : result?.error, language), !result?.ok);
    button.disabled = false;
  }

  async function refresh(force = false) {
    if (refreshing) {
      queued = true;
      queuedForce ||= force;
      return;
    }
    refreshing = true;
    $('refresh').disabled = true;
    try {
      const checkPosters = force || Date.now() - lastPosterScan > 2500;
      const reply = await chrome.runtime.sendMessage({action: 'media', tabId, posters: checkPosters});
      if (!Array.isArray(reply?.items)) throw new Error('resources unavailable');
      if (checkPosters) lastPosterScan = Date.now();
      language = extensionLocale(reply.language, chrome.i18n.getUILanguage());
      await loadCatalog(language);
      localizeDocument(language);
      document.title = 'NDM Next · ' + text('title');
      sourceClosed = reply.source === null;
      detecting = reply.detecting !== false;
      items = sourceClosed ? [] : reply.items.sort((a, b) => Number(mediaKind(a) === 'image') - Number(mediaKind(b) === 'image'));
      const ids = new Set(items.map(item => item.id));
      for (const item of items) if (item.poster) posters.set(item.id, item.poster);
      for (const tracked of [selected, results, posters]) {
        for (const id of tracked.keys()) if (!ids.has(id)) tracked.delete(id);
      }
      $('source').textContent = sourceClosed ? text('closed') : reply.source?.title || mediaHost({url: reply.source?.url});
      if (force || pageUrl !== reply.source?.url) {
        pageUrl = reply.source?.url;
        const context = await chrome.runtime.sendMessage({action: 'context', tabId, refresh: force}).catch(() => null);
        pages = sourceClosed ? [] : context?.pages || [];
      }
      renderPageActions($('pageActions'), pages, language, sendPage);
      render();
    } catch { notice(mediaMessage('manager_unavailable', language), true); }
    finally {
      refreshing = false;
      $('refresh').disabled = false;
      if (queued) {
        const forceNext = queuedForce;
        queued = queuedForce = false;
        refresh(forceNext);
      }
    }
  }

  function filterChanged() { limit = pageSize; render(); }
  $('search').oninput = filterChanged;
  $('kind').onchange = filterChanged;
  $('refresh').onclick = () => refresh(true);
  $('showMore').onclick = () => { limit += pageSize; render(); };
  $('selectAll').onchange = () => {
    for (const item of visible()) {
      if ($('selectAll').checked) selected.add(item.id);
      else selected.delete(item.id);
    }
    render();
  };
  $('download').onclick = () => downloadItems(visible().filter(item => selected.has(item.id)).map(item => item.id));
  chrome.runtime.onMessage.addListener(message => {
    if (message.action === 'mediaChanged' && message.tabId === tabId) refresh();
  });
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === 'local' && Object.keys(changes).some(key => Object.hasOwn(DEFAULTS, key))) refresh(true);
  });
  if (Number.isSafeInteger(tabId) && tabId >= 0) refresh(true);
  else {
    sourceClosed = true;
    loadCatalog(language).then(() => { localizeDocument(language); render(); });
  }
})();
