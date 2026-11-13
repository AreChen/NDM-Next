const DEFAULTS = { enabled: true, minBytes: 1048576, excludedExtensions: ['data','woff','woff2','ttf','otf','eot'], blockedDomains: [], language: 'auto', media: true, backend:'neatdm', captureMode:'automatic', transferPolicy:'safe' };
function isHls(item) {
  const type = String(item.type || '').split(';')[0].trim().toLowerCase();
  if (['application/vnd.apple.mpegurl','application/x-mpegurl','audio/mpegurl','audio/x-mpegurl'].includes(type)) return true;
  try { return new URL(item.url).pathname.toLowerCase().endsWith('.m3u8'); } catch { return false; }
}
function normalizeDomain(value) { try { return new URL(value.includes('://') ? value : `https://${value}`).hostname.toLowerCase().replace(/\.$/, ''); } catch { return ''; } }
function domainMatches(host, blocked) { host = normalizeDomain(host); blocked = normalizeDomain(blocked); return !!host && !!blocked && (host === blocked || host.endsWith('.' + blocked)); }
function shouldSkip(item, settings = DEFAULTS) {
  if (!settings.enabled) return true;
  let url; try { url = new URL(item.url); } catch { return true; }
  if (!['http:', 'https:'].includes(url.protocol)) return true;
  if ((settings.blockedDomains || []).some(d => domainMatches(url.hostname, d) || domainMatches(item.pageUrl || '', d))) return true;
  const ext = decodeURIComponent(url.pathname).split('/').pop().split('.').pop().toLowerCase();
  if ((settings.excludedExtensions || []).map(s => s.toLowerCase().replace(/^\./, '')).includes(ext)) return true;
  return Number.isFinite(item.size) && item.size >= 0 && item.size < (settings.minBytes || 0);
}
function encodeTask(item) {
  const safe = x => { x = String(x ?? ''); if (/[\r\n]/.test(x)) throw new Error('Invalid header value'); return x; };
  let result = `1:GET\r\n2:${safe(item.url)}\r\n6:normal\r\n`;
  if (item.pageUrl && /^https?:/.test(item.pageUrl)) { const page = new URL(item.pageUrl); page.hash = ''; result += `Origin: ${safe(page.origin)}\r\nReferer: ${safe(page.href)}\r\n`; }
  if (item.cookie) result += `Cookie: ${safe(item.cookie)}\r\n`;
  if (Number.isFinite(item.size) && item.size >= 0) result += `7:${item.size}\r\n`;
  if (item.type) result += `8:${safe(item.type)}\r\n`;
  return result;
}
if (typeof module !== 'undefined') module.exports = { DEFAULTS, normalizeDomain, domainMatches, shouldSkip, encodeTask, isHls };
