const PREVIEW_MAX_BYTES = 768 * 1024;
const previewCache = new Map();
const previewTargets = new WeakMap();
const previewQueues = Array.from({length: 3}, () => Promise.resolve());
let previewLane = 0;

function previewSource(item) {
  const value = item.poster || (mediaKind(item) === 'image' ? item.url : '');
  if (!value) return null;
  if (value.length < 48000 && /^data:image\/(?:png|webp);base64,[a-z0-9+/=]+$/i.test(value)) return value;
  try {
    const url = new URL(value);
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || value.length > 4096) return null;
    if (!item.poster && [...url.searchParams.keys()].some(key => /token|signature|^sig$|^auth|^x-amz-/i.test(key))) return null;
    return url.href;
  } catch { return null; }
}

async function loadPreview(url) {
  if (url.startsWith('data:')) return url;
  try {
    const response = await fetch(url, {
      credentials: 'omit', cache: 'force-cache', redirect: 'error', referrerPolicy: 'no-referrer',
      signal: AbortSignal.timeout(4000),
    });
    const type = response.headers.get('content-type')?.split(';')[0];
    if (!response.ok || !type?.startsWith('image/') || !response.body
        || Number(response.headers.get('content-length')) > PREVIEW_MAX_BYTES) {
      await response.body?.cancel();
      return null;
    }
    const reader = response.body.getReader();
    const chunks = [];
    let length = 0;
    while (true) {
      const {value, done} = await reader.read();
      if (done) return URL.createObjectURL(new Blob(chunks, {type}));
      length += value.byteLength;
      if (length > PREVIEW_MAX_BYTES) {
        await reader.cancel();
        return null;
      }
      chunks.push(value);
    }
  } catch { return null; }
}

function cachedPreview(url) {
  if (previewCache.has(url)) return previewCache.get(url);
  const pending = loadPreview(url);
  previewCache.set(url, pending);
  if (previewCache.size > 40) {
    const first = previewCache.keys().next().value;
    const evicted = previewCache.get(first);
    previewCache.delete(first);
    evicted.then(value => { if (value?.startsWith('blob:')) URL.revokeObjectURL(value); });
  }
  return pending;
}

const previewObserver = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => {
  for (const {target, isIntersecting} of entries) {
    if (!isIntersecting) continue;
    previewObserver.unobserve(target);
    const lane = previewLane++ % previewQueues.length;
    previewQueues[lane] = previewQueues[lane].then(async () => {
      if (!target.isConnected) return;
      const url = await cachedPreview(previewTargets.get(target));
      if (!target.isConnected || !url) return;
      const image = document.createElement('img');
      image.alt = '';
      image.decoding = 'async';
      image.onload = () => target.classList.add('has-preview');
      image.src = url;
      target.append(image);
    }).catch(() => {});
  }
});

function mediaThumbnail(item, language) {
  const preview = document.createElement('div');
  preview.className = 'thumbnail';
  preview.setAttribute('aria-label', extensionMessage('preview', language, mediaName(item, language)));
  const format = document.createElement('span');
  format.textContent = mediaFormat(item);
  preview.append(format);
  const url = previewSource(item);
  if (url && previewObserver) {
    previewTargets.set(preview, url);
    previewObserver.observe(preview);
  }
  return preview;
}

function resetPreviews() { previewObserver?.disconnect(); }
