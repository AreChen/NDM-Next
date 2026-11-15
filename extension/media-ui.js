// Shared by the popup and the isolated-world content script. No page messages
// or page-provided request headers cross the extension boundary.
const MEDIA_TEXT = {
  en:{download:'Download',sending:'Sending…',sent:'Sent to download manager',unknown:'Size unknown',manager_unavailable:'Unable to send. Check the download manager connection.',unsupported_request:'This media request cannot be replayed safely. Try a direct download link.',capture_expired:'Media details expired. Play the media again and retry.',media_unavailable:'Media is no longer available. Play it again and retry.'},
  zh:{download:'下载',sending:'正在发送…',sent:'已发送至下载管理器',unknown:'大小未知',manager_unavailable:'发送失败，请检查下载管理器连接。',unsupported_request:'此媒体请求无法安全重放，请尝试直接下载链接。',capture_expired:'媒体信息已过期，请重新播放后重试。',media_unavailable:'媒体已不可用，请重新播放后重试。'},
};
const mediaManifest = typeof module !== 'undefined' ? require('./rules.js').isMediaManifest : isMediaManifest;
const mediaSegment = typeof module !== 'undefined' ? require('./rules.js').isMediaSegment : isMediaSegment;
function mediaLanguage(language,uiLanguage='en') { return language === 'zh' || (language === 'auto' && uiLanguage.toLowerCase().startsWith('zh')) ? 'zh' : 'en'; }
function mediaMessage(key,language) { return MEDIA_TEXT[language]?.[key] || MEDIA_TEXT[language]?.manager_unavailable || MEDIA_TEXT.en.manager_unavailable; }
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
  const above=rect.top-height-gap;
  const top=Math.max(edge,Math.min(above>=edge ? above : rect.top+gap,viewportHeight-height-edge));
  return {left,top};
}
if (typeof module !== 'undefined') {
  module.exports={mediaLanguage,mediaMessage,mediaSizeLabel,matchMedia,mediaButtonPosition};
}
