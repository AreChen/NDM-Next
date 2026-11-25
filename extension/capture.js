const REPLAY_HEADERS = new Set(['accept','accept-language','authorization','cookie','origin','referer','user-agent']);
const IGNORED_HEADERS = new Set(['host','connection','accept-encoding','upgrade-insecure-requests','priority','cache-control','pragma','dnt']);

function captureRequest(details, {media = false} = {}) {
  if (details.method !== 'GET' || !Array.isArray(details.requestHeaders)) return null;
  let url;
  try { url = new URL(details.url); } catch { return null; }
  if (!['http:','https:'].includes(url.protocol) || url.username || url.password) return null;
  // Signed and token-bearing links may be single-use. Immediate takeover can
  // fail for those links; the user can choose verified takeover instead.
  const headers = {};
  const seen = new Set();
  let bytes = 0;
  for (const header of details.requestHeaders) {
    const name = String(header.name || '').toLowerCase();
    if (media && (name === 'range' || name === 'if-range')) {
      if (seen.has(name) || typeof header.value !== 'string' || !header.value || /[\r\n\0]/.test(header.value)) return null;
      seen.add(name);
      if (name === 'range' && !/^bytes=(?:\d+-\d*|-\d+)$/.test(header.value)) return null;
      // Replay the complete media resource; ordinary downloads retain the strict policy.
      continue;
    }
    if (REPLAY_HEADERS.has(name)) {
      if (typeof header.value !== 'string' || /[\r\n\0]/.test(header.value) || Object.hasOwn(headers,name)) return null;
      bytes += name.length + header.value.length;
      if (bytes > 65536) return null;
      headers[name] = header.value;
    } else if (!IGNORED_HEADERS.has(name) && !name.startsWith('sec-')) {
      // Range/conditional and custom authentication headers cannot be safely replayed.
      return null;
    }
  }
  // A top-level navigation marked "none" came from browser UI, without an
  // initiating site. An opaque initiator ("null") does not qualify.
  const directNavigation = details.type === 'main_frame' && details.tabId >= 0 && details.initiator === undefined
    && details.requestHeaders.some(header => String(header.name).toLowerCase() === 'sec-fetch-site' && header.value === 'none');
  return {requestId:details.requestId,url:details.url,tabId:details.tabId,frameId:details.frameId ?? 0,documentId:details.documentId,initiator:details.initiator,directNavigation,headers};
}

function findCapture(captures,url,now=Date.now()) {
  const matches = captures.filter(item => item.url === url && now-item.when <= 120000 && now >= item.when);
  return matches.length === 1 && matches[0].headers ? matches[0] : null;
}

if (typeof module !== 'undefined') module.exports = {captureRequest,findCapture};
