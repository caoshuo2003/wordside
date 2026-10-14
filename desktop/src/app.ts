declare const WORDSIDE_SAMPLES: Record<string, LookupPayload>;
let sampleJob:Promise<void>|undefined;
function loadSamples(){
 if(typeof WORDSIDE_SAMPLES!=='undefined')return Promise.resolve();
 return sampleJob ||= new Promise<void>((resolve,reject)=>{const script=document.createElement('script');script.src='samples.js';script.onload=()=>resolve();script.onerror=()=>{sampleJob=undefined;script.remove();reject(new Error('演示数据加载失败'));};document.head.append(script);});
}
const get = <T extends HTMLElement>(id:string) => document.getElementById(id) as T;
const card = get<HTMLElement>('card');
const enabled = get<HTMLInputElement>('enabled');
const statusLine = get<HTMLElement>('status');
const terminal = get<HTMLElement>('terminal');
const pin = get<HTMLButtonElement>('pin');
const theme = get<HTMLSelectElement>('theme');
const palette = get<HTMLSelectElement>('palette');
const palettes=['green','lavender','blue','stone'];
let selectedPalette='green';
function applyPalette(value:string) {
  selectedPalette=palettes.includes(value)?value:'green';
  document.body.dataset.palette=selectedPalette;
  palette.value=selectedPalette;
}
const body = get<HTMLElement>('entry-body');
let pinned=false, timer:ReturnType<typeof setTimeout> | undefined, generation=0;
let request:AbortController | undefined;
let audio:HTMLAudioElement | undefined;
let currentEntry:Entry | undefined;
let lastCompleted:{entry:Entry;origin:string}|undefined;
let queryInFlight=false;
let manualLookup=false;
function el<K extends keyof HTMLElementTagNameMap>(tag:K, content='', cls=''): HTMLElementTagNameMap[K] {
  const node=document.createElement(tag); node.textContent=content; node.className=cls; return node;
}
function remember(key:string,value:string) { try { localStorage.setItem('wordside.'+key,value); } catch {} }
function recall(key:string,otherwise:string) { try { return localStorage.getItem('wordside.'+key) || otherwise; } catch { return otherwise; } }
const systemTheme=matchMedia('(prefers-color-scheme: dark)');
function applyTheme() {
  const dark=theme.value==='dark' || (theme.value==='system' && systemTheme.matches);
  document.body.classList.toggle('dark',dark);
  document.body.classList.toggle('light',!dark);
  document.documentElement.style.colorScheme=dark?'dark':'light';
}
theme.value=recall('theme','system');
if(!['dark','light','system'].includes(theme.value)) theme.value='system';
theme.addEventListener('change',()=>{remember('theme',theme.value);applyTheme();});
systemTheme.addEventListener('change',applyTheme); applyTheme();
applyPalette(recall('palette','green'));
palette.addEventListener('change',()=>{applyPalette(palette.value);remember('palette',selectedPalette);});
enabled.checked=recall('enabled','true')==='true';
function cancel() { clearTimeout(timer);body.setAttribute('aria-busy','false');queryInFlight=false; generation++; request?.abort(); audio?.pause(); return nativeAPI?.core.invoke<void>('cancel_lookup').catch(()=>{}); }
function closeCard(fromNative=false) { manualLookup=false;lastCompleted=undefined;void cancel(); card.hidden=true; pinned=false; pin.setAttribute('aria-pressed','false'); if(isPopup&&!fromNative)void nativeAPI?.core.invoke('hide_popup'); }
function positionCard(x:number,y:number) {
  if(isPopup||pinned)return;
  const width=card.offsetWidth,height=card.offsetHeight;
  card.style.left=`${Math.min(Math.max(12,x),Math.max(12,innerWidth-width-12))}px`;
  card.style.top=`${Math.min(Math.max(12,y+height+12>innerHeight?y-height-20:y),Math.max(12,innerHeight-height-12))}px`;
}
function showCard(x:number,y:number) {card.hidden=false;positionCard(x,y);}
function updateStatus() {statusLine.textContent=enabled.checked?'自动划词已开启 · 选择 terminal 或 run':'自动划词已关闭 · 选中文字不会发起查询';}
enabled.addEventListener('change',()=>{if(!manualLookup){if(!enabled.checked&&pinned)prepareCapture();else void cancel();if(!enabled.checked&&!pinned)closeCard();}remember('enabled',String(enabled.checked));updateStatus();}); updateStatus();
function play(url:string,button:HTMLButtonElement) {
  audio?.pause(); audio=new Audio(url);
  void audio.play().catch(()=>{button.textContent='↻';button.title='暂时无法播放，点击重试';button.setAttribute('aria-label','重新播放发音');});
}
// Reuse unchanged nodes without resetting focus or details.
function reconcileChildren(parent:Element, children:HTMLElement[]) {
  const retained=new Set<Node>(children);
  for(const child of Array.from(parent.childNodes))if(!retained.has(child))child.remove();
  let cursor=parent.firstChild;
  for(const child of children){
    if(child===cursor)cursor=cursor.nextSibling;
    else parent.insertBefore(child,cursor);
  }
}
function renderText<K extends keyof HTMLElementTagNameMap>(parent:HTMLElement,tag:K,content:string,cls:string):HTMLElementTagNameMap[K] {
  const node=parent.querySelector<HTMLElementTagNameMap[K]>(`:scope > .${cls}`)||el(tag,'',cls);
  if(node.textContent!==content)node.textContent=content;
  return node;
}
const detailContents=new WeakMap<HTMLDetailsElement,string>();
function renderDetails(parent:HTMLElement,cls:string,summary:string,content:unknown,items:(previous:HTMLElement[])=>HTMLElement[]):HTMLDetailsElement {
  const details=parent.querySelector<HTMLDetailsElement>(`:scope > .${cls}`)||el('details','',cls);
  const signature=JSON.stringify(content);
  if(detailContents.get(details)!==signature){
    const caption=details.querySelector('summary')||el('summary');
    if(caption.textContent!==summary)caption.textContent=summary;
    const previous=Array.from(details.children).filter(node=>node!==caption) as HTMLElement[];
    reconcileChildren(details,[caption,...items(previous)]);detailContents.set(details,signature);
  }
  return details;
}
let renderedWord:string|undefined, renderedPronunciation:string|undefined;
function renderLookupState(title:string,message:string,loading=false){
  const state=el('div','','lookup-state'+(loading?' is-loading':''));state.setAttribute('role','status');
  const icon=el('span',loading?'':'⌕','lookup-state-icon');icon.setAttribute('aria-hidden','true');
  state.append(icon,el('h3',title,'state-title'));if(message)state.append(el('p',message,'state-message'));body.replaceChildren(state);
}
function resetTranslationStatus(){get('translation-status').hidden=true;get('translation-status').textContent='';delete card.dataset.translationState;}
function renderEntry(entry:Entry, origin:string) {
  const sameWord=renderedWord===entry.word&&currentEntry?.word===entry.word;
  const scroll=sameWord?body.scrollTop:0;
  const oldSenses=sameWord?Array.from(body.querySelectorAll<HTMLElement>(':scope > .sense')):[];
  const oldHeadings=sameWord?Array.from(body.querySelectorAll<HTMLElement>(':scope > .pos-heading')):[];
  body.setAttribute('aria-busy','false');get('source-link').textContent='原词典 ↗';currentEntry=entry;lastCompleted={entry,origin};get('word').textContent=entry.word;get('word-pos').textContent=entry.pos.join(' · ');
  get<HTMLAnchorElement>('source-link').href=entry.sourceUrl||wordURL(entry.word);
  const translated=entry.definitions.filter(d=>d.translation.trim()).length;
  const missing=entry.definitions.length-translated;
  const state=!entry.definitions.length?'empty':!missing?'bilingual':!translated?'english':'partial';
  card.dataset.translationState=state;
  const badge=get('translation-status');badge.hidden=state==='bilingual'||state==='empty';
  badge.textContent=state==='english'?'英文释义':state==='partial'?'部分仅英文':'';
  const totalExamples=entry.definitions.reduce((n,d)=>n+d.examples.length,0);
  get('entry-count').textContent=entry.definitions.length?`${entry.definitions.length} 条释义${totalExamples?` · ${totalExamples} 条例句`:''}`:'';
  get('origin').textContent=entry.query&&identity(entry.query)!==identity(entry.word)?`${entry.query} → ${entry.word}`:origin;
  get('expand-examples').hidden=totalExamples===0;
  const children:HTMLElement[]=[];
  // Definitions and examples remain complete; only unchanged DOM is reused.
  let lastGroup='',headingIndex=0;
  entry.definitions.forEach((d,i)=>{
    const group=d.pos;
    if(group&&group!==lastGroup){const heading=oldHeadings[headingIndex++]||el('h3','','pos-heading');if(heading.textContent!==group)heading.textContent=group;children.push(heading);lastGroup=group;}
    const hasTranslation=Boolean(d.translation.trim());
    const section=oldSenses[i]||el('section','','sense');section.dataset.index=String(i);section.classList.toggle('without-translation',!hasTranslation);
    const number=renderText(section,'span',String(i+1).padStart(2,'0'),'number');
    const content=section.querySelector<HTMLElement>(':scope > .sense-content')||el('div','','sense-content');
    const parts:HTMLElement[]=[];
    if(hasTranslation)parts.push(renderText(content,'h4',d.translation,'translation'));
    else if(state==='partial')parts.push(renderText(content,'span','英文释义','translation-note'));
    if(d.text.trim())parts.push(renderText(content,'p',d.text,'definition-text'));
    else if(!hasTranslation)parts.push(renderText(content,'p','暂无释义','definition-text'));
    if(d.examples.length)parts.push(renderDetails(content,'examples',`例句 ${d.examples.length}`,d.examples,previous=>d.examples.map((e,j)=>{
      const quote=previous[j]||el('blockquote');const value=e.text;
      if(quote.firstChild?.nodeType===Node.TEXT_NODE){if(quote.firstChild.nodeValue!==value)quote.firstChild.nodeValue=value;}
      else quote.prepend(document.createTextNode(value));
      const translation=quote.querySelector('small');
      if(e.translation.trim()){if(translation){if(translation.textContent!==e.translation)translation.textContent=e.translation;}else quote.append(el('small',e.translation));}
      else translation?.remove();
      return quote;
    })));
    const source=d.source;
    if(source)parts.push(renderText(content,'p',source,'sense-source'));
    reconcileChildren(content,parts);reconcileChildren(section,[number,content]);children.push(section);
  });
  const pronunciation=get('pronunciation');const pronunciationContent=JSON.stringify(entry.pronunciation);
  if(!sameWord||renderedPronunciation!==pronunciationContent){
    const previous=sameWord?Array.from(pronunciation.querySelectorAll<HTMLElement>(':scope > .pron-row')):[];
    const rows=entry.pronunciation.map((a,i)=>{
      const row=previous[i]||el('div','','pron-row');
      const parts:HTMLElement[]=[Object.assign(renderText(row,'span',a.pos,'pron-pos'),{title:a.pos}),renderText(row,'b',a.lang.toUpperCase(),'pron-lang'),renderText(row,'span',a.pron,'pron-value')];
      if(a.url){
        let button=row.querySelector<HTMLButtonElement>('.audio-button');
        if(!button){button=el('button','▶','audio-button');button.type='button';const audioButton=button;button.addEventListener('click',()=>play(audioButton.dataset.url!,audioButton));}
        if(button.dataset.url!==a.url){button.dataset.url=a.url;button.textContent='▶';}
        button.setAttribute('aria-label',`${a.pos} ${a.lang} 发音`);parts.push(button);
      }
      reconcileChildren(row,parts);return row;
    });
    reconcileChildren(pronunciation,rows);renderedPronunciation=pronunciationContent;
  }
  if(children.length)reconcileChildren(body,children);else renderLookupState('暂无释义','可以查看原词典。');renderedWord=entry.word;
  get('expand-examples').textContent=Array.from(body.querySelectorAll<HTMLDetailsElement>('.examples')).some(d=>!d.open)?'展开全部例句':'收起全部例句';body.scrollTop=scroll;
}
function renderPayload(payload:LookupPayload,origin:string,query:string){renderEntry(readApiEntry(payload,query,payload.language||'en-cn'),origin);}
function settlePayload(payload:LookupPayload,accepted:boolean){
  if(nativeAPI&&payload.cacheTicket!==undefined)void nativeAPI.core.invoke('settle_lookup',{ticket:payload.cacheTicket,accepted}).catch(()=>{});
}
async function requestPayload(word:string,language:'en-cn'|'en',refresh:boolean,signal:AbortSignal):Promise<LookupPayload>{
  if(signal.aborted)throw new DOMException('查询已取消','AbortError');
  if(nativeAPI){
    const serial=generation;
    return new Promise((resolve,reject)=>{
      const abort=()=>{if(serial===generation)void nativeAPI!.core.invoke('cancel_lookup').catch(()=>{});reject(new DOMException('查询已取消','AbortError'));};
      signal.addEventListener('abort',abort,{once:true});
      nativeAPI!.core.invoke<LookupPayload>('lookup',{word,refresh,language}).then(resolve,reject).finally(()=>signal.removeEventListener('abort',abort));
    });
  }
  throw new Error('本机查词请使用 WordSide 桌面版。');
}
async function lookup(word:string,x:number,y:number,mode:'sample'|'live'='live',refresh=false){
  resetTranslationStatus();get('source-link').textContent='原词典 ↗';card.classList.remove('capture-error');get('source-link').hidden=false;get('expand-examples').hidden=false;
  const cancellation=cancel(),serial=generation,controller=new AbortController();request=controller;queryInFlight=true;
  get('word').textContent=word;get('word-pos').textContent='';get('entry-count').textContent='';get('origin').textContent='';
  get<HTMLAnchorElement>('source-link').href=wordURL(word);get('pronunciation').replaceChildren();
  currentEntry=undefined;renderedWord=undefined;renderedPronunciation=undefined;
  get('expand-examples').hidden=true;body.setAttribute('aria-busy','true');renderLookupState('正在查词','',true);showCard(x,y);
  try{
    word=normalizeSelection(word);get('word').textContent=word;get<HTMLAnchorElement>('source-link').href=wordURL(word);
    if(mode==='sample'){
      await loadSamples();if(serial!==generation)return;
      if(!WORDSIDE_SAMPLES[word])throw new Error('没有此词的 API 快照，请使用在线查询。');
      renderPayload(WORDSIDE_SAMPLES[word],'示例词条',word);
    }else{
      const timeout=setTimeout(()=>controller.abort(),35000),alive=()=>serial===generation&&!controller.signal.aborted;
      const fetchEntry=async(language:'en-cn'|'en')=>{
        const payload=await requestPayload(word,language,refresh,controller.signal);
        if(!alive()){if(!payload.cacheHit)settlePayload(payload,false);throw new DOMException('查询已取消','AbortError');}
        try{const entry=readApiEntry(payload,word,language);settlePayload(payload,entry.definitions.length>0);return {entry,payload};}
        catch(error){settlePayload(payload,false);throw error;}
      };
      try{
        await cancellation;if(!alive())return;
        let result:{entry:Entry;payload:LookupPayload};
        try{result=await fetchEntry('en-cn');}catch(primary){
          if(!(primary instanceof ApiLookupError)||!primary.notFound||!alive())throw primary;
          try{result=await fetchEntry('en');}catch(english){
            if(!(english instanceof ApiLookupError)||!english.notFound||!alive())throw english;
            throw new ApiLookupError(`未找到「${word}」`,true,combineSuggestions(word,primary.suggestions,english.suggestions));
          }
        }
        if(!alive())return;
        renderEntry(result.entry,'Cambridge');
      }finally{clearTimeout(timeout);}
    }
    if(serial===generation)positionCard(x,y);
  }catch(error){
    if(serial!==generation)return;
    const missing=error instanceof ApiLookupError&&error.notFound;
    const notice=lookupNotice(error);get('entry-count').textContent='';get('expand-examples').hidden=true;
    if(error instanceof SelectionInputError)get('source-link').hidden=true;
    renderLookupState(notice.title,missing&&!error.suggestions.length?'换一个拼写，或输入其他单词。':notice.message);
    if(missing&&error.suggestions.length){
      body.append(el('p','你可能想查','suggestion-label'));
      const candidates=el('div','','suggestions');candidates.setAttribute('aria-label','Cambridge 拼写建议');
      for(const candidate of error.suggestions){
        const button=el('button',candidate,'suggestion');button.type='button';
        button.addEventListener('click',()=>{if(serial===generation)void lookup(candidate,x,y);});candidates.append(button);
      }
      body.append(candidates);
    }
    if(notice.action==='correct'){
      const correction=el('form','','correction'),input=el('input');input.value=word;input.maxLength=100;input.required=true;input.setAttribute('aria-label','查询词');
      const submit=el('button','查词');submit.type='submit';correction.append(input,submit);
      correction.addEventListener('submit',event=>{event.preventDefault();if(serial===generation)void lookup(input.value,x,y);});body.append(correction);
    }else{
      const actions=el('div','','lookup-actions'),retry=el('button','重试');retry.type='button';retry.addEventListener('click',()=>{if(serial===generation)void lookup(word,x,y,mode,true);});actions.append(retry);body.append(actions);
    }
  }finally{if(serial===generation){queryInFlight=false;body.setAttribute('aria-busy','false');}}
}
interface CaptureFailure {code:string;message?:string;serial?:number;manual?:boolean}
function clearCapturedEntry(){
  resetTranslationStatus();body.setAttribute('aria-busy','false');void cancel();currentEntry=undefined;lastCompleted=undefined;renderedWord=undefined;renderedPronunciation=undefined;
  get('word').textContent='';get('word-pos').textContent='';get('entry-count').textContent='';get('origin').textContent='';get('pronunciation').replaceChildren();body.replaceChildren();
  get<HTMLAnchorElement>('source-link').removeAttribute('href');get('source-link').hidden=true;get('expand-examples').hidden=true;card.hidden=true;
}
// A capture may end silently (unsupported area or changed source). Keep a pinned
// result visible until an actual selected word arrives; never leave speculative loading.
function prepareCapture(){
  if(!pinned||card.hidden){clearCapturedEntry();return;}
  const wasPending=queryInFlight;void cancel();
  if(!wasPending)return;
  if(lastCompleted){
    card.classList.remove('capture-error');get('source-link').hidden=false;get('expand-examples').hidden=false;
    renderEntry(lastCompleted.entry,lastCompleted.origin);
  }else{
    get('entry-count').textContent='';get('expand-examples').hidden=true;body.setAttribute('aria-busy','false');
    renderLookupState('查询已取消','选一个词重新查询。');
  }
}
function showCaptureFailure(failure:CaptureFailure){
  clearCapturedEntry();card.classList.add('capture-error');manualLookup=true;
  const notice=captureNotice(failure.code);get('word').textContent=notice.title;
  const content=el('div','','capture-failure');content.append(el('p',notice.message));
  const form=el('form','','correction');const input=el('input');input.placeholder='输入英文单词或短语';input.maxLength=100;input.required=true;input.setAttribute('aria-label','直接输入查询词');
  const submit=el('button','查词');submit.type='submit';form.append(input,submit);form.addEventListener('submit',event=>{event.preventDefault();void lookup(input.value,0,0);});content.append(form);body.append(content);showCard(0,0);
}
get('preview').addEventListener('click',()=>void lookup('terminal',innerWidth*.53,innerHeight*.2,'sample'));
get('preview-run').addEventListener('click',()=>void lookup('run',innerWidth*.53,innerHeight*.2,'sample'));
get('lookup-form').addEventListener('submit',event=>{event.preventDefault();const word=get<HTMLInputElement>('query').value.trim();if(word)void lookup(word,innerWidth*.53,innerHeight*.2);});
terminal.addEventListener('pointerup',event=>{
  clearTimeout(timer);if(!enabled.checked)return;
  const selection=getSelection();if(!selection||!terminal.contains(selection.anchorNode)||!terminal.contains(selection.focusNode))return;
  const word=selection.toString().trim();if(!word||word.length>100||word.includes('\n'))return;
  const mode=['terminal','run'].includes(word.toLowerCase())?'sample':'live';
  timer=setTimeout(()=>{if(enabled.checked&&getSelection()?.toString().trim()===word)void lookup(mode==='sample'?word.toLowerCase():word,event.clientX+14,event.clientY+16,mode);},180);
});
get('close').addEventListener('click',()=>closeCard());
pin.addEventListener('click',()=>{pinned=!pinned;pin.setAttribute('aria-pressed',String(pinned));});
get('expand-examples').addEventListener('click',()=>{
  const all=Array.from(body.querySelectorAll<HTMLDetailsElement>('.examples'));const open=all.some(d=>!d.open);all.forEach(d=>d.open=open);get('expand-examples').textContent=open?'收起全部例句':'展开全部例句';
});
document.addEventListener('pointerdown',event=>{clearTimeout(timer);if(isPopup)return;if(!pinned&&!card.contains(event.target as Node)&&!(event.target as Element).closest('button,input,select,label,form'))closeCard();});
document.addEventListener('keydown',event=>{if(event.key==='Escape')closeCard();});
window.addEventListener('resize',()=>{if(!card.hidden)positionCard(parseFloat(card.style.left),parseFloat(card.style.top));});
const params=new URLSearchParams(location.search);
if(['dark','light'].includes(params.get('theme')||'')){theme.value=params.get('theme')!;applyTheme();}
if(palettes.includes(params.get('palette')||''))applyPalette(params.get('palette')!);
if(params.has('preview'))void lookup(params.get('preview')==='run'?'run':'terminal',innerWidth*.53,innerHeight*.16,'sample');
