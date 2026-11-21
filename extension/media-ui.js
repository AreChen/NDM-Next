// Shared by the popup and the isolated-world content script. No page messages
// or page-provided request headers cross the extension boundary.
const RESOURCE_MESSAGES = new Set([
  'download', 'sending', 'sent', 'sentShort', 'unknown', 'manager_unavailable',
  'unsupported_request', 'capture_expired', 'media_unavailable', 'plugin_unavailable', 'hideInline',
]);
const mediaText = typeof module !== 'undefined' ? require('./i18n.js').extensionMessage : extensionMessage;
const mediaManifest = typeof module !== 'undefined' ? require('./rules.js').isMediaManifest : isMediaManifest;
const mediaSegment = typeof module !== 'undefined' ? require('./rules.js').isMediaSegment : isMediaSegment;
function mediaMessage(key,language) {
  return mediaText(RESOURCE_MESSAGES.has(key) ? key : 'manager_unavailable', language);
}
function mediaSizeLabel(item,language) {
  if (mediaManifest(item) || mediaSegment(item) || !Number.isSafeInteger(item.size) || item.size <= 0) return mediaMessage('unknown',language);
  const units=['B','KB','MB','GB','TB'];
  const unit=Math.min(Math.floor(Math.log(item.size)/Math.log(1024)),units.length-1);
  return `${new Intl.NumberFormat(language,{maximumFractionDigits:1}).format(item.size/(1024**unit))} ${units[unit]}`;
}
function matchMedia(sources,items,playerCount) {
  const normalize=value=>{try {const url=new URL(value);url.hash='';return url.href;} catch{return null;}};
  const urls=sources.map(normalize).filter(Boolean);
  // currentSrc precedes src and fallback <source> choices.
  for (const url of urls) {
    const exact=items.find(item=>url === normalize(item.url));
    if (exact) return exact;
  }
  // MSE hides the original URL. Only offer the sole manifest in a frame with
  // one player; ambiguous associations remain available in the popup.
  if (playerCount === 1 && urls.some(url=>url.startsWith('blob:'))) {
    const manifests=items.filter(mediaManifest);
    if (manifests.length === 1) return manifests[0];
  }
  return null;
}
function mediaButtonPosition(rect,width,height,viewportWidth,viewportHeight) {
  const edge=8,gap=6;
  const left=Math.max(edge,Math.min(rect.right-width-edge,viewportWidth-width-edge));
  const top=Math.max(edge,Math.min(rect.top+gap,viewportHeight-height-edge));
  return {left,top};
}
if (typeof module !== 'undefined') {
  module.exports={mediaMessage,mediaSizeLabel,matchMedia,mediaButtonPosition};
}
