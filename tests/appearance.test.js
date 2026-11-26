const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

const script = fs.readFileSync('extension/appearance.js', 'utf8');
const light = {mode: 'light', primary: 0x367aed, secondary: 0x1ebbcf, surface: 0xf6f8ff};
const dark = {mode: 'dark', primary: 0xc7c7c7, secondary: 0x909090, surface: 0x222222};
const systemIcon = '(prefers-color-scheme: light)';

function signal() {
  const listeners = [];
  return {
    addListener: listener => listeners.push(listener),
    emit: (...args) => listeners.forEach(listener => listener(...args)),
  };
}

function browser(initialSettings = {}, available = true) {
  const settings = {...initialSettings};
  const changed = signal();
  const storage = {
    onChanged: changed,
    local: {
      get: async defaults => ({...defaults, ...settings}),
      set: async values => {
        Object.assign(settings, values);
        changed.emit(Object.fromEntries(Object.entries(values).map(([key, value]) => [key, {newValue: value}])), 'local');
      },
    },
  };
  function open(view) {
    const html = fs.readFileSync(`extension/${view}.html`, 'utf8');
    assert.match(html, /<script src="appearance\.js"><\/script>/);
    const dataset = {}, colors = new Map(), ports = [];
    const icons = [...html.matchAll(/data-light-icon/g)].map(() => ({media: systemIcon}));
    const docEvents = new Map(), windowEvents = new Map(), timers = new Map();
    const choice = view === 'popup' ? {disabled: true, addEventListener: (name, handler) => { choice[name] = handler; }} : null;
    const document = {
      documentElement: {dataset, style: {setProperty: colors.set.bind(colors), removeProperty: colors.delete.bind(colors)}},
      getElementById: id => id === 'appearance' ? choice : null,
      querySelectorAll: () => icons,
      addEventListener: docEvents.set.bind(docEvents),
      hidden: false,
    };
    const runtime = {connectNative: name => {
      assert.equal(name, 'com.onesdm.host');
      if (!available) throw new Error('Native host missing');
      const port = {
        onMessage: signal(), onDisconnect: signal(), requests: [], closed: false,
        postMessage(value) { this.requests.push(value.type); },
        disconnect() { this.closed = true; },
        theme(appearance) { this.onMessage.emit({ok: true, appearance}); },
      };
      ports.push(port);
      return port;
    }};
    let timerId = 0;
    vm.runInNewContext(script, {
      chrome: {storage, runtime}, document, window: {addEventListener: windowEvents.set.bind(windowEvents)},
      setTimeout: fn => { timers.set(++timerId, fn); return timerId; },
      clearTimeout: id => timers.delete(id),
    });
    return {dataset, colors, icons, ports, choice, timers, close: () => windowEvents.get('pagehide')(),
      focus: () => windowEvents.get('focus')(), setHidden: hidden => {
        document.hidden = hidden;
        docEvents.get('visibilitychange')();
      }};
  }
  return {open, storage};
}

const ready = () => new Promise(resolve => setImmediate(resolve));

test('popup and media share the default desktop appearance and receive changes without polling', async () => {
  const {open} = browser();
  const popup = open('popup'), media = open('media');
  for (const view of [popup, media]) assert.equal(view.dataset.appearance, undefined);
  await ready();
  assert.equal(popup.choice.value, 'desktop');
  assert.equal(popup.choice.disabled, false);
  for (const view of [popup, media]) {
    const [port] = view.ports;
    port.theme(light);
    assert.equal(view.dataset.appearance, 'light');
    assert.equal(view.colors.get('--primary'), '#367aed');
    assert(view.icons.every(icon => icon.media === 'all'));
    port.theme(dark);
    assert.equal(view.dataset.appearance, 'dark');
    assert.equal(view.colors.get('--surface'), '#222222');
    assert(view.icons.every(icon => icon.media === 'not all'));
    assert.equal(view.timers.size, 0);
    view.focus();
    assert.deepEqual(port.requests, ['watch_appearance']);
    assert.equal(view.ports.length, 1);
    view.close();
    assert.equal(port.closed, true);
  }

  const reopened = open('popup');
  await ready();
  reopened.ports[0].theme(dark);
  assert.equal(reopened.dataset.appearance, 'dark');
  reopened.setHidden(true);
  assert.equal(reopened.ports[0].closed, true);
  reopened.setHidden(false);
  reopened.ports[1].theme(light);
  assert.equal(reopened.dataset.appearance, 'light');
});

test('selecting system changes both views immediately and leaves native messaging idle', async () => {
  const {open} = browser();
  const popup = open('popup'), media = open('media');
  await ready();
  popup.ports[0].theme(dark);
  media.ports[0].theme(dark);
  popup.choice.value = 'system';
  popup.choice.change();
  const reopened = open('popup');
  await ready();
  for (const view of [popup, media, reopened]) {
    assert.equal(view.dataset.appearance, undefined);
    assert.equal(view.colors.size, 0);
    assert(view.icons.every(icon => icon.media === systemIcon));
    view.ports[0]?.theme(light); // A late message from a closed connection is ignored.
    assert.equal(view.dataset.appearance, undefined);
  }
  assert.equal(reopened.choice.value, 'system');
  assert.equal(reopened.ports.length, 0);
  reopened.choice.value = 'desktop';
  reopened.choice.change();
  assert.deepEqual(reopened.ports[0].requests, ['watch_appearance']);
});

test('offline, legacy and malformed native replies restore system appearance', async () => {
  const failures = [
    port => port.onMessage.emit({ok: false, error: 'appearance_unavailable'}),
    port => port.onMessage.emit({ok: false, error: 'invalid_request'}),
    port => port.theme({...light, primary: 0x1000000}),
    port => port.theme({...dark, secondary: '#fff; color: transparent'}),
    port => port.theme({...dark, mode: 'system'}),
    port => port.onDisconnect.emit(),
  ];
  for (const fail of failures) {
    const view = browser().open('media');
    await ready();
    view.ports[0].theme(light);
    fail(view.ports[0]);
    assert.equal(view.dataset.appearance, undefined);
    assert.equal(view.colors.size, 0);
    assert.equal(view.timers.size, 0);
    assert(view.icons.every(icon => icon.media === systemIcon));
    view.focus();
    view.ports[1].theme(dark);
    assert.equal(view.dataset.appearance, 'dark');
  }
  const missing = browser({}, false).open('popup');
  const stalled = browser().open('media');
  await ready();
  assert.equal(missing.dataset.appearance, undefined);
  for (const timeout of stalled.timers.values()) timeout();
  assert.equal(stalled.dataset.appearance, undefined);
  assert.equal(stalled.ports[0].closed, true);
  assert.equal(stalled.timers.size, 0);
});

test('custom surface, accent text and button ink remain readable in both modes', async () => {
  const view = browser().open('media');
  await ready();
  const luminance = rgb => [16, 8, 0].reduce((result, shift, index) => {
    const channel = (rgb >> shift & 255) / 255;
    const linear = channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
    return result + [0.2126, 0.7152, 0.0722][index] * linear;
  }, 0);
  const ratio = (a, b) => {
    const [min, max] = [luminance(a), luminance(b)].sort((first, second) => first - second);
    return (max + 0.05) / (min + 0.05);
  };
  const rgb = key => parseInt(view.colors.get(`--${key}`).slice(1), 16);
  for (const mode of ['light', 'dark']) {
    for (const surface of [0, 0xffffff, 0xffbcb9, 0x001290]) {
      for (const primary of [0, 0x777777, 0xff0000, 0xffffff]) {
        view.ports[0].theme({mode, primary, secondary: 0xb58dee, surface});
        assert.equal(rgb('primary'), primary);
        assert(ratio(rgb('primary'), rgb('primary-ink')) >= 4.5);
        for (const key of ['page', 'surface', 'tint', 'input']) {
          assert(ratio(rgb('accent'), rgb(key)) >= 4.5);
          assert(ratio(mode === 'light' ? 0x526580 : 0xa0aec8, rgb(key)) >= 4.5);
        }
      }
    }
  }
});
