(() => {
  const chips = new Map();
  let items = [], enabled = false, dismissed = false, language = 'en', catalogLanguage;
  let frame, fetching = false, refreshAgain = false, host, shadow;

  function raiseHost(force = false) {
    if (!host?.isConnected || typeof host.showPopover !== 'function') return;
    try {
      if (force && host.matches(':popover-open')) host.hidePopover();
      if (!host.matches(':popover-open')) host.showPopover();
    } catch { /* Fixed positioning covers browsers without popovers. */ }
  }

  function ensureHost() {
    if (host?.isConnected) { raiseHost(); return; }
    host = document.createElement('div');
    host.setAttribute('popover', 'manual');
    host.style.cssText = 'all:initial!important;position:fixed!important;inset:0!important;margin:0!important;border:0!important;padding:0!important;width:100vw!important;height:100vh!important;pointer-events:none!important;z-index:2147483647!important;';
    shadow = host.attachShadow({mode: 'closed'});
    const style = document.createElement('style');
    style.textContent = [
      ':host{color-scheme:dark}:host::backdrop{background:transparent!important;pointer-events:none!important}',
      '.pill{position:absolute;display:flex;align-items:center;gap:2px;border:1px solid #b9a7ff56;border-radius:999px;padding:2px;background:#151a2ef2;box-shadow:0 2px 8px #070d2080;font:600 11px/1.15 system-ui,sans-serif;color:#f4efff;opacity:0;pointer-events:none;transition:opacity .14s;max-width:calc(100vw - 16px)}',
      '.pill.shown,.pill:focus-within{opacity:1;pointer-events:auto}.pill[hidden]{display:none!important}',
      'button{font:inherit;color:inherit;cursor:pointer;background:transparent;border:0;border-radius:999px;padding:3px 7px;min-height:18px}',
      'button:hover{background:#a68bff32}button:focus-visible{outline:2px solid #c4b4ff;outline-offset:0}button:disabled{cursor:wait}',
      '.close{padding:2px 4px;line-height:1;color:#c1b7d9;font-size:12px}.action{max-width:220px}',
      '@media(prefers-reduced-motion:reduce){.pill{transition:none}}',
    ].join('');
    shadow.append(style);
    document.documentElement.append(host);
    raiseHost();
  }

  function schedule() {
    if (!frame) frame = requestAnimationFrame(() => { frame = undefined; render(); });
  }

  const resizeObserver = new ResizeObserver(schedule);
  const activity = ['pointerenter', 'pointerleave', 'focusin', 'focusout'];
  function remove(player) {
    const entry = chips.get(player);
    if (!entry) return;
    clearTimeout(entry.noticeTimer);
    entry.pill.remove();
    resizeObserver.unobserve(player);
    for (const event of activity) player.removeEventListener(event, schedule);
    chips.delete(player);
  }

  function playerSources(player) {
    return [player.currentSrc, player.src, ...[...player.querySelectorAll('source[src]')].map(source => source.src)].filter(Boolean);
  }

  function playerPosters(items) {
    if (!Array.isArray(items)) return {};
    const players = [...document.querySelectorAll('video,audio')].slice(0, 50);
    const posters = {};
    for (const player of players) {
      const resource = matchMedia(playerSources(player), items, players.length);
      if (!resource) continue;
      const poster = player.poster || (players.length === 1
        ? document.querySelector('meta[property="og:image"]')?.content : undefined);
      if (poster) {
        try { posters[resource.id] = new URL(poster, document.baseURI).href; } catch {}
      } else if (player.readyState >= 2 && player.videoWidth && player.videoHeight) {
        try {
          const canvas = document.createElement('canvas');
          canvas.width = 160;
          canvas.height = Math.max(1, Math.min(160, Math.round(160 * player.videoHeight / player.videoWidth)));
          canvas.getContext('2d').drawImage(player, 0, 0, canvas.width, canvas.height);
          posters[resource.id] = canvas.toDataURL('image/webp', 0.65);
        } catch { /* The page may not allow cross-origin frame reads. */ }
      }
    }
    return posters;
  }

  function addChip(player, id) {
    ensureHost();
    const pill = document.createElement('div');
    const button = document.createElement('button');
    const close = document.createElement('button');
    pill.className = 'pill';
    button.className = 'action';
    button.type = close.type = 'button';
    close.className = 'close';
    close.textContent = '×';
    const entry = {id, pill, button, close};
    chips.set(player, entry);
    resizeObserver.observe(player);

    for (const event of activity) {
      player.addEventListener(event, schedule);
      pill.addEventListener(event, schedule);
    }
    pill.addEventListener('pointerdown', event => event.stopPropagation());
    close.addEventListener('click', event => {
      event.preventDefault();
      event.stopPropagation();
      dismissed = true;
      schedule();
      chrome.runtime.sendMessage({action:'dismissInline'}).catch(() => {});
    });
    button.addEventListener('click', async event => {
      event.preventDefault();
      event.stopPropagation();
      button.disabled = true;
      clearTimeout(entry.noticeTimer);
      button.textContent = mediaMessage('sending', language);
      schedule();
      const response = await chrome.runtime.sendMessage({action: 'send', id}).catch(() => null);
      if (chips.get(player) !== entry) return;
      button.disabled = false;
      button.textContent = mediaMessage(response?.ok ? 'sentShort' : response?.error, language);
      button.setAttribute('aria-label', button.textContent);
      entry.noticeTimer = setTimeout(() => {
        entry.noticeTimer = undefined;
        schedule();
      }, response?.ok ? 1200 : 4000);
      schedule();
    });
    pill.append(button, close);
    shadow.append(pill);
    return entry;
  }

  function render() {
    const players = enabled && !dismissed
      ? [...document.querySelectorAll('video,audio')].filter(player => player.isConnected) : [];
    for (const player of chips.keys()) if (!players.includes(player)) remove(player);
    for (const player of players) {
      const item = matchMedia(playerSources(player), items, players.length);
      if (!item) { remove(player); continue; }
      let entry = chips.get(player);
      if (entry && entry.id !== item.id) { remove(player); entry = undefined; }
      entry ||= addChip(player, item.id);
      const {pill, button, close} = entry;
      if (!button.disabled && !entry.noticeTimer) {
        button.textContent = '↓ NDM';
        button.title = mediaMessage('download', language) + ' · ' + mediaSizeLabel(item, language);
        button.setAttribute('aria-label', 'NDM Next · ' + button.title);
      }
      close.title = mediaMessage('hideInline', language);
      close.setAttribute('aria-label', close.title);
      const rect = player.getBoundingClientRect();
      pill.hidden = !(rect.width >= 100 && rect.height >= 24 && rect.bottom > 0 && rect.right > 0
        && rect.top < innerHeight && rect.left < innerWidth && getComputedStyle(player).visibility !== 'hidden');
      pill.classList.toggle('shown', player.matches(':hover') || player.matches(':focus-within')
        || pill.matches(':hover') || button.disabled || !!entry.noticeTimer);
      const position = mediaButtonPosition(rect, pill.offsetWidth, pill.offsetHeight, innerWidth, innerHeight);
      pill.style.left = position.left + 'px';
      pill.style.top = position.top + 'px';
    }
    if (!chips.size) { host?.remove(); host = undefined; }
    else raiseHost();
  }

  async function refresh() {
    if (fetching) { refreshAgain = true; return; }
    fetching = true;
    try {
      const result = await chrome.runtime.sendMessage({action: 'media', locale: catalogLanguage});
      items = result?.items || [];
      enabled = result?.inlineMedia === true;
      language = extensionLocale(result?.language, chrome.i18n.getUILanguage());
      if (result?.labels) {
        installCatalog(language, result.labels);
        catalogLanguage = language;
      }
    } catch { items = []; enabled = false; }
    finally {
      fetching = false;
      schedule();
      if (refreshAgain) { refreshAgain = false; refresh(); }
    }
  }

  new MutationObserver(schedule).observe(document.documentElement, {childList: true, subtree: true, attributes: true, attributeFilter: ['src']});
  for (const event of ['loadedmetadata', 'loadstart', 'emptied', 'play']) document.addEventListener(event, schedule, true);
  document.addEventListener('fullscreenchange', () => { raiseHost(true); schedule(); });
  document.addEventListener('toggle', event => {
    if (event.target !== host && event.newState === 'open') { raiseHost(true); schedule(); }
  }, true);
  addEventListener('scroll', () => { if (chips.size) schedule(); }, {passive: true, capture: true});
  addEventListener('resize', schedule, {passive: true});
  chrome.runtime.onMessage.addListener((message, sender, reply) => {
    if (message.action === 'mediaChanged') refresh();
    if (message.action === 'mediaPosters' && sender.id === chrome.runtime.id) reply(playerPosters(message.items));
  });
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area !== 'local') return;
    if (changes.inlineMedia?.newValue === true) dismissed = false;
    if (Object.keys(changes).some(key => Object.hasOwn(DEFAULTS, key))) refresh();
  });
  refresh();
})();
