const test=require('node:test');
const assert=require('node:assert/strict');
const vm=require('node:vm');
const fs=require('node:fs');

function setup() {
  const elements=new Map(),sent=[],saved=[];
  class Element {
    constructor(){this.children=[];this.classes=new Set();this.classList={contains:key=>this.classes.has(key),add:key=>this.classes.add(key),toggle:(key,value)=>value?this.classes.add(key):this.classes.delete(key)};}
    replaceChildren(...children){this.children=children;}
    append(...children){this.children.push(...children);}
    setAttribute(name,value){this[name]=value;}
  }
  const get=id=>{if(!elements.has(id))elements.set(id,new Element());return elements.get(id);};
  const document={documentElement:{},getElementById:get,querySelectorAll:()=>[],createElement:()=>new Element()};
  const item={id:'media-1',url:'https://x.test/movie.mp4',type:'video/mp4',size:2097152};
  let fail=false;
  const chrome={storage:{local:{get:async()=>({backend:'next',language:'en'}),set:async value=>saved.push(value)}},tabs:{query:async()=>[{id:1,url:'https://page.test'}]},i18n:{getUILanguage:()=> 'en'},runtime:{onMessage:{addListener(){}},sendMessage:async message=>{
    sent.push(message);
    if(message.action==='media')return {items:[item]};
    if(message.action==='status')return {connected:true};
    if(fail)throw Error('disconnected');return {ok:false,error:'unsupported_request'};
  }}};
  const ctx={chrome,document,URL,Intl};vm.createContext(ctx);
  for(const file of ['rules.js','media-ui.js','popup.js'])vm.runInContext(fs.readFileSync('extension/'+file,'utf8'),ctx);
  return {get,sent,saved,fail:()=>fail=true,ready:()=>new Promise(resolve=>setImmediate(resolve))};
}

test('popup shows file size and actionable send failures while restoring its button',async()=>{
  const t=setup();await t.ready();
  const row=t.get('mediaList').children[0],button=row.children[1];
  assert.match(row.children[0].children[1].textContent,/2 MB/);
  await button.onclick();assert.equal(button.disabled,false);
  assert.match(t.get('message').textContent,/cannot be replayed/i);
  const sent=t.sent.find(message=>message.action==='send');assert.equal(sent.id,'media-1');assert.equal(sent.item,undefined);
  t.fail();await button.onclick();assert.equal(button.disabled,false);assert.match(t.get('message').textContent,/connection/i);
  const html=fs.readFileSync('extension/popup.html','utf8');
  assert(html.indexOf('id="message"')<html.indexOf('<details'));
});

test('popup inline toggle persists immediately without saving other settings',async()=>{
  const t=setup();await t.ready();await t.get('inlineMedia').onchange({target:{checked:false}});
  assert.equal(t.saved.length,1);assert.equal(t.saved[0].inlineMedia,false);assert.deepEqual(Object.keys(t.saved[0]),['inlineMedia']);
});
