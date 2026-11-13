const test=require('node:test'); const assert=require('node:assert/strict'); const vm=require('node:vm'); const fs=require('node:fs');
function setup({online=true, size=2097152, url='https://files.test/archive.zip'}={}) {
 const listeners={}; const calls=[];
 const chrome={storage:{local:{get:async x=>x}},cookies:{getAll:async()=>[]},downloads:{onCreated:{addListener:f=>listeners.download=f},search:async()=>[{id:1,state:'in_progress',totalBytes:size}],cancel:async id=>calls.push(['cancel',id]),erase:async x=>calls.push(['erase',x.id])},webRequest:{onHeadersReceived:{addListener:f=>listeners.media=f}},tabs:{onUpdated:{addListener(){}},onRemoved:{addListener(){}},get:async()=>({url:'https://site.test/'})},runtime:{onMessage:{addListener(){}}}};
 class Socket { constructor(){setTimeout(()=>online?this.onopen?.():this.onerror?.(),0)} send(s){calls.push(['send',s])} close(){} }
 const ctx={chrome,WebSocket:Socket,URL,AbortSignal,setTimeout,clearTimeout,console,fetch:async()=>({ok:true,headers:{get:()=>null}}),importScripts(){vm.runInContext(fs.readFileSync('extension/rules.js','utf8'),ctx)}};
 vm.createContext(ctx); vm.runInContext(fs.readFileSync('extension/background.js','utf8'),ctx);
 return {run:()=>listeners.download({id:1,url,referrer:'https://site.test/',totalBytes:size,mime:'application/zip'}),calls};
}
test('sends eligible download before cancelling browser download',async()=>{const t=setup();await t.run();assert.deepEqual(t.calls.map(x=>x[0]),['send','cancel','erase'])});
test('connection failure leaves browser download alone',async()=>{const t=setup({online:false});await t.run();assert.deepEqual(t.calls,[])});
test('small downloads remain in browser',async()=>{const t=setup({size:100});await t.run();assert.deepEqual(t.calls,[])});
