(() => {
  const root = document.documentElement;
  const chooser = document.getElementById('appearance');
  const lightIcons = document.querySelectorAll('[data-light-icon]');
  const colorKeys = ['page', 'surface', 'input', 'line', 'accent', 'tint', 'primary', 'primary-ink'];
  let source, port, handshake;

  function mix(from, to, weight) {
    const channel = shift => Math.round(((from >> shift) & 255) * (1 - weight) + ((to >> shift) & 255) * weight);
    return (channel(16) << 16) | (channel(8) << 8) | channel(0);
  }

  function luminance(color) {
    const linear = shift => {
      const value = ((color >> shift) & 255) / 255;
      return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0);
  }

  function contrast(a, b) {
    const first = luminance(a), second = luminance(b);
    return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
  }

  function readable(color, fallback, backgrounds) {
    for (let weight = 0; weight < 1; weight += 0.125) {
      const shade = mix(color, fallback, weight);
      if (backgrounds.every(background => contrast(shade, background) >= 4.5)) return shade;
    }
    return fallback;
  }

  function palette(appearance) {
    const dark = appearance.mode === 'dark';
    const muted = dark ? 0xa0aec8 : 0x526580;
    const neutral = dark ? 0x090909 : 0xe4e4e4;
    const surface = readable(appearance.surface, dark ? 0x151b2c : 0xfcfdff, [muted]);
    const tint = readable(mix(surface, appearance.secondary, 0.16), surface, [muted]);
    const page = mix(surface, neutral, 0.45);
    const input = mix(surface, neutral, 0.32);
    const accent = readable(appearance.primary, dark ? 0xffffff : 0, [surface, tint, page, input]);
    const primary = appearance.primary;
    const ink = contrast(primary, 0) >= contrast(primary, 0xffffff) ? 0 : 0xffffff;
    return [page, surface, input, mix(surface, muted, 0.22), accent, tint, primary, ink];
  }

  function valid(appearance) {
    return (appearance?.mode === 'light' || appearance?.mode === 'dark') &&
      ['primary', 'secondary', 'surface'].every(key =>
        Number.isInteger(appearance[key]) && appearance[key] >= 0 && appearance[key] <= 0xffffff);
  }

  function render(appearance) {
    if (appearance) {
      root.dataset.appearance = appearance.mode;
      const colors = palette(appearance);
      colorKeys.forEach((key, index) => root.style.setProperty(`--${key}`, `#${colors[index].toString(16).padStart(6, '0')}`));
    } else {
      delete root.dataset.appearance;
      colorKeys.forEach(key => root.style.removeProperty(`--${key}`));
    }
    for (const icon of lightIcons) {
      icon.media = appearance ? (appearance.mode === 'light' ? 'all' : 'not all') : '(prefers-color-scheme: light)';
    }
  }

  function disconnect() {
    const previous = port;
    port = undefined;
    clearTimeout(handshake);
    previous?.disconnect();
    render(null);
  }

  function connect() {
    if (source !== 'desktop' || document.hidden || port) return;
    try {
      const connection = chrome.runtime.connectNative('com.onesdm.host');
      port = connection;
      connection.onDisconnect.addListener(() => {
        // Consume the Chromium error for missing or unauthorized native hosts.
        void chrome.runtime.lastError;
        if (port !== connection) return;
        port = undefined;
        disconnect();
      });
      connection.onMessage.addListener(reply => {
        if (port !== connection) return;
        if (reply?.ok !== true || !valid(reply.appearance)) {
          disconnect();
          return;
        }
        clearTimeout(handshake);
        render(reply.appearance);
      });
      // A stalled handshake reverts to system appearance once. Updates are pushed.
      handshake = setTimeout(disconnect, 2500);
      connection.postMessage({type: 'watch_appearance'});
    } catch {
      disconnect();
    }
  }

  function useSource(value) {
    source = value === 'system' ? 'system' : 'desktop';
    if (chooser) {
      chooser.value = source;
      chooser.disabled = false;
    }
    if (source === 'desktop') connect();
    else disconnect();
  }

  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === 'local' && Object.hasOwn(changes, 'appearanceSource')) useSource(changes.appearanceSource.newValue);
  });
  chooser?.addEventListener('change', () => {
    chrome.storage.local.set({appearanceSource: chooser.value}).catch(() => { chooser.value = source; });
  });
  chrome.storage.local.get({appearanceSource: 'desktop'}).then(settings => {
    if (source === undefined) useSource(settings.appearanceSource);
  }, () => {
    if (source === undefined) useSource('system');
  });

  document.addEventListener('visibilitychange', () => document.hidden ? disconnect() : connect());
  window.addEventListener('pagehide', disconnect);
  window.addEventListener('pageshow', connect);
  window.addEventListener('focus', connect);
})();
