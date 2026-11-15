const test=require('node:test');
const assert=require('node:assert/strict');
const {mediaSizeLabel,matchMedia,mediaMessage}=require('../extension/media-ui.js');

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
