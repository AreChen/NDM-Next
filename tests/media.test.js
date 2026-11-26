const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');

function setup(type, config={}, options={}) {
  const listeners={}, prepared=[],localWrites=[],windowCalls=[],nativeCalls=[];const openWindows=[];
  let pauseTab=false,releaseTab;
  const storage={get:async key=>key===null?{}:{...key,backend:'next',...config},set:async value=>localWrites.push(value),remove:async()=>{}};
  const session=options.session && {setAccessLevel:async value=>{options.session.access=value.accessLevel;},get:async key=>{await options.restoreGate;return structuredClone({[key]:options.session[key]});},set:async value=>{options.session.writes=(options.session.writes || 0)+1;Object.assign(options.session,structuredClone(value));}};
  const chrome={
    storage:{local:storage,session}, downloads:{onCreated:{addListener(){}}},
    webRequest:{onBeforeSendHeaders:{addListener:f=>listeners.headers=f},onHeadersReceived:{addListener:f=>listeners.media=f}},
    tabs:{onUpdated:{addListener:fn=>listeners.navigation=fn},onRemoved:{addListener:fn=>listeners.removed=fn},get:async id=>{if(pauseTab)await new Promise(resolve=>releaseTab=resolve);return {url:`https://page.test/${id}`};}},
    windows:{getAll:async()=>openWindows,create:async options=>{windowCalls.push(options);const window={id:10,tabs:[{url:options.url}]};openWindows.push(window);return window;},update:async(id,options)=>windowCalls.push({id,...options})},
    runtime:{id:'extension',getURL:path=>'chrome-extension://extension/'+path,onInstalled:{addListener(){}},onMessage:{addListener:f=>listeners.message=f},sendNativeMessage:async (_host,msg)=>{
      nativeCalls.push(msg);
      if (msg.type==='browser_context' && options.context) return options.context;
      if(msg.type==='prepare') prepared.push(msg.task);
      return {ok:true,state:msg.type==='prepare'?'prepared':'queued'};
    }}, alarms:{create(){},onAlarm:{addListener(){}}},
  };
  const ctx={chrome,URL,Date:options.now ? class extends Date {static now(){return options.now();}} : Date,crypto:{randomUUID:()=> '00000000-0000-4000-8000-000000000001'},setTimeout,clearTimeout,console,
    importScripts(...files){for(const file of files)vm.runInContext(fs.readFileSync('extension/'+file,'utf8'),ctx)}};
  vm.createContext(ctx); vm.runInContext(fs.readFileSync('extension/background.js','utf8'),ctx);
  const send=(message,sender={id:'extension',url:'chrome-extension://extension/popup.html'})=>new Promise(resolve=>listeners.message(message,sender,resolve));
  return {prepared,localWrites,windowCalls,nativeCalls,send,pauseTab:()=>pauseTab=true,releaseTab:()=>releaseTab(),navigate:()=>listeners.navigation(1,{status:'loading'}),remove:()=>listeners.removed(1),observe:async (tabId,requestId,cookie,extra={})=>{
    await vm.runInContext('captureReady',ctx);
    const url=extra.url || 'https://video.test/playlist?id=42';
    listeners.headers({requestId,tabId,url,frameId:extra.frameId??0,documentId:extra.requestDocumentId??extra.documentId,method:'GET',requestHeaders:[{name:'Cookie',value:cookie},...(extra.requestHeaders||[])]});
    await listeners.media({requestId,tabId,url,frameId:extra.frameId??0,documentId:extra.documentId,method:'GET',statusCode:extra.statusCode??200,responseHeaders:[{name:'Content-Type',value:extra.type || type},{name:'Content-Length',value:'512'},...(extra.responseHeaders||[])]});
  }};
}

test('small images show up independently of the takeover threshold and stream capacity',async()=>{
  const t=setup('image/webp');
  await t.observe(1,'stream','media=1',{url:'https://video.test/watch.m3u8',type:'application/vnd.apple.mpegurl'});
  for(let index=0;index<84;index++){
    await t.observe(1,'image-'+index,'image=1',{url:'https://video.test/'+index+'.webp'});
  }
  const {items}=await t.send({action:'media',tabId:1});
  assert.equal(items.filter(item=>item.type==='image/webp').length,80);
  assert(items.some(item=>item.url.endsWith('.m3u8')));
  const image=items.find(item=>item.type==='image/webp');
  assert.equal((await t.send({action:'send',tabId:1,id:image.id})).ok,true);
  assert.equal(t.prepared[0].expected_size,512);
});

test('media detection can find and send a resource while browser download takeover is disabled',async()=>{
  const t=setup('video/mp4',{enabled:false,media:true});
  await t.observe(1,'media-only','media=1',{url:'https://video.test/video.mp4'});
  const {items,detecting}=await t.send({action:'media',tabId:1});
  assert.equal(detecting,true);
  assert.equal(items.length,1);
  assert.equal((await t.send({action:'send',tabId:1,id:items[0].id})).ok,true);
  assert.equal(t.prepared[0].headers.cookie,'media=1');
});

test('desktop capabilities offer a page action scoped to its original tab',async()=>{
  const t=setup('video/mp4',{},{
    context:{ok:true,desktop_version:'0.4.3',plugins:[
      {id:'yt-dlp',version:'0.2.0',name:{en:'Media Downloader'},page:true},
      {id:'bittorrent',version:'0.1.1',name:{en:'BitTorrent'},page:false},
    ]},
  });
  const data=await t.send({action:'context',tabId:1,page_url:'https://unrelated.test/'});
  assert.equal(data.plugins.length,2);
  assert.equal(data.pages.length,1);
  assert.equal(data.pages[0].version,'0.2.0');
  assert.equal(t.nativeCalls[0].page_url,'https://page.test/1');
  const {id}=data.pages[0];
  const sender={id:'extension',tab:{id:1},frameId:0,url:'https://page.test/1'};
  assert.equal((await t.send({action:'send',id},sender)).error,'invalid_sender');
  assert.equal((await t.send({action:'send',id,tabId:2})).ok,false);
  assert.equal((await t.send({action:'send',id,tabId:1})).ok,true);
  assert.equal(t.prepared[0].url,'https://page.test/1');
  assert.equal(t.prepared[0].plugin,'yt-dlp');
  assert.equal(t.prepared[0].headers,undefined);
  t.navigate();
  assert.equal((await t.send({action:'send',id,tabId:1})).ok,false);
});

for (const type of ['application/vnd.apple.mpegurl','application/dash+xml']) test(`small ${type} manifests remain selectable and use the latest capture from the correct tab`,async()=>{
  const t=setup(type); await new Promise(resolve=>setTimeout(resolve,0));
  await t.observe(1,'1','first=1');
  await t.observe(2,'2','other=2');
  await t.observe(1,'3','latest=3');
  const {items}=await t.send({action:'media',tabId:1});
  assert.equal(items.length,1);
  assert.equal(JSON.stringify(items).includes('latest=3'),false);
  assert.equal((await t.send({action:'send',tabId:1,item:items[0]})).ok,true);
  assert.equal(t.prepared[0].headers.cookie,'latest=3');
  assert.equal(t.prepared[0].expected_size,null);
  assert.equal(t.prepared[0].media_type,type);
  const other=await t.send({action:'media',tabId:2});
  assert.equal((await t.send({action:'send',tabId:2,item:other.items[0]})).ok,true);
  assert.equal(t.prepared[1].headers.cookie,'other=2');
});

test('known video and audio extensions are detected when servers use generic MIME types',async()=>{
  for(const extension of ['webm','mkv','mov','flv','mp3','m4a','ogg','opus','flac','wav']) {
    const t=setup('application/octet-stream',{minBytes:0});
    await t.observe(1,'file','session=1',{url:`https://video.test/file.${extension}?token=1`});
    assert.equal((await t.send({action:'media',tabId:1})).items.length,1,extension);
  }
});

test('resource windows stay bound to their source tab and focus an existing window',async()=>{
  const t=setup('video/mp4',{minBytes:0});
  await t.observe(1,'first','session=1',{frameId:2});await t.observe(2,'other','session=2');
  assert.equal((await t.send({action:'openMediaWindow',tabId:1})).ok,true);
  assert.equal(t.windowCalls[0].url,'chrome-extension://extension/media.html?tab=1');
  assert.equal(t.windowCalls[0].focused,true);
  assert.equal((await t.send({action:'openMediaWindow',tabId:1})).ok,true);
  assert.equal(t.windowCalls.length,2);assert.equal(t.windowCalls[1].id,10);
  const sender={id:'extension',url:'chrome-extension://extension/media.html?tab=1',tab:{id:99}};
  const result=await t.send({action:'media',tabId:1},sender);
  assert.equal(result.items.length,1);assert.equal(result.items[0].frameId,2);
  assert.equal((await t.send({action:'send',tabId:1,id:result.items[0].id},sender)).ok,true);
  assert.equal(t.prepared[0].headers.cookie,'session=1');
  for(const invalid of [{...sender,url:'https://evil.test/media.html?tab=1'},{...sender,url:'chrome-extension://extension/media.html?tab=2'},{...sender,id:'other'}]) {
    assert.equal((await t.send({action:'media',tabId:1},invalid)).ok,false);
  }
  assert.equal((await t.send({action:'openMediaWindow',tabId:1},{id:'extension',url:'https://page.test/1',tab:{id:1}})).ok,false);
  t.navigate();assert.equal((await t.send({action:'media',tabId:1},sender)).items.length,0);
});

test('HTML and arbitrary binary files are not mistaken for media by their suffix',async()=>{
  for(const [type,url] of [['text/html','https://video.test/movie.mp4'],['application/octet-stream','https://video.test/data.bin']]) {
    const t=setup(type,{minBytes:0});await t.observe(1,'file','', {url});
    assert.equal((await t.send({action:'media',tabId:1})).items.length,0);
  }
});

test('range media sends a full GET using its observed credentials and complete length',async()=>{
  const t=setup('video/mp4');
  await t.observe(1,'range','session=1',{requestHeaders:[{name:'Range',value:'bytes=100-611'},{name:'If-Range',value:'"tag"'}],statusCode:206,responseHeaders:[{name:'Content-Range',value:'bytes 100-611/2097152'}]});
  const {items}=await t.send({action:'media',tabId:1});
  assert.equal(items.length,1);
  assert.equal(items[0].size,2097152);
  assert.equal((await t.send({action:'send',tabId:1,id:items[0].id})).ok,true);
  assert.equal(t.prepared[0].expected_size,2097152);
  assert.equal(t.prepared[0].headers.cookie,'session=1');
  assert.equal(t.prepared[0].headers.range,undefined);
});

test('partial and encoded responses never advertise fragment length as complete media size',async()=>{
  for(const extra of [{statusCode:206},{statusCode:206,responseHeaders:[{name:'Content-Range',value:'bytes 0-511/*'}]},{statusCode:206,responseHeaders:[{name:'Content-Range',value:'bytes 0-511/200'}]},{responseHeaders:[{name:'Content-Encoding',value:'gzip'}]}]) {
    const t=setup('video/mp4'); await t.observe(1,'x','a=1',extra);
    const {items}=await t.send({action:'media',tabId:1}); assert.equal(items.length,1); assert.equal(items[0].size,undefined);
  }
});

test('content scripts can only list and send media from their own tab and frame',async()=>{
  const t=setup('video/mp4',{minBytes:0}); await t.observe(1,'x','a=1');
  const sender={id:'extension',tab:{id:1},frameId:0,url:'https://page.test/1'};
  const {items}=await t.send({action:'media'},sender); assert.equal(items.length,1);
  assert.equal((await t.send({action:'send',tabId:2,id:items[0].id},sender)).ok,false);
  assert.equal((await t.send({action:'send',id:items[0].id},{...sender,tab:{id:2}})).ok,false);
  assert.equal((await t.send({action:'send',id:items[0].id},{...sender,frameId:1})).ok,false);
  assert.equal((await t.send({action:'send',id:items[0].id},{...sender,id:'other-extension'})).ok,false);
  assert.equal((await t.send({action:'send',id:items[0].id,item:{url:'https://evil.test',headers:{cookie:'evil'}}},sender)).ok,true);
  assert.equal(t.prepared[0].url,items[0].url);
  assert.equal((await t.send({action:'media',tabId:1},{id:'extension',url:'https://evil.test'})).ok,false);
});

test('unsafe media explains why sending fails and disabled media is removed',async()=>{
  const t=setup('video/mp4',{minBytes:0}); await t.observe(1,'x','a=1',{requestHeaders:[{name:'X-Custom',value:'private'}]});
  const {items}=await t.send({action:'media',tabId:1});
  assert.equal((await t.send({action:'send',tabId:1,id:items[0].id})).error,'unsupported_request');
  const disabled=setup('video/mp4',{media:false}); await disabled.observe(1,'x','a=1');
  assert.equal((await disabled.send({action:'media',tabId:1})).items.length,0);
});

test('navigation cannot repopulate media with a response still resolving its tab',async()=>{
  const t=setup('video/mp4',{minBytes:0});t.pauseTab();
  const pending=t.observe(1,'x','secret=1');
  await new Promise(resolve=>setImmediate(resolve));
  t.navigate();t.releaseTab();await pending;
  assert.equal((await t.send({action:'media',tabId:1})).items.length,0);
});

test('same media URL in separate frames retains independent captures and stable IDs',async()=>{
  const t=setup('video/mp4',{minBytes:0});
  await t.observe(1,'x','frame=0');
  const original=(await t.send({action:'media',tabId:1})).items[0];
  await t.observe(1,'y','frame=1',{frameId:1});
  await t.observe(1,'z','frame=0-new');
  const {items}=await t.send({action:'media',tabId:1});assert.equal(items.length,2);
  assert.equal(items.find(item=>item.frameId===0).id,original.id);
  const sender={id:'extension',tab:{id:1},frameId:1,url:'https://page.test/frame'};
  const frame=(await t.send({action:'media'},sender)).items;
  assert.equal(frame.length,1);assert.equal((await t.send({action:'send',id:frame[0].id},sender)).ok,true);
  assert.equal(t.prepared[0].headers.cookie,'frame=1');
});

test('extension-based manifests are detected and stream fragments do not replace them',async()=>{
  const t=setup('application/octet-stream');
  await t.observe(1,'hls','a=1',{url:'https://video.test/movie.m3u8'});
  await t.observe(1,'dash','a=1',{url:'https://video.test/movie.mpd'});
  const {items}=await t.send({action:'media',tabId:1});assert.equal(items.length,2);assert(items.every(item=>item.size===undefined));
  const stream=setup('video/mp2t');
  await stream.observe(1,'hls','a=1',{url:'https://video.test/movie.m3u8'});
  for(let i=0;i<55;i++)await stream.observe(1,String(i),'a=1',{url:`https://video.test/${i}.ts`});
  const result=await stream.send({action:'media',tabId:1});assert.equal(result.items.length,1);assert.match(result.items[0].url,/m3u8$/);
});

test('media detection hands an extensionless mobile playlist to OnesDM with its request headers',async()=>{
  const t=setup('application/vnd.apple.mpegurl',{enabled:false,media:true});
  await t.observe(1,'mobile','fixture=1',{requestHeaders:[
    {name:'User-Agent',value:'Mobile Safari fixture'},
    {name:'Referer',value:'https://page.test/1'},
    {name:'Accept-Language',value:'zh-CN,zh;q=0.9'},
  ]});
  const {items}=await t.send({action:'media',tabId:1});
  assert.equal(items.length,1);
  assert.equal((await t.send({action:'send',tabId:1,id:items[0].id})).ok,true);
  assert.equal(t.prepared[0].media_type,'application/vnd.apple.mpegurl');
  assert.equal(t.prepared[0].headers['user-agent'],'Mobile Safari fixture');
  assert.equal(t.prepared[0].headers.referer,'https://page.test/1');
  assert.equal(t.prepared[0].headers['accept-language'],'zh-CN,zh;q=0.9');
});

test('blocking the site or turning off media detection hides resources and prevents sending',async()=>{
  const config={minBytes:0};const t=setup('video/mp4',config);await t.observe(1,'x','private=1');
  const {items}=await t.send({action:'media',tabId:1});assert.equal(items.length,1);
  for(const change of [{blockedDomains:['page.test']},{media:false}]) {
    Object.assign(config,{blockedDomains:[],enabled:true,media:true},change);
    assert.equal((await t.send({action:'media',tabId:1})).items.length,0);
    assert.equal((await t.send({action:'send',tabId:1,id:items[0].id})).ok,false);
  }
  assert.equal(t.prepared.length,0);
});

test('iframe document navigation cannot list or send captures from the reused frame ID',async()=>{
  const t=setup('video/mp4',{minBytes:0});
  await t.observe(1,'old','secret=old',{frameId:3,documentId:'old-document'});
  const oldSender={id:'extension',tab:{id:1},frameId:3,documentId:'old-document',url:'https://frame.test/old'};
  const {items}=await t.send({action:'media'},oldSender);assert.equal(items.length,1);
  const newSender={...oldSender,documentId:'new-document',url:'https://frame.test/new'};
  assert.equal((await t.send({action:'media'},newSender)).items.length,0);
  assert.equal((await t.send({action:'send',id:items[0].id,documentId:'old-document'},newSender)).ok,false);
  await t.observe(1,'new','secret=new',{frameId:3,documentId:'new-document'});
  const latest=(await t.send({action:'media'},newSender)).items;
  assert.equal(latest.length,1);assert.equal((await t.send({action:'send',id:latest[0].id},newSender)).ok,true);
  assert.equal(t.prepared[0].headers.cookie,'secret=new');
  assert.equal((await t.send({action:'send',id:latest[0].id},oldSender)).ok,false);
});

test('document IDs captured with request headers scope media even if response metadata omits them',async()=>{
  const t=setup('video/mp4',{minBytes:0});
  await t.observe(1,'old','secret=old',{frameId:3,requestDocumentId:'old-document'});
  const sender={id:'extension',tab:{id:1},frameId:3,documentId:'new-document',url:'https://frame.test/new'};
  assert.equal((await t.send({action:'media'},sender)).items.length,0);
  // Older Chromium builds omit documentId from both APIs; frame checks remain usable.
  const legacy=setup('video/mp4',{minBytes:0});await legacy.observe(1,'x','secret=legacy',{frameId:3});
  assert.equal((await legacy.send({action:'media'},sender)).items.length,1);
});

test('request and response from different known documents cannot share captured credentials',async()=>{
  const t=setup('video/mp4',{minBytes:0});
  await t.observe(1,'x','secret=old',{frameId:3,documentId:'new-document',requestDocumentId:'old-document'});
  const sender={id:'extension',tab:{id:1},frameId:3,documentId:'new-document',url:'https://frame.test/new'};
  const {items}=await t.send({action:'media'},sender);assert.equal(items.length,1);
  assert.equal((await t.send({action:'send',id:items[0].id},sender)).error,'unsupported_request');
  assert.equal(t.prepared.length,0);
});

test('session restores a media ID and its observed credentials after a worker restart',async()=>{
  const session={};const options={session};
  const first=setup('video/mp4',{minBytes:0},options);
  await first.observe(1,'old','private=restart',{frameId:3,documentId:'player'});
  const sender={id:'extension',tab:{id:1},frameId:3,documentId:'player',url:'https://frame.test/player'};
  const {items}=await first.send({action:'media'},sender);
  const restarted=setup('video/mp4',{minBytes:0},options);
  assert.equal((await restarted.send({action:'send',id:items[0].id},sender)).ok,true);
  assert.equal(restarted.prepared[0].headers.cookie,'private=restart');
  assert.equal(session.access,'TRUSTED_CONTEXTS');
  assert.equal(JSON.stringify([...first.localWrites,...restarted.localWrites]).includes('private=restart'),false);
  assert.equal((await restarted.send({action:'send',id:items[0].id},{...sender,documentId:'new-document'})).ok,false);
  await restarted.observe(1,'refreshed','private=refreshed',{frameId:3,documentId:'player'});
  assert.equal((await restarted.send({action:'media'},sender)).items[0].id,items[0].id);
  assert.equal((await restarted.send({action:'send',id:items[0].id},sender)).ok,true);
  assert.equal(restarted.prepared[1].headers.cookie,'private=refreshed');
  await restarted.observe(1,'new','private=new',{url:'https://video.test/new.mp4'});
  const all=await restarted.send({action:'media',tabId:1});
  assert.equal(new Set(all.items.map(item=>item.id)).size,2);
});

test('expired, navigated and removed media are never restored after a worker restart',async()=>{
  for(const change of ['expire','navigate','remove']) {
    const session={};let now=Date.now();const options={session,now:()=>now};
    const first=setup('video/mp4',{minBytes:0},options);
    await first.observe(1,'old','private=stale');
    const {items}=await first.send({action:'media',tabId:1});
    if(change==='expire')now+=1800001;else first[change]();
    // Wait for the event's session mutation, as Chrome does before suspending.
    await first.send({action:'media',tabId:1});
    const restarted=setup('video/mp4',{minBytes:0},options);
    assert.equal((await restarted.send({action:'media',tabId:1})).items.length,0,change);
    assert.equal((await restarted.send({action:'send',tabId:1,id:items[0].id})).ok,false,change);
  }
});

test('navigation or removal while session restoration is pending cannot resurrect old captures',async()=>{
  for(const action of ['navigate','remove']) {
    const session={};const first=setup('video/mp4',{minBytes:0},{session});
    await first.observe(1,'old','private=old');
    await first.send({action:'media',tabId:1});
    let release;const restoreGate=new Promise(resolve=>release=resolve);
    const restarted=setup('video/mp4',{minBytes:0},{session,restoreGate});
    restarted[action]();release();
    assert.equal((await restarted.send({action:'media',tabId:1})).items.length,0,action);
    const again=setup('video/mp4',{minBytes:0},{session});
    assert.equal((await again.send({action:'media',tabId:1})).items.length,0,action);
  }
});

test('session snapshots retain the50-per-tab bound and coalesce burst updates and ignored segments',async()=>{
  const session={};const first=setup('video/mp4',{minBytes:0},{session});
  for(let i=0;i<60;i++)await first.observe(1,String(i),'private=bounded',{url:`https://video.test/${i}.mp4`});
  await new Promise(resolve=>setTimeout(resolve,180));
  assert.equal(session.writes,1);
  const restarted=setup('video/mp4',{minBytes:0},{session});
  assert.equal((await restarted.send({action:'media',tabId:1})).items.length,50);
  const streamSession={};const stream=setup('video/mp2t',{minBytes:0},{session:streamSession});
  await stream.observe(1,'manifest','private=stream',{url:'https://video.test/movie.m3u8'});
  await new Promise(resolve=>setTimeout(resolve,180));
  assert.equal(streamSession.writes,1);
  for(let i=0;i<60;i++)await stream.observe(1,String(i),'private=stream',{url:`https://video.test/${i}.ts`});
  await new Promise(resolve=>setTimeout(resolve,180));
  assert.equal(streamSession.writes,1);
});
