const libraryKind=typeof module !== 'undefined' ? require('./rules.js').mediaKind : mediaKind;
const libraryText=typeof module !== 'undefined' ? require('./i18n.js').extensionMessage : extensionMessage;
function mediaName(item, language = 'en') {
  try {
    const name=new URL(item.url).pathname.split('/').pop();
    return decodeURIComponent(name || '') || libraryText('media', language);
  } catch {return libraryText('media', language);}
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
function filterMedia(items,query,kind,language = 'en') {
  const words=mediaSearchText(query).trim().split(/\s+/u).filter(Boolean);
  return items.filter(item=>{
    const itemKind=libraryKind(item);
    if (kind !== 'all' && kind !== itemKind) return false;
    const haystack=mediaSearchText(`${mediaName(item, language)} ${mediaHost(item)} ${mediaFormat(item)}`);
    return words.every(word=>haystack.includes(word));
  });
}

function pluginName(plugin, language) {
  const name = plugin.name || {};
  if (name[language]) return name[language];
  if ((plugin.plugin || plugin.id) === 'yt-dlp' && name.en === 'Media Downloader') {
    return libraryText('mediaDownloader', language);
  }
  return (language === 'zh-Hant' && name.zh) || name.en || plugin.plugin || plugin.id;
}

function renderPageActions(container, pages, language, send) {
  container.hidden = pages.length === 0;
  container.replaceChildren();
  for (const page of pages) {
    const button = document.createElement('button');
    button.type = 'button';
    const label = document.createElement('span');
    label.className = 'page-action-label';
    label.textContent = extensionMessage('pagePlugin', language, pluginName(page, language));
    label.title = label.textContent;
    const version = document.createElement('span');
    version.className = 'badge page-action-version';
    version.textContent = `v${page.version}`;
    button.append(label, ' ', version);
    button.onclick = () => send(page.id, button);
    container.append(button);
  }
}
if (typeof module !== 'undefined') module.exports={mediaName,mediaHost,mediaFormat,mediaSearchText,filterMedia,pluginName};
