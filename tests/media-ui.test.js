const test=require('node:test');
const assert=require('node:assert/strict');
const {mediaSizeLabel,matchMedia,mediaMessage,mediaButtonPosition}=require('../extension/media-ui.js');
const {extensionLocale,extensionMessage}=require('../extension/i18n.js');

test('the language catalogs match and honor the browser locale or an explicit setting',()=>{
  const locales = {
    en: 'en', zh: 'zh_CN', 'zh-Hant': 'zh_TW', ja: 'ja', ko: 'ko',
    es: 'es', 'pt-BR': 'pt_BR', fr: 'fr', de: 'de',
  };
  for (const [browserTag, expected] of Object.entries({
    'zh-CN': 'zh', 'zh-Hans-HK': 'zh', 'zh-HK': 'zh-Hant', 'zh-TW': 'zh-Hant',
    'zh-Hant': 'zh-Hant', pt_PT: 'pt-BR', 'es-MX': 'es', 'fr-CA': 'fr',
    'de-AT': 'de', 'ja-JP': 'ja', 'ko-KR': 'ko', 'uk-UA': 'en',
  })) {
    assert.equal(extensionLocale('auto', browserTag), expected, browserTag);
  }
  assert.equal(extensionLocale('obsolete', 'fr-CA'), 'fr');
  const english = require('../extension/_locales/en/messages.json');
  const keys = Object.keys(english).sort();
  const placeholders = (message) => (message.match(/\$\d+/g) || []).sort();
  for (const [choice, directory] of Object.entries(locales)) {
    assert.equal(extensionLocale(choice, 'zh-CN'), choice);
    const catalog = require(`../extension/_locales/${directory}/messages.json`);
    assert.deepEqual(Object.keys(catalog).sort(), keys, directory);
    for (const key of keys) {
      const message = catalog[key].message;
      assert.ok(message.trim().length > 0, `${choice}/${key}`);
      assert.deepEqual(placeholders(message), placeholders(english[key].message), `${choice}/${key}`);
    }
    assert.ok(extensionMessage('pagePlugin', choice, 'BitTorrent').includes('BitTorrent'));
  }
  assert.equal(extensionMessage('pagePlugin','zh','流媒体下载'),'用 流媒体下载 解析本页');
});

test('built-in media names follow the interface and plugin translations take precedence', () => {
  const {pluginName} = require('../extension/media-library.js');
  const media = {id: 'page-1', plugin: 'yt-dlp', name: {en: 'Media Downloader', zh: '流媒体下载'}};
  assert.equal(pluginName(media, 'ja'), 'メディアダウンローダー');
  assert.equal(pluginName({...media, name: {...media.name, ja: '翻訳済み'}}, 'ja'), '翻訳済み');
  assert.equal(pluginName({id: 'other', name: {en: 'Other engine'}}, 'de'), 'Other engine');
});

test('the media control sits in the upper corner of the player',()=>{
  assert.deepEqual(mediaButtonPosition({top:120,right:650},190,30,1000,800),{left:452,top:126});
  assert.deepEqual(mediaButtonPosition({top:0,right:1200},190,30,1000,800),{left:802,top:8});
  assert.deepEqual(mediaButtonPosition({top:-100,right:90},190,30,1000,800),{left:8,top:8});
});

test('size labels keep unknown and manifest sizes honest in both languages',()=>{
  assert.equal(mediaSizeLabel({size:2097152,url:'https://x/video.mp4'},'en'),'2 MB');
  assert.equal(mediaSizeLabel({size:512,url:'https://x/video.mpd'},'en'),'Size unknown');
  assert.equal(mediaSizeLabel({size:512,type:'application/vnd.apple.mpegurl'},'zh'),'大小未知');
  assert.equal(mediaSizeLabel({size:undefined},'zh'),'大小未知');
  assert.equal(mediaSizeLabel({size:2097152,url:'https://x/segment.m4s'},'en'),'Size unknown');
});
test('inline media associates exact sources and only unambiguous single-player manifests',()=>{
  const direct={id:'direct',url:'https://x/video.mp4?token=1'};
  const manifest={id:'hls',url:'https://x/video.m3u8'};
  assert.equal(matchMedia(['https://x/video.mp4?token=1#t=1'],[direct,manifest],2),direct);
  assert.equal(matchMedia([direct.url,manifest.url],[manifest,direct],1),direct);
  assert.equal(matchMedia(['https://x/video.mp4?token=2'],[direct],1),null);
  assert.equal(matchMedia(['blob:https://x/abc'],[manifest],1),manifest);
  assert.equal(matchMedia(['blob:https://x/abc'],[manifest],2),null);
  assert.equal(matchMedia(['blob:https://x/abc'],[manifest,{...manifest,id:'other',url:'https://x/other.mpd'}],1),null);
  assert.equal(matchMedia([], [manifest],1),null);
});
test('send errors explain unsupported and expired requests in each language',()=>{
  assert.match(mediaMessage('unsupported_request','en'),/cannot be replayed/i);
  assert.match(mediaMessage('capture_expired','zh'),/重新播放/);
  assert.match(mediaMessage('manager_unavailable','en'),/download manager/i);
});
