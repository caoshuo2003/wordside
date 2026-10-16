// The schema from Nongfsq/cambridge-dictionary-api/data.js.
// Source HTML stays in the native backend; only validated JSON reaches the UI.
interface Example { id?:number; text:string; translation:string }
interface Definition { id?:number; pos:string; source:string; text:string; translation:string; examples:Example[] }
interface Pronunciation { pos:string; lang:string; pron:string; url:string }
interface Entry { word:string; query?:string; language?:string; sourceUrl?:string; pos:string[]; definitions:Definition[]; pronunciation:Pronunciation[]; warnings:string[] }
interface LookupPayload { entry:unknown; apiError?:string; apiStatus?:number; apiCode?:string; suggestions?:unknown; cacheHit?:boolean; cacheTicket?:number; language?:'en-cn'|'en' }
class ApiLookupError extends Error {constructor(message:string,public notFound=false,public suggestions:string[]=[],public code='INVALID_RESPONSE',public status?:number){super(message);}}
class SelectionInputError extends Error {}
function combineSuggestions(query:string,...groups:string[][]):string[]{
  const seen=new Set([identity(query)]),result:string[]=[];
  for(const group of groups)for(const word of group){const key=identity(word);if(!seen.has(key)&&result.length<10){seen.add(key);result.push(word);}}
  return result;
}
function readSuggestions(value:unknown,query:string):string[]{
  if(value===undefined)return [];
  if(!Array.isArray(value)||value.length>10)throw new ApiLookupError('词典建议的数据格式异常，请重试。');
  const words=value.map(word=>{
    if(typeof word!=='string'||!word||word.length>80||word.trim()!==word||word.split(' ').length>5||
       !/[a-z]/i.test(word)||!/^[a-z0-9][a-z0-9 '-]*$/i.test(word))throw new ApiLookupError('词典建议的数据格式异常，请重试。');
    return word;
  });
  return combineSuggestions(query,words);
}
function object(value:unknown):Record<string,unknown>{
  if(!value||typeof value!=='object'||Array.isArray(value))throw new ApiLookupError('API 返回的数据格式异常，请重试。');
  return value as Record<string,unknown>;
}
function field(value:unknown,name:string,optional=false):string{
  if(optional&&value===undefined)return '';
  if(typeof value!=='string')throw new ApiLookupError(`API 的 ${name} 字段格式异常，请重试。`);
  return value;
}
function array(value:unknown,name:string):unknown[]{
  if(!Array.isArray(value))throw new ApiLookupError(`API 缺少有效的 ${name} 数组，请重试。`);
  return value;
}
function id(value:unknown):number|undefined{
  if(value===undefined)return undefined;
  if(typeof value!=='number'||!Number.isInteger(value)||value<0)throw new ApiLookupError('API 的序号字段格式异常，请重试。');
  return value;
}
function normalized(value:string){return value.replace(/\s+/g,' ').trim();}
function safeURL(value:string){try{const url=new URL(value);return ['https:','http:'].includes(url.protocol)?url.href:'';}catch{return '';}}
function readEntry(raw:unknown):Entry{
  const data=object(raw),warnings:string[]=[];
  const word=field(data.word,'word');if(!word.trim())throw new ApiLookupError('API 未返回词条名称，请重试。');
  const pos=array(data.pos,'pos').map(value=>field(value,'pos'));
  const definitions=array(data.definition,'definition').map((value,i)=>{
    const d=object(value),meaning=field(d.text,'definition.text'),translation=field(d.translation,'definition.translation');
    if(!meaning.trim())warnings.push(`第 ${i+1} 条释义未提供英文解释。`);
    if(!translation.trim())warnings.push(`第 ${i+1} 条释义未提供中文翻译。`);
    const examples=array(d.example,'definition.example').map(value=>{
      const e=object(value);return {id:id(e.id),text:field(e.text,'example.text'),translation:field(e.translation,'example.translation')};
    });
    return {id:id(d.id),pos:field(d.pos,'definition.pos'),source:field(d.source,'definition.source',true),text:meaning,translation,examples};
  });
  const pronunciation=array(data.pronunciation,'pronunciation').map(value=>{
    const p=object(value);return {pos:field(p.pos,'pronunciation.pos'),lang:field(p.lang,'pronunciation.lang'),pron:field(p.pron,'pronunciation.pron'),url:safeURL(field(p.url,'pronunciation.url'))};
  });
  return {word,pos,definitions,pronunciation,warnings};
}
function readApiEntry(payload:LookupPayload,query:string,language:'en-cn'|'en'='en-cn'):Entry{
  const typedError=payload.apiStatus!==undefined||payload.apiCode!==undefined;
  if(payload.apiError||typedError){
    const notFound=typedError?payload.apiStatus===404&&payload.apiCode==='WORD_NOT_FOUND':Boolean(payload.apiError?.startsWith('词典服务返回 HTTP 404'));
    throw new ApiLookupError('词典 API 查询未完成：'+(payload.apiError||'错误响应缺少说明'),notFound,notFound&&typedError?readSuggestions(payload.suggestions,query):[],payload.apiCode||'LOOKUP_FAILED',payload.apiStatus);
  }
  const data=object(payload.entry);
  if(typeof data.error==='string'){
    const notFound=data.error.trim().toLowerCase()==='word not found'&&(data.code===undefined||data.code==='WORD_NOT_FOUND');
    throw new ApiLookupError('词典 API 返回：'+data.error,notFound,[],typeof data.code==='string'?data.code:'LOOKUP_FAILED');
  }
  const entry=readEntry(data);entry.query=query;entry.language=language==='en'?'en':undefined;entry.sourceUrl=wordURL(entry.word,language);
  if(language==='en')entry.warnings=entry.warnings.filter(w=>!w.includes('中文翻译'));
  if(identity(entry.word)!==identity(query))entry.warnings.unshift(`查询「${query}」，接口实际返回「${entry.word}」；以下显示接口返回的完整词条。`);
  if(language==='en-cn'){
    const missing=entry.definitions.reduce((n,d)=>n+d.examples.filter(e=>!e.translation.trim()).length,0);
    if(missing)entry.warnings.push(`接口有 ${missing} 条例句未提供中文翻译。`);
  }
  return entry;
}
function wordURL(word:string,language:'en-cn'|'en'='en-cn'){return `https://dictionary.cambridge.org/dictionary/${language==='en'?'english':'english-chinese-simplified'}/${encodeURIComponent(word)}`;}
function normalizeSelection(raw:string){
  const value=raw.trim();
  if(Array.from(value).length>100||/[\u0000-\u001f\u007f-\u009f]/.test(value))throw new SelectionInputError('请选择单行英文单词或短语，不要选择日志、路径或命令。');
  const edges="`\"'.,;:!?()[]{}“”‘’，。；：！？（）【】《》〈〉「」『』…·•-‐‑‒–—―−﹘﹣－_=+*|#$%^&<>";
  const chars=Array.from(value);let start=0,end=chars.length;
  const isEdge=(char:string)=>/\s/.test(char)||edges.includes(char);
  while(start<end&&isEdge(chars[start]))start++;
  while(end>start&&isEdge(chars[end-1]))end--;
  const word=chars.slice(start,end).join('').replace(/[‘’]/g,"'").replace(/[\u2010\u2011]/g,'-').replace(/\s+/g,' ');
  if(!word||word.length>80||word.split(' ').length>5||!/[a-z]/i.test(word)||!/^[a-z0-9][a-z0-9 '-]*$/i.test(word))throw new SelectionInputError('请选择不超过 80 字符的英文单词或短语，不能包含多行、路径或命令。');
  return word;
}
function identity(value:string){const key=value.toLowerCase().replace(/ +/g,' ').trim().replace(/’/g,"'");return key==='runtime'?'run-time':key;}
