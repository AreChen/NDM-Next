const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const {mediaName,mediaFormat,filterMedia}=require('../extension/media-library.js');

test('resource search handles decoded multilingual names and generic MIME formats',()=>{
  const items=[{id:'zh',url:'https://cdn.test/%E9%9F%B3%E4%B9%90%EF%BC%A1.mp3',type:'application/octet-stream'},
    {id:'fr',url:'https://media.test/Caf%C3%A9.webm',type:'video/webm'},
    {id:'hls',url:'https://cdn.test/index.m3u8',type:'application/octet-stream'}];
  assert.equal(mediaName(items[0]),'音乐Ａ.mp3');assert.equal(mediaFormat(items[2]),'HLS');
  assert.equal(mediaFormat({url:'https://cdn.test/download.php?id=42',type:'video/mp4'}),'MP4');
  assert.deepEqual(filterMedia(items,'音乐a CDN','audio').map(item=>item.id),['zh']);
  assert.deepEqual(filterMedia(items,'CAFE webm','video').map(item=>item.id),['fr']);
  assert.deepEqual(filterMedia(items,'','hls').map(item=>item.id),['hls']);
  assert.equal(filterMedia(items,'music','all').length,0);
  assert.equal(mediaName({url:'https://cdn.test/%invalid'}),'Media');
});

function setup({language='en',failure}={}) {
  const elements=new Map(),sent=[],listeners={},timers=new Map();let timerId=0;
  class Element {
    constructor(){this.children=[];this.value='';this.textContent='';this.classes=new Set();this.classList={toggle:(key,on)=>on?this.classes.add(key):this.classes.delete(key)};}
    append(...children){this.children.push(...children);}
    replaceChildren(...children){this.children=children;}
    setAttribute(key,value){this[key]=value;}
  }
  const get=id=>{if(!elements.has(id))elements.set(id,new Element());return elements.get(id);};
  get('kind').value='all';
  let result={items:[{id:'video',url:'https://cdn.test/movie.mp4',type:'video/mp4',size:2097152},{id:'stream',url:'https://cdn.test/index.m3u8',type:'application/vnd.apple.mpegurl',size:500}],language,source:{title:'Source page',url:'https://page.test'}};
  const chrome={i18n:{getUILanguage:()=>language},runtime:{onMessage:{addListener:fn=>listeners.message=fn},sendMessage:async message=>{
    sent.push(message);if(message.action==='media')return structuredClone(result);
    return message.id===failure?{ok:false,error:'unsupported_request'}:{ok:true};
  }},storage:{onChanged:{addListener:fn=>listeners.storage=fn}}};
  const document={documentElement:{},getElementById:get,querySelectorAll:()=>[],createElement:()=>new Element()};
  const ctx={document,chrome,location:{href:'chrome-extension://extension/media.html?tab=1'},URL,Intl,setTimeout:(fn,delay)=>{const id=++timerId;timers.set(id,{fn,delay});return id;},clearTimeout:id=>timers.delete(id)};
  vm.createContext(ctx);
  for (const file of ['rules.js', 'i18n.js', 'media-ui.js', 'media-library.js', 'previews.js', 'media.js']) {
    vm.runInContext(fs.readFileSync("extension/" + file, "utf8"), ctx);
    if (file === "i18n.js") {
      for (const [locale, directory] of [["en", "en"], ["zh", "zh_CN"]]) {
        const catalog = fs.readFileSync("extension/_locales/" + directory + "/messages.json", "utf8");
        vm.runInContext("installCatalog(" + JSON.stringify(locale) + ", " + catalog + ")", ctx);
      }
    }
  }
  return {get,sent,listeners,timers,setResult:value=>result=value,ready:()=>new Promise(resolve=>setImmediate(resolve))};
}

test('resource window filters, preserves selection and sends only selected opaque IDs',async()=>{
  const t=setup();await t.ready();assert.equal(t.get('list').children.length,2);
  const stream=t.get('list').children[1];assert.match(stream.children[2].children[1].textContent,/Size unknown/);
  stream.children[0].checked=true;stream.children[0].onchange();
  t.get('kind').value='video';t.get('kind').onchange();assert.equal(t.get('list').children.length,1);
  assert.equal(t.get('download').disabled,true);
  t.get('kind').value='hls';t.get('kind').onchange();
  assert.equal(t.get('download').disabled,false);assert.match(t.get('download').textContent,/\(1\)/);
  await t.get('download').onclick();
  const sends=t.sent.filter(message=>message.action==='send');
  assert.equal(sends.length,1);assert.equal(sends[0].id,'stream');assert.equal(sends[0].tabId,1);
  assert.deepEqual(Object.keys(sends[0]).sort(),['action','id','tabId']);
  assert.equal(t.get('download').disabled,true);
  const [{fn}]=t.timers.values();fn();assert.equal(t.get('feedback').textContent,'');
});

test('batch sends retain failed selection and show per-resource errors in Chinese',async()=>{
  const t=setup({language:'zh',failure:'stream'});await t.ready();
  t.get('selectAll').checked=true;t.get('selectAll').onchange();
  await t.get('download').onclick();assert.equal(t.sent.filter(message=>message.action==='send').length,2);
  assert.match(t.get('feedback').textContent,/1 \/ 2/);assert.match(t.get('download').textContent,/下载所选 \(1\)/);
  assert(t.get('list').children[1].children[2].children[2].children.some(child=>child.textContent.includes('无法安全重放')));
  assert.equal(t.get('download').disabled,false);
});

test('resource notifications stay scoped to source tab and a closed source clears selection',async()=>{
  const t=setup();await t.ready();
  t.get('selectAll').checked=true;t.get('selectAll').onchange();const count=t.sent.length;
  t.listeners.message({action:'mediaChanged',tabId:2});await t.ready();assert.equal(t.sent.length,count);
  t.setResult({items:[],language:'en',source:null});t.listeners.message({action:'mediaChanged',tabId:1});await t.ready();
  assert.equal(t.get('list').children.length,0);assert.equal(t.get('download').disabled,true);
  assert.match(t.get('empty').textContent,/source tab is closed/);
});
