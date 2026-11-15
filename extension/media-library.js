// Metadata only: this window never plays media or makes media URL requests.
const libraryKind=typeof module !== 'undefined' ? require('./rules.js').mediaKind : mediaKind;
function mediaName(item) {
  try {
    const name=new URL(item.url).pathname.split('/').pop();
    return decodeURIComponent(name || '') || 'Media';
  } catch {return 'Media';}
}
function mediaHost(item) {try {return new URL(item.url).host;} catch {return '';}}
function mediaFormat(item) {
  const kind=libraryKind(item);
  if (kind === 'hls' || kind === 'dash') return kind.toUpperCase();
  const suffix=mediaName(item).match(/\.([a-z0-9]{2,5})$/i)?.[1];
  const knownSuffix=libraryKind({url:item.url,type:''}) ? suffix : null;
  return (knownSuffix || String(item.type || '').split(';')[0].split('/')[1] || kind || 'Media').toUpperCase();
}
function mediaSearchText(value) {return String(value).normalize('NFKC').toLowerCase().normalize('NFD').replace(/\p{M}/gu,'');}
function filterMedia(items,query,kind) {
  const words=mediaSearchText(query).trim().split(/\s+/u).filter(Boolean);
  return items.filter(item=>{
    const itemKind=libraryKind(item);
    if (kind !== 'all' && kind !== itemKind) return false;
    const haystack=mediaSearchText(`${mediaName(item)} ${mediaHost(item)} ${mediaFormat(item)}`);
    return words.every(word=>haystack.includes(word));
  });
}
if (typeof module !== 'undefined') module.exports={mediaName,mediaHost,mediaFormat,mediaSearchText,filterMedia};
