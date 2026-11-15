(() => {
  const words={
    en:{title:'Media library',refresh:'Refresh',search:'Search name, source or format',all:'All formats',video:'Video',audio:'Audio',selectAll:'Select visible',empty:'Play media on the source page to discover resources.',noMatch:'No matching resources.',closed:'The source tab is closed. Open the library again from a web page.',hint:'Streams are merged by NDM Next. Their total size may be unknown.',download:'Download selected',send:'Download',sent:'Sent ✓',sending:'Sending…',type:'Media type',list:'Detected media',count:(shown,total,selected)=>`${shown} / ${total} resources · ${selected} selected`,done:(sent,total)=>`Sent ${sent} / ${total} to NDM Next`},
    zh:{title:'媒体资源',refresh:'刷新',search:'搜索名称、来源或格式',all:'全部格式',video:'视频',audio:'音频',selectAll:'选择当前结果',empty:'在来源页面播放媒体后，这里会显示发现的资源。',noMatch:'没有匹配的资源。',closed:'来源标签页已关闭，请从网页重新打开资源窗口。',hint:'流媒体由 NDM Next 合并，下载前可能无法确定总大小。',download:'下载所选',send:'下载',sent:'已发送 ✓',sending:'正在发送…',type:'媒体类型',list:'发现的媒体',count:(shown,total,selected)=>`${shown} / ${total} 项资源 · 已选 ${selected} 项`,done:(sent,total)=>`已向 NDM Next 发送 ${sent} / ${total} 项`},
  };
  const $=id=>document.getElementById(id);
  const raw=new URL(location.href).searchParams.get('tab');
  const tabId=raw !== null && /^\d+$/.test(raw) ? Number(raw) : NaN;
  const selected=new Set(),results=new Map();
  let items=[],language='en',sourceClosed=false,busy=false,refreshing=false,refreshAgain=false,noticeTimer;
  const text=key=>words[language][key];
  function translate() {
    document.documentElement.lang=language;document.title=`NDM Next · ${text('title')}`;
    for(const el of document.querySelectorAll('[data-i18n]')) el.textContent=text(el.dataset.i18n);
    $('search').placeholder=text('search');$('search').setAttribute('aria-label',text('search'));
    $('kind').setAttribute('aria-label',text('type'));$('list').setAttribute('aria-label',text('list'));
  }
  function notice(message,error=false) {
    clearTimeout(noticeTimer);$('feedback').textContent=message;$('feedback').classList.toggle('error',error);
    noticeTimer=setTimeout(()=>{$('feedback').textContent='';},error?6000:2500);
  }
  const visible=()=>filterMedia(items,$('search').value,$('kind').value);
  function element(tag,className,value) {
    const el=document.createElement(tag);if(className)el.className=className;if(value !== undefined)el.textContent=value;return el;
  }
  function updateSelection() {
    const shown=visible(),n=shown.filter(item=>selected.has(item.id)).length;
    $('selectAll').checked=shown.length>0 && n === shown.length;
    $('selectAll').indeterminate=n>0 && n<shown.length;
    $('selectAll').disabled=busy || !shown.length;
    $('download').disabled=busy || sourceClosed || !selected.size;
    $('download').textContent=busy?text('sending'):`${text('download')}${selected.size?` (${selected.size})`:''}`;
    $('count').textContent=words[language].count(shown.length,items.length,selected.size);
  }
  function render() {
    const shown=visible();$('list').replaceChildren();
    for(const item of shown) {
      const row=element('div','resource'+(selected.has(item.id)?' selected':''));
      const checkbox=element('input');checkbox.type='checkbox';checkbox.checked=selected.has(item.id);checkbox.disabled=busy;
      checkbox.setAttribute('aria-label',mediaName(item));
      checkbox.onchange=()=>{if(checkbox.checked)selected.add(item.id);else selected.delete(item.id);row.classList.toggle('selected',checkbox.checked);updateSelection();};
      const format=element('span','format',mediaFormat(item));
      const info=element('div','info'),name=element('div','name',mediaName(item));name.title=mediaName(item);
      const detail=element('div','meta');detail.append(element('span','host',mediaHost(item)),element('span','size',`${mediaFormat(item)} · ${mediaSizeLabel(item,language)}`));
      info.append(name,detail);
      const result=results.get(item.id);
      if(result?.error) info.append(element('p','failure',mediaMessage(result.error,language)));
      const button=element('button','row-send'+(result?.sent?' sent':''),text(result?.pending?'sending':result?.sent?'sent':'send'));
      button.disabled=busy || sourceClosed;button.onclick=()=>downloadItems([item.id]);
      row.append(checkbox,format,info,button);$('list').append(row);
    }
    $('empty').hidden=shown.length>0;
    $('empty').textContent=text(sourceClosed?'closed':items.length?'noMatch':'empty');
    updateSelection();
  }
  async function downloadItems(ids) {
    if(busy || sourceClosed)return;
    busy=true;let sent=0;
    try {
      for(const id of ids) {
        if(!items.some(item=>item.id===id))continue;
        results.set(id,{pending:true});render();
        let result;
        try {result=await chrome.runtime.sendMessage({action:'send',tabId,id});}
        catch {result={ok:false,error:'manager_unavailable'};}
        if(result?.ok){sent++;selected.delete(id);results.set(id,{sent:true});}
        else results.set(id,{error:result?.error || 'manager_unavailable'});
        render();
      }
      notice(words[language].done(sent,ids.length),sent !== ids.length);
    } finally {busy=false;render();}
  }
  async function refresh() {
    if(refreshing){refreshAgain=true;return;}
    refreshing=true;$('refresh').disabled=true;
    try {
      const result=await chrome.runtime.sendMessage({action:'media',tabId});
      if(!Array.isArray(result?.items))throw Error('unavailable');
      language=mediaLanguage(result.language || 'auto',chrome.i18n.getUILanguage());translate();
      sourceClosed=result.source === null;items=sourceClosed?[]:result.items;
      const ids=new Set(items.map(item=>item.id));
      for(const id of selected)if(!ids.has(id))selected.delete(id);
      for(const id of results.keys())if(!ids.has(id))results.delete(id);
      $('source').textContent=sourceClosed?text('closed'):result.source?.title || result.source?.url || '';
      render();
    } catch {notice(mediaMessage('manager_unavailable',language),true);}
    finally {refreshing=false;$('refresh').disabled=false;if(refreshAgain){refreshAgain=false;refresh();}}
  }
  $('search').oninput=render;$('kind').onchange=render;$('refresh').onclick=refresh;
  $('selectAll').onchange=()=>{for(const item of visible())if($('selectAll').checked)selected.add(item.id);else selected.delete(item.id);render();};
  $('download').onclick=()=>downloadItems([...selected]);
  chrome.runtime.onMessage.addListener(message=>{if(message.action==='mediaChanged' && message.tabId===tabId)refresh();});
  chrome.storage.onChanged.addListener((changes,area)=>{if(area==='local' && Object.keys(changes).some(key=>Object.hasOwn(DEFAULTS,key)))refresh();});
  if(Number.isSafeInteger(tabId) && tabId>=0)refresh();
  else {sourceClosed=true;translate();render();}
})();
