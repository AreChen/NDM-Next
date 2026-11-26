const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');

function setup({capture=true,method='GET',url='https://files.test/file.zip',finalUrl=url,finalCapture,size=2097152,mime='application/zip',referrer='https://page.test/',initiator,requestType,fetchSite,config={},prepareOk=true,backendAbsent=false,sharedSession={},delayInitialSettings=false,failInitialSettings=false,nativeError}={}) {
  const listeners={}, calls=[], nativeAttempts=[], saved={backend:'next',transferPolicy:'immediate',...config};
  let preparedTask;
  if (backendAbsent) delete saved.backend;
  let state='in_progress', error;
  let releaseInitialSettings;
  let firstSettings=true;
  let failNextRead=false;
  const storage={
    get:async key=>{
      const value=key===null?{...saved}:typeof key==='string'?{[key]:saved[key]}:{...key,...saved};
      if (firstSettings && key && typeof key==='object') {
        firstSettings=false;
        if (delayInitialSettings) await new Promise(resolve=>{releaseInitialSettings=resolve});
        if (failInitialSettings) throw new Error('Storage unavailable');
      }
      if (key && typeof key==='object' && failNextRead) {
        failNextRead=false;
        throw new Error('Storage unavailable');
      }
      return value;
    },
    set:async value=>Object.assign(saved,value), remove:async key=>delete saved[key],
  };
  const chrome={
    storage:{local:storage,onChanged:{addListener:f=>listeners.settingsChanged=f},session:{get:async key=>({[key]:sharedSession[key]}),set:async value=>Object.assign(sharedSession,value),remove:async key=>delete sharedSession[key]}}, cookies:{getAll:async()=>[]},
    downloads:{onCreated:{addListener:f=>listeners.download=f},
      search:async()=>[{id:5,state,error,totalBytes:size,url,finalUrl}],
      cancel:async()=>{calls.push('cancel');state='interrupted';error='USER_CANCELED'},
      erase:async()=>calls.push('erase')},
    webRequest:{onBeforeSendHeaders:{addListener:f=>listeners.headers=f},onHeadersReceived:{addListener:f=>listeners.media=f}},
    tabs:{onUpdated:{addListener(){}},onRemoved:{addListener(){}},get:async()=>({url:referrer})},
    runtime:{onInstalled:{addListener:f=>listeners.installed=f},onMessage:{addListener(){}},
      sendNativeMessage:async (name,message)=>{
        nativeAttempts.push([name,message.type]);
        if (name==='com.onesdm.host' && nativeError) throw new Error(nativeError);
        calls.push(message.type);
        if(message.type==='prepare'){ preparedTask=message.task;assert.equal(message.task.headers.cookie,'session=private');return {ok:prepareOk,state:'prepared'} }
        if(message.type==='commit')return {ok:true,state:'queued'};
        return {ok:true};
      }},
    alarms:{create(){},onAlarm:{addListener(){}}},
    notifications:{create:async()=>{calls.push('notification')},clear:async()=>{},onButtonClicked:{addListener:f=>listeners.approve=f},onClosed:{addListener(){}}},
    i18n:{getUILanguage:()=> 'en-US'},
  };
  const ctx={chrome,URL,AbortSignal,crypto:{randomUUID:()=> '00000000-0000-4000-8000-000000000001'},setTimeout,clearTimeout,console,fetch:async()=>({ok:true,headers:{get:()=>null}}),importScripts(...files){for(const file of files)vm.runInContext(fs.readFileSync('extension/'+file,'utf8'),ctx)}};
  vm.createContext(ctx);vm.runInContext(fs.readFileSync('extension/background.js','utf8'),ctx);
  const download=()=>listeners.download({id:5,url,finalUrl,referrer,totalBytes:size,mime});
  const ready=()=>vm.runInContext('captureReady',ctx);
  const request=(details={})=>listeners.headers({requestId:'abc',method,url,tabId:1,requestHeaders:[{name:'Cookie',value:'session=private'}],...details});
  return {calls,saved,nativeAttempts,task:()=>preparedTask,install:reason=>listeners.installed({reason}),approve:()=>listeners.approve('ndm-next-confirm:5',0),
    request,download,ready,releaseInitialSettings:()=>releaseInitialSettings(),
    failSettingsRead:()=>failNextRead=true,
    state:()=>JSON.parse(vm.runInContext('JSON.stringify({size:requests.size, expires:!!expiryTimer, download:!!requests.get("abc")?.headers, media:!!requests.get("abc")?.mediaCapture})',ctx)),
    change:(values,fail=false)=>{
      Object.assign(saved,values);
      failNextRead=fail;
      listeners.settingsChanged(Object.fromEntries(Object.entries(values).map(([name,newValue])=>[name,{newValue}])), 'local');
      return ready();
    },
    run:async()=>{
    await ready();
    const requestHeaders=[{name:'Cookie',value:'session=private'}];
    if(fetchSite)requestHeaders.push({name:'Sec-Fetch-Site',value:fetchSite});
    if(capture)listeners.headers({requestId:'abc',method,url,tabId:1,initiator,type:requestType,requestHeaders});
    if(finalCapture)listeners.headers({requestId:'abc',method,url:finalUrl,tabId:1,initiator,type:requestType,requestHeaders:finalCapture==='unsafe'?[{name:'X-Private',value:'secret'}]:requestHeaders});
    return download();
  }};
}

test('new backend stores request before cancel and commits after cancel',async()=>{
  const t=setup();await t.run();assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
  assert.deepEqual(t.nativeAttempts,[['com.onesdm.host','prepare'],['com.onesdm.host','commit']]);
});
test('small DASH/HLS manifests hand off without using the XML or playlist length as media size',async()=>{
  for(const item of [
    {url:'https://files.test/video.mpd',mime:'application/dash+xml'},
    {url:'https://files.test/manifest',mime:'application/dash+xml; charset=UTF-8'},
    {url:'https://files.test/video.m3u8',mime:'application/vnd.apple.mpegurl'},
  ]) {
    const t=setup({...item,size:512});await t.run();
    assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
    assert.equal(t.task().expected_size,null);
    assert.equal(t.task().media_type,item.mime);
  }
});
test('unknown download lengths are captured without passing a fabricated expected size',async()=>{
  for (const size of [-1,0]) {
    const t=setup({size});await t.run();
    assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
    assert.equal(t.task().expected_size,null);
  }
});
test('GitHub archive redirects can use the captured original request when the final request is unavailable',async()=>{
  const url='https://github.com/owner/repo/archive/refs/heads/main.zip';
  const finalUrl='https://codeload.github.com/owner/repo/zip/refs/heads/main';
  const t=setup({url,finalUrl,size:-1,referrer:'https://github.com/owner/repo'});await t.run();
  assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
  assert.equal(t.task().url,url);
  assert.equal(t.task().expected_size,null);
});
test('redirect handoff respects unsafe target requests, blocked sites, and known small files',async()=>{
  const url='https://github.com/owner/repo/archive/refs/heads/main.zip';
  const finalUrl='https://codeload.github.com/owner/repo/zip/refs/heads/main';
  for (const options of [
    {finalCapture:'unsafe'},
    {config:{blockedDomains:['codeload.github.com']}},
    {size:42},
  ]) {
    const t=setup({url,finalUrl,size:-1,...options});await t.run();
    assert.deepEqual(t.calls,[]);
  }
  const observed=setup({url,finalUrl,size:-1,finalCapture:'safe'});await observed.run();
  assert.equal(observed.task().url,finalUrl);
});
test('signed download links follow the configured immediate handoff',async()=>{
  const t=setup({url:'https://files.test/file.zip?X-Amz-Signature=signed'});
  await t.run();assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
});
test('a no-referrer redirect uses its observed initiator for the domain blacklist',async()=>{
  const redirect={url:'https://files.test/file.zip',finalUrl:'https://cdn.test/object',finalCapture:'safe',referrer:'',initiator:'https://page.test'};
  const allowed=setup({...redirect,config:{blockedDomains:['unrelated.test']}});
  await allowed.run();
  assert.deepEqual(allowed.calls,['prepare','cancel','commit','erase']);
  assert.equal(allowed.task().source_url,null);
  for (const domain of ['page.test','files.test','cdn.test']) {
    const blocked=setup({...redirect,config:{blockedDomains:[domain]}});
    await blocked.run();
    assert.deepEqual(blocked.calls,[]);
  }
});
test('a browser-initiated direct navigation still checks both download domains',async()=>{
  const redirect={url:'https://files.test/file.zip',finalUrl:'https://cdn.test/object',finalCapture:'safe',referrer:'',requestType:'main_frame',fetchSite:'none'};
  const allowed=setup({...redirect,config:{blockedDomains:['unrelated.test']}});
  await allowed.run();
  assert.deepEqual(allowed.calls,['prepare','cancel','commit','erase']);
  for (const options of [
    {config:{blockedDomains:['files.test']}},
    {config:{blockedDomains:['cdn.test']}},
    {initiator:'null'},
    {requestType:'sub_frame'},
    {fetchSite:'cross-site'},
  ]) {
    const blocked=setup({...redirect,config:{blockedDomains:['unrelated.test']},...options});
    await blocked.run();
    assert.deepEqual(blocked.calls,[]);
  }
});
test('headers seen during a cold start are never used for takeover after settings load',async()=>{
  const t=setup({delayInitialSettings:true});
  t.request();
  assert.deepEqual(t.state(), {size:0,expires:false,download:false,media:false});
  const cold=t.download();
  t.releaseInitialSettings();
  await cold;
  assert.deepEqual(t.calls,[]);
  await t.run();
  assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
});
test('settings failure and both features disabled leave request capture off',async()=>{
  for(const options of [{failInitialSettings:true},{config:{enabled:false,media:false}}]){
    const t=setup(options);
    await t.run();
    assert.deepEqual(t.state(), {size:0,expires:false,download:false,media:false});
    assert.deepEqual(t.calls,[]);
  }
});
test('either feature can record its own request data without enabling the other',async()=>{
  const downloadOnly=setup({config:{enabled:true,media:false}});
  await downloadOnly.run();
  assert.deepEqual(downloadOnly.calls,['prepare','cancel','commit','erase']);
  assert.deepEqual(downloadOnly.state(), {size:1,expires:true,download:true,media:false});

  const mediaOnly=setup({config:{enabled:false,media:true}});
  await mediaOnly.run();
  assert.deepEqual(mediaOnly.calls,[]);
  assert.deepEqual(mediaOnly.state(), {size:1,expires:true,download:false,media:true});
});
test('disabling both features or failing a settings refresh discards the cache and its timer',async()=>{
  for (const fail of [false,true]) {
    const t=setup({capture:false});
    await t.ready();
    t.request();
    assert.equal(t.state().size,1);
    const refreshed=t.change(fail?{media:false}:{enabled:false,media:false},fail);
    t.request();
    assert.deepEqual(t.state(), {size:0,expires:false,download:false,media:false});
    await refreshed;
    t.request();
    assert.deepEqual(t.state(), {size:0,expires:false,download:false,media:false});
    await t.change({enabled:true,media:false});
    await t.download();
    assert.deepEqual(t.calls,[]);
    t.request();
    await t.download();
    assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
  }
});
test('a superseded startup read cannot re-enable capture after both features are disabled',async()=>{
  const t=setup({delayInitialSettings:true});
  const startup=t.ready();
  await t.change({enabled:false,media:false});
  t.releaseInitialSettings();
  await startup;
  t.request();
  assert.deepEqual(t.state(), {size:0,expires:false,download:false,media:false});
});
test('a failed settings read during operation closes capture until a later successful read',async()=>{
  const t=setup();
  await t.ready();
  t.request();
  assert.equal(t.state().size,1);
  t.failSettingsRead();
  await t.download();
  t.request();
  assert.deepEqual(t.state(), {size:0,expires:false,download:false,media:false});
  assert.deepEqual(t.calls,[]);
  await t.download(); // Fresh settings can open the gate, but cannot recover dropped headers.
  assert.deepEqual(t.calls,[]);
  await t.run();
  assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
});
test('new extensions support a desktop that only registers the old native host',async()=>{
  const t=setup({nativeError:'Specified native messaging host not found.'});
  await t.run();
  assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
  assert.deepEqual(t.nativeAttempts,[
    ['com.onesdm.host','prepare'],['com.ndmnext.host','prepare'],
    ['com.onesdm.host','commit'],['com.ndmnext.host','commit'],
  ]);
});
test('a communication failure does not retry a native operation under the legacy name',async()=>{
  const t=setup({nativeError:'Error when communicating with the native messaging host.'});
  await t.run();
  assert(t.nativeAttempts.some(([,type])=>type==='prepare'));
  assert(t.nativeAttempts.every(([name])=>name==='com.onesdm.host'));
  assert.deepEqual(t.calls,[]);
});
test('missing capture, non-GET and missing page with a blacklist stay in browser',async()=>{
  for(const options of [{capture:false},{method:'POST'},{referrer:'',config:{blockedDomains:['example.com']}}]){
    const t=setup(options);await t.run();assert.deepEqual(t.calls,[]);
  }
});
test('host preparation failure never cancels the browser download',async()=>{
  const t=setup({prepareOk:false});await t.run();assert.deepEqual(t.calls,['prepare','abort']);
});
test('per-download confirmation leaves the browser alone until approved',async()=>{
  const t=setup({config:{captureMode:'confirm'}});await t.run();
  assert.deepEqual(t.calls,['notification']);
  t.approve();
  await new Promise(resolve=>setTimeout(resolve,10));
  assert.deepEqual(t.calls,['notification','prepare','cancel','commit','erase']);
});
test('confirmation survives an extension service-worker restart without writing credentials to local storage',async()=>{
  const sharedSession={};
  const first=setup({config:{captureMode:'confirm'},sharedSession}); await first.run();
  assert(!JSON.stringify(first.saved).includes('session=private'));
  const restarted=setup({config:{captureMode:'confirm'},sharedSession});
  restarted.approve();
  await new Promise(resolve=>setTimeout(resolve,10));
  assert.deepEqual(restarted.calls,['prepare','cancel','commit','erase']);
  assert.deepEqual(sharedSession,{});
});
test('new installs default to OnesDM while updates retain the legacy backend',async()=>{
  const fresh=setup({capture:false,backendAbsent:true});await fresh.install('install');assert.equal(fresh.saved.backend,'next');
  const upgraded=setup({capture:false,backendAbsent:true});await upgraded.install('update');assert.equal(upgraded.saved.backend,'neatdm');
  const chosen=setup({capture:false,config:{backend:'next'}});await chosen.install('update');assert.equal(chosen.saved.backend,'next');
});
