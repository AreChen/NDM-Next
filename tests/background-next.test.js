const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');

function setup({capture=true,method='GET',url='https://files.test/file.zip',finalUrl=url,finalCapture,size=2097152,mime='application/zip',referrer='https://page.test/',initiator,requestType,fetchSite,config={},prepareOk=true,backendAbsent=false,sharedSession={},delayInitialSettings=false}={}) {
  const listeners={}, calls=[], saved={backend:'next',transferPolicy:'immediate',...config};
  let preparedTask;
  if (backendAbsent) delete saved.backend;
  let state='in_progress', error;
  let releaseInitialSettings;
  let firstSettings=true;
  const storage={
    get:async key=>{
      if (firstSettings && key && typeof key==='object') {
        firstSettings=false;
        if (delayInitialSettings) await new Promise(resolve=>{releaseInitialSettings=resolve});
      }
      return key===null?{...saved}:typeof key==='string'?{[key]:saved[key]}:{...key,...saved};
    },
    set:async value=>Object.assign(saved,value), remove:async key=>delete saved[key],
  };
  const chrome={
    storage:{local:storage,session:{get:async key=>({[key]:sharedSession[key]}),set:async value=>Object.assign(sharedSession,value),remove:async key=>delete sharedSession[key]}}, cookies:{getAll:async()=>[]},
    downloads:{onCreated:{addListener:f=>listeners.download=f},
      search:async()=>[{id:5,state,error,totalBytes:size,url,finalUrl}],
      cancel:async()=>{calls.push('cancel');state='interrupted';error='USER_CANCELED'},
      erase:async()=>calls.push('erase')},
    webRequest:{onBeforeSendHeaders:{addListener:f=>listeners.headers=f},onHeadersReceived:{addListener:f=>listeners.media=f}},
    tabs:{onUpdated:{addListener(){}},onRemoved:{addListener(){}},get:async()=>({url:referrer})},
    runtime:{onInstalled:{addListener:f=>listeners.installed=f},onMessage:{addListener(){}},
      sendNativeMessage:async (_name,message)=>{ calls.push(message.type); if(message.type==='prepare'){ preparedTask=message.task;assert.equal(message.task.headers.cookie,'session=private');return {ok:prepareOk,state:'prepared'} } if(message.type==='commit')return {ok:true,state:'queued'};return {ok:true} }},
    alarms:{create(){},onAlarm:{addListener(){}}},
    notifications:{create:async()=>{calls.push('notification')},clear:async()=>{},onButtonClicked:{addListener:f=>listeners.approve=f},onClosed:{addListener(){}}},
    i18n:{getUILanguage:()=> 'en-US'},
  };
  const ctx={chrome,URL,AbortSignal,crypto:{randomUUID:()=> '00000000-0000-4000-8000-000000000001'},setTimeout,clearTimeout,console,fetch:async()=>({ok:true,headers:{get:()=>null}}),importScripts(...files){for(const file of files)vm.runInContext(fs.readFileSync('extension/'+file,'utf8'),ctx)}};
  vm.createContext(ctx);vm.runInContext(fs.readFileSync('extension/background.js','utf8'),ctx);
  return {calls,saved,task:()=>preparedTask,install:reason=>listeners.installed({reason}),approve:()=>listeners.approve('ndm-next-confirm:5',0),runCold:async()=>{
    listeners.headers({requestId:'abc',method,url,tabId:1,requestHeaders:[{name:'Cookie',value:'session=private'}]});
    const result=listeners.download({id:5,url,finalUrl,referrer,totalBytes:size,mime});
    releaseInitialSettings();
    return result;
  },run:async()=>{
    await new Promise(resolve=>setTimeout(resolve,0));
    const requestHeaders=[{name:'Cookie',value:'session=private'}];
    if(fetchSite)requestHeaders.push({name:'Sec-Fetch-Site',value:fetchSite});
    if(capture)listeners.headers({requestId:'abc',method,url,tabId:1,initiator,type:requestType,requestHeaders});
    if(finalCapture)listeners.headers({requestId:'abc',method,url:finalUrl,tabId:1,initiator,type:requestType,requestHeaders:finalCapture==='unsafe'?[{name:'X-Private',value:'secret'}]:requestHeaders});
    return listeners.download({id:5,url,finalUrl,referrer,totalBytes:size,mime});
  }};
}

test('new backend stores request before cancel and commits after cancel',async()=>{
  const t=setup();await t.run();assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
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
test('a download that wakes the extension is captured while settings load',async()=>{
  const t=setup({delayInitialSettings:true});
  await t.runCold();assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
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
test('new installs default to NDM Next while updates retain the legacy backend',async()=>{
  const fresh=setup({capture:false,backendAbsent:true});await fresh.install('install');assert.equal(fresh.saved.backend,'next');
  const upgraded=setup({capture:false,backendAbsent:true});await upgraded.install('update');assert.equal(upgraded.saved.backend,'neatdm');
  const chosen=setup({capture:false,config:{backend:'next'}});await chosen.install('update');assert.equal(chosen.saved.backend,'next');
});
