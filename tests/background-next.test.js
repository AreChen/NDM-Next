const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');

function setup({capture=true,method='GET',referrer='https://page.test/',config={},prepareOk=true,backendAbsent=false}={}) {
  const listeners={}, calls=[], saved={backend:'next',...config};
  if (backendAbsent) delete saved.backend;
  let state='in_progress', error;
  const storage={
    get:async key=>key===null?{...saved}:typeof key==='string'?{[key]:saved[key]}:{...key,...saved},
    set:async value=>Object.assign(saved,value), remove:async key=>delete saved[key],
  };
  const chrome={
    storage:{local:storage}, cookies:{getAll:async()=>[]},
    downloads:{onCreated:{addListener:f=>listeners.download=f},
      search:async()=>[{id:5,state,error,totalBytes:2097152,finalUrl:'https://files.test/file.zip'}],
      cancel:async()=>{calls.push('cancel');state='interrupted';error='USER_CANCELED'},
      erase:async()=>calls.push('erase')},
    webRequest:{onBeforeSendHeaders:{addListener:f=>listeners.headers=f},onHeadersReceived:{addListener:f=>listeners.media=f}},
    tabs:{onUpdated:{addListener(){}},onRemoved:{addListener(){}},get:async()=>({url:referrer})},
    runtime:{onInstalled:{addListener:f=>listeners.installed=f},onMessage:{addListener(){}},
      sendNativeMessage:async (_name,message)=>{ calls.push(message.type); if(message.type==='prepare'){ assert.equal(message.task.headers.cookie,'session=private');return {ok:prepareOk,state:'prepared'} } if(message.type==='commit')return {ok:true,state:'queued'};return {ok:true} }},
    alarms:{create(){},onAlarm:{addListener(){}}},
    notifications:{create:async()=>{calls.push('notification')},clear:async()=>{},onButtonClicked:{addListener:f=>listeners.approve=f},onClosed:{addListener(){}}},
    i18n:{getUILanguage:()=> 'en-US'},
  };
  const ctx={chrome,URL,AbortSignal,crypto:{randomUUID:()=> '00000000-0000-4000-8000-000000000001'},setTimeout,clearTimeout,console,fetch:async()=>({ok:true,headers:{get:()=>null}}),importScripts(...files){for(const file of files)vm.runInContext(fs.readFileSync('extension/'+file,'utf8'),ctx)}};
  vm.createContext(ctx);vm.runInContext(fs.readFileSync('extension/background.js','utf8'),ctx);
  return {calls,saved,install:reason=>listeners.installed({reason}),approve:()=>listeners.approve('ndm-next-confirm:5',0),run:async()=>{
    await new Promise(resolve=>setTimeout(resolve,0));
    if(capture)listeners.headers({requestId:'abc',method,url:'https://files.test/file.zip',tabId:1,requestHeaders:[{name:'Cookie',value:'session=private'}]});
    return listeners.download({id:5,url:'https://files.test/file.zip',referrer,totalBytes:2097152,mime:'application/zip'});
  }};
}

test('new backend stores request before cancel and commits after cancel',async()=>{
  const t=setup();await t.run();assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
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
test('new installs default to NDM Next while updates retain the legacy backend',async()=>{
  const fresh=setup({capture:false,backendAbsent:true});await fresh.install('install');assert.equal(fresh.saved.backend,'next');
  const upgraded=setup({capture:false,backendAbsent:true});await upgraded.install('update');assert.equal(upgraded.saved.backend,'neatdm');
  const chosen=setup({capture:false,config:{backend:'next'}});await chosen.install('update');assert.equal(chosen.saved.backend,'next');
});
