const test=require('node:test');
const assert=require('node:assert/strict');
const vm=require('node:vm');
const fs=require('node:fs');

// A small DOM boundary double: production matching, rendering, messaging and
// lifecycle code run unchanged, without browser automation or network access.
function setup({popovers=true}={}) {
  const nodes=[],listeners={},messages=[],frames=[],observers=[],timers=new Map();let timerSequence=0;
  class Element {
    constructor(tag) {this.tag=tag;this.children=[];this.style={};this.attributes={};this.events={};this.textContent='';this.isConnected=false;this.offsetWidth=190;this.offsetHeight=30;this.classes=new Set();this.classList={add:value=>this.classes.add(value),remove:value=>this.classes.delete(value),contains:value=>this.classes.has(value)};nodes.push(this);}
    showPopover() {this.popoverOpen=true;}
    hidePopover() {this.popoverOpen=false;}
    matches(selector) {return selector===':popover-open' && this.popoverOpen===true;}
    append(...children) {for(const child of children){child.parent=this;child.isConnected=this.isConnected;this.children.push(child);}}
    remove() {if(this.parent)this.parent.children=this.parent.children.filter(child=>child!==this);this.isConnected=false;}
    replaceChildren(...children) {this.children=[];this.append(...children);}
    attachShadow() {this.shadow=new Element('shadow');this.shadow.isConnected=true;return this.shadow;}
    addEventListener(name,fn) {this.events[name]=fn;}
    setAttribute(name,value) {this.attributes[name]=value;}
    querySelectorAll() {return [];}
    getBoundingClientRect() {return {left:10,top:10,right:650,bottom:370,width:640,height:360};}
  }
  if (!popovers) Element.prototype.showPopover=undefined;
  const root=new Element('html');root.isConnected=true;
  let players=[],inlineMedia=true,reject=false,blocked=false;
  const item={id:'detected',url:'https://video.test/file.mp4',size:2097152,type:'video/mp4'};
  const document={documentElement:root,createElement:tag=>new Element(tag),querySelectorAll:()=>players,addEventListener:(name,fn)=>listeners[name]=fn};
  const chrome={i18n:{getUILanguage:()=> 'en'},runtime:{onMessage:{addListener:fn=>listeners.message=fn},sendMessage:async message=>{
    messages.push(message);if(message.action==='media') return {items:blocked?[]:[item],inlineMedia,language:'en'};
    if(reject)throw Error('offline');return {ok:true};
  }},storage:{onChanged:{addListener:fn=>listeners.storage=fn}}};
  const ctx={document,chrome,URL,Intl,setTimeout:(fn,delay)=>{const id=++timerSequence;timers.set(id,{fn,delay});return id;},clearTimeout:id=>timers.delete(id),innerWidth:1000,innerHeight:800,getComputedStyle:()=>({visibility:'visible'}),requestAnimationFrame:fn=>{frames.push(fn);return frames.length;},ResizeObserver:class{observe(){}unobserve(){}},MutationObserver:class{constructor(fn){observers.push(fn);}observe(){}},addEventListener:(name,fn)=>listeners[name]=fn};
  vm.createContext(ctx);
  for(const file of ['rules.js','media-ui.js','content.js']) vm.runInContext(fs.readFileSync('extension/'+file,'utf8'),ctx);
  const flush=async()=>{await new Promise(resolve=>setImmediate(resolve));while(frames.length)frames.shift()();};
  const addPlayer=()=>{const player=new Element('video');player.src=item.url;player.currentSrc=item.url;root.append(player);players.push(player);observers[0]();return player;};
  return {messages,listeners,flush,addPlayer,timers,expire:()=>{for(const [id,{fn}] of [...timers]){timers.delete(id);fn();}},host:()=>root.children.find(node=>node.attributes.popover==='manual'),buttons:()=>nodes.filter(node=>node.tag==='button' && node.isConnected),disable:()=>{inlineMedia=false;listeners.storage({inlineMedia:{newValue:false}},'local');},block:()=>{blocked=true;listeners.storage({blockedDomains:{newValue:['page.test']}},'local');},reject:()=>reject=true,remove:player=>{player.remove();players=players.filter(p=>p!==player);observers[0]();}};
}

test('inline button uses detected ID, reports failed sends and disappears when disabled',async()=>{
  const t=setup();t.addPlayer();await t.flush();
  const [button]=t.buttons();assert(button);assert.equal(button.hidden,false);
  assert(button.children.some(child=>child.textContent==='2 MB'));
  t.reject();await button.events.click({preventDefault(){},stopPropagation(){}});
  assert.equal(button.disabled,false);assert.match(button.textContent,/connection/);
  const message=t.messages.find(message=>message.action==='send');
  assert.deepEqual(Object.keys(message).sort(),['action','id']);assert.equal(message.id,'detected');
  t.disable();await t.flush();assert.equal(t.buttons().length,0);
});

test('send notices expire and restore the compact button; disabling clears pending notices',async()=>{
  const t=setup();t.addPlayer();await t.flush();const [button]=t.buttons();
  await button.events.click({preventDefault(){},stopPropagation(){}});
  assert.equal(t.timers.size,1);
  assert([...t.timers.values()][0].delay<=1500);
  t.expire();await t.flush();assert.equal(button.classList.contains('notice'),false);
  assert(button.children.some(child=>/NDM.*Download/.test(child.textContent)));
  t.reject();await button.events.click({preventDefault(){},stopPropagation(){}});
  assert.equal(t.timers.size,1);t.disable();await t.flush();assert.equal(t.timers.size,0);
});

test('SPA media insertion and removal use cached captures without request polling',async()=>{
  const t=setup();await t.flush();const player=t.addPlayer();await t.flush();assert.equal(t.buttons().length,1);
  const queries=t.messages.filter(message=>message.action==='media').length;
  for(let i=0;i<25;i++)t.listeners.scroll();await t.flush();
  assert.equal(t.messages.filter(message=>message.action==='media').length,queries);
  t.remove(player);await t.flush();assert.equal(t.buttons().length,0);
  t.addPlayer();await t.flush();assert.equal(t.buttons().length,1);
});

test('blocking a domain removes its existing inline control immediately',async()=>{
  const t=setup();t.addPlayer();await t.flush();assert.equal(t.buttons().length,1);
  t.block();await t.flush();assert.equal(t.buttons().length,0);
});

test('inline control is above the player in a non-modal top-layer host and returns after fullscreen changes',async()=>{
  const t=setup();const player=t.addPlayer();
  player.getBoundingClientRect=()=>({left:10,top:120,right:650,bottom:480,width:640,height:360});
  await t.flush();
  assert.equal(t.buttons()[0].style.top,'84px');
  assert.equal(t.host().popoverOpen,true);
  assert.match(t.host().style.cssText,/pointer-events:none/);
  assert.match(t.host().shadow.children[0].textContent,/:host::backdrop\{[^}]*pointer-events:none/);
  t.host().hidePopover();t.listeners.fullscreenchange();await t.flush();
  assert.equal(t.host().popoverOpen,true);
});

test('browsers without the popover API retain the fixed-position media control',async()=>{
  const t=setup({popovers:false});t.addPlayer();await t.flush();
  assert.equal(t.buttons().length,1);
  assert.equal(t.buttons()[0].style.top,'16px');
  assert.equal(t.host().popoverOpen,undefined);
});
