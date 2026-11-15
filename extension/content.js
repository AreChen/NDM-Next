(() => {
  let items=[], enabled=false, language='en', frame, fetching=false, refreshAgain=false;
  let host, shadow;
  const buttons=new Map();
  function ensureHost() {
    if (host?.isConnected) return;
    if (host) {document.documentElement.append(host);return;}
    host=document.createElement('div');
    host.style.cssText='all:initial!important;position:fixed!important;inset:0!important;pointer-events:none!important;z-index:2147483647!important;';
    shadow=host.attachShadow({mode:'closed'});
    const style=document.createElement('style');
    style.textContent=`:host{color-scheme:dark}button{position:absolute;pointer-events:auto;display:flex;align-items:center;gap:7px;max-width:calc(100vw - 16px);border:1px solid #ffffff30;border-radius:9px;padding:7px 10px;background:#171c2ff2;color:#f5f4ff;box-shadow:0 3px 14px #0005;font:600 12px/1.3 system-ui,sans-serif;cursor:pointer;white-space:nowrap}button[hidden]{display:none!important}button:hover{background:#40377b}button:focus-visible{outline:2px solid #b5a7ff;outline-offset:2px}button:disabled{cursor:wait;opacity:.8}.arrow{color:#c1b5ff;font-size:16px}.size{font-weight:400;color:#c6c7d8;font-size:11px}.notice{white-space:normal;max-width:280px}`;
    shadow.append(style); document.documentElement.append(host);
  }
  function schedule() { if (!frame) frame=requestAnimationFrame(() => {frame=undefined;render();}); }
  const resizeObserver=new ResizeObserver(schedule);
  function remove(player) { const entry=buttons.get(player); if (entry) {entry.button.remove();resizeObserver.unobserve(player);buttons.delete(player);} }
  function render() {
    if (buttons.size) ensureHost();
    const players=enabled ? [...document.querySelectorAll('video,audio')].filter(player=>player.isConnected) : [];
    for (const player of buttons.keys()) if (!players.includes(player)) remove(player);
    for (const player of players) {
      const sources=[player.currentSrc,player.src,...[...player.querySelectorAll('source[src]')].map(source=>source.src)].filter(Boolean);
      const item=matchMedia(sources,items,players.length);
      if (!item) {remove(player);continue;}
      let entry=buttons.get(player);
      if (entry && entry.id !== item.id) {remove(player);entry=undefined;}
      if (!entry) {
        ensureHost();
        const button=document.createElement('button'); button.type='button';
        entry={button,id:item.id}; buttons.set(player,entry); resizeObserver.observe(player);
        button.addEventListener('click',async event=>{
          event.preventDefault();event.stopPropagation();button.disabled=true;
          button.textContent=mediaMessage('sending',language);
          try {
            const result=await chrome.runtime.sendMessage({action:'send',id:entry.id});
            button.textContent=mediaMessage(result?.ok?'sent':result?.error,language);
          } catch {button.textContent=mediaMessage('manager_unavailable',language);}
          button.classList.add('notice');button.disabled=false;
          button.setAttribute('aria-label',button.textContent);
        });
        shadow.append(button);
      }
      const {button}=entry;
      if (!button.disabled && !button.classList.contains('notice')) {
        button.replaceChildren();
        for (const [className,value] of [['arrow','↓'],['label',`NDM · ${mediaMessage('download',language)}`],['size',mediaSizeLabel(item,language)]]) {
          const span=document.createElement('span');span.className=className;span.textContent=value;button.append(span);
        }
        button.setAttribute('aria-label',`${mediaMessage('download',language)} · ${mediaSizeLabel(item,language)}`);
        button.title=`NDM Next · ${item.type || ''}`;
      }
      const rect=player.getBoundingClientRect();
      const shown=rect.width>=100 && rect.height>=24 && rect.bottom>0 && rect.right>0 && rect.top<innerHeight && rect.left<innerWidth && getComputedStyle(player).visibility!=='hidden';
      button.hidden=!shown;
      button.style.left=`${Math.max(8,Math.min(rect.right-button.offsetWidth-8,innerWidth-button.offsetWidth-8))}px`;
      button.style.top=`${Math.max(8,rect.top+8)}px`;
    }
    if (!buttons.size) {host?.remove();host=undefined;}
  }
  async function refresh() {
    if (fetching) {refreshAgain=true;return;}
    fetching=true;
    try {
      const result=await chrome.runtime.sendMessage({action:'media'});
      items=result?.items || [];enabled=result?.inlineMedia === true;
      language=mediaLanguage(result?.language,chrome.i18n.getUILanguage());
    } catch {items=[];enabled=false;}
    finally {fetching=false;schedule();if(refreshAgain){refreshAgain=false;refresh();}}
  }
  new MutationObserver(schedule).observe(document.documentElement,{childList:true,subtree:true,attributes:true,attributeFilter:['src']});
  for (const event of ['loadedmetadata','loadstart','emptied','play']) document.addEventListener(event,schedule,true);
  addEventListener('scroll',schedule,{passive:true,capture:true});addEventListener('resize',schedule,{passive:true});
  chrome.runtime.onMessage.addListener(message=>{if(message.action==='mediaChanged') refresh();});
  chrome.storage.onChanged.addListener((changes,area)=>{if(area==='local' && Object.keys(changes).some(key=>Object.hasOwn(DEFAULTS,key))) refresh();});
  refresh();
})();
