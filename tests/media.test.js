const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');

function setup() {
  const listeners={}, prepared=[];
  const storage={get:async key=>key===null?{}:{...key,backend:'next'},set:async()=>{},remove:async()=>{}};
  const chrome={
    storage:{local:storage}, downloads:{onCreated:{addListener(){}}},
    webRequest:{onBeforeSendHeaders:{addListener:f=>listeners.headers=f},onHeadersReceived:{addListener:f=>listeners.media=f}},
    tabs:{onUpdated:{addListener(){}},onRemoved:{addListener(){}},get:async id=>({url:`https://page.test/${id}`})},
    runtime:{onInstalled:{addListener(){}},onMessage:{addListener:f=>listeners.message=f},sendNativeMessage:async (_host,msg)=>{
      if(msg.type==='prepare') prepared.push(msg.task);
      return {ok:true,state:msg.type==='prepare'?'prepared':'queued'};
    }}, alarms:{create(){},onAlarm:{addListener(){}}},
  };
  const ctx={chrome,URL,crypto:{randomUUID:()=> '00000000-0000-4000-8000-000000000001'},setTimeout,clearTimeout,console,
    importScripts(...files){for(const file of files)vm.runInContext(fs.readFileSync('extension/'+file,'utf8'),ctx)}};
  vm.createContext(ctx); vm.runInContext(fs.readFileSync('extension/background.js','utf8'),ctx);
  const send=message=>new Promise(resolve=>listeners.message(message,{},resolve));
  return {prepared,send,observe:async (tabId,requestId,cookie)=>{
    const url='https://video.test/playlist?id=42';
    listeners.headers({requestId,tabId,url,method:'GET',requestHeaders:[{name:'Cookie',value:cookie}]});
    await listeners.media({requestId,tabId,url,method:'GET',responseHeaders:[{name:'Content-Type',value:'application/vnd.apple.mpegurl'},{name:'Content-Length',value:'512'}]});
  }};
}

test('small HLS playlists remain selectable and use the latest capture from the correct tab',async()=>{
  const t=setup(); await new Promise(resolve=>setTimeout(resolve,0));
  await t.observe(1,'1','first=1');
  await t.observe(2,'2','other=2');
  await t.observe(1,'3','latest=3');
  const {items}=await t.send({action:'media',tabId:1});
  assert.equal(items.length,1);
  assert.equal(JSON.stringify(items).includes('latest=3'),false);
  assert.equal((await t.send({action:'send',tabId:1,item:items[0]})).ok,true);
  assert.equal(t.prepared[0].headers.cookie,'latest=3');
  assert.equal(t.prepared[0].expected_size,null);
  assert.equal(t.prepared[0].media_type,'application/vnd.apple.mpegurl');
  const other=await t.send({action:'media',tabId:2});
  assert.equal((await t.send({action:'send',tabId:2,item:other.items[0]})).ok,true);
  assert.equal(t.prepared[1].headers.cookie,'other=2');
});
