// Product copy is chosen from stable states. Raw API/OS messages stay diagnostic.
interface LookupNotice { title:string; message:string; action:'correct'|'retry' }
function lookupNotice(error:unknown):LookupNotice {
  if(error instanceof SelectionInputError)return {title:'请换一个查询词',message:'输入一个英文单词或简短短语。',action:'correct'};
  if(error instanceof ApiLookupError){
    if(error.notFound)return {title:'未找到这个词',message:'试试下面的建议，或换一个拼写。',action:'correct'};
    if(error.code==='INVALID_RESPONSE'||error.code==='INVALID_CAMBRIDGE_PAGE')return {title:'释义暂时不可用',message:'稍后再试，或查看原词典。',action:'retry'};
    if(error.code==='CAMBRIDGE_TIMEOUT'||error.status===504)return {title:'查询有些慢',message:'稍后再试。',action:'retry'};
    if(error.code==='CAMBRIDGE_BLOCKED')return {title:'词典暂时无法查询',message:'稍后再试。',action:'retry'};
    if(error.code==='CAMBRIDGE_UNAVAILABLE')return {title:'暂时无法连接词典',message:'检查网络连接后再试。',action:'retry'};
    if(error.code==='INVALID_ENTRY'||error.code==='UNSUPPORTED_LANGUAGE'||error.status===400)return {title:'请换一个查询词',message:'输入一个英文单词或简短短语。',action:'correct'};
  }
  if(error instanceof Error&&error.name==='AbortError')return {title:'查询有些慢',message:'稍后再试。',action:'retry'};
  return {title:'查询暂未完成',message:'稍后再试。',action:'retry'};
}
function captureNotice(code:string):{title:string;message:string} {
  const notices:Record<string,{title:string;message:string}>={
    no_selection:{title:'没有读到选中文字',message:'重新选中单词后再试，或在下方输入。'},
    unreported_selection:{title:'这里暂时无法取词',message:'重新选中后再试，或在下方输入。'},
    unsupported:{title:'这里暂时无法取词',message:'可以在下方直接输入要查的词。'},
    protected:{title:'此处无法取词',message:'为保护隐私，不读取受保护的内容。'},
    source_changed:{title:'请重新选词',message:'回到原来的窗口，选中单词后再试。'},
    invalid_selection:{title:'请选中英文单词',message:'选择一个单词或简短短语，也可以直接输入。'},
    access_denied:{title:'这里暂时无法取词',message:'可以在下方直接输入要查的词。'},
    copy_unavailable:{title:'这里暂时无法取词',message:'重新选中后再试，或在下方直接输入。'},
    clipboard_unavailable:{title:'暂时无法保留剪贴板',message:'当前内容保持不变，可以在下方直接输入。'},
    clipboard_restore_failed:{title:'剪贴板恢复未完成',message:'请检查剪贴板内容；本次没有继续查询。'},
    read_error:{title:'暂时没有读到文字',message:'重新选词后再试，或在下方输入。'},
    timeout:{title:'取词有些慢',message:'重新选词后再试，或在下方输入。'}
  };
  return Object.hasOwn(notices,code)?notices[code]:notices.read_error;
}
// Cambridge parts of speech; grammar codes such as "noun [ C ]" are dropped. Unknown values stay as given.
const POS_LABELS:Record<string,string>={noun:'名词',verb:'动词',adjective:'形容词',adverb:'副词',preposition:'介词',conjunction:'连词',pronoun:'代词',determiner:'限定词',predeterminer:'前位限定词',exclamation:'感叹词',interjection:'感叹词',number:'数词','ordinal number':'序数词',prefix:'前缀',suffix:'后缀','auxiliary verb':'助动词','modal verb':'情态动词','phrasal verb':'短语动词',idiom:'习语',phrase:'短语',abbreviation:'缩写','combining form':'构词成分','plural noun':'复数名词',symbol:'符号',article:'冠词'};
function posLabel(pos:string):{zh:string;en:string} {
  const en=pos.replace(/\[[^\]]*\]/g,' ').replace(/\s+/g,' ').trim().toLowerCase();
  return {zh:Object.hasOwn(POS_LABELS,en)?POS_LABELS[en]:en,en};
}
interface PronunciationChip { lang:string; pron:string; url:string; pos:string }
// One chip per distinct sound. Identical UK/US readings listed under several parts of speech
// collapse into one; differing readings keep their part of speech so none is lost.
function pronunciationChips(list:Pronunciation[]):PronunciationChip[] {
  const lang=(value:string)=>({uk:'英',us:'美'} as Record<string,string>)[value.toLowerCase()]||value.toUpperCase();
  const chips:PronunciationChip[]=[],seen=new Map<string,PronunciationChip>(),parts=new Map<PronunciationChip,Set<string>>();
  for(const item of list){
    if(!item.pron.trim())continue;
    const key=item.lang.toLowerCase()+'\u0000'+item.pron.trim(),pos=posLabel(item.pos).zh;
    let chip=seen.get(key);
    if(!chip){chip={lang:lang(item.lang),pron:item.pron.trim(),url:item.url,pos:''};seen.set(key,chip);parts.set(chip,new Set());chips.push(chip);}
    else if(!chip.url&&item.url)chip.url=item.url;
    if(pos)parts.get(chip)!.add(pos);
  }
  const variants=new Map<string,number>();for(const chip of chips)variants.set(chip.lang,(variants.get(chip.lang)||0)+1);
  for(const chip of chips)if(variants.get(chip.lang)!>1)chip.pos=[...parts.get(chip)!].join('/');
  return chips;
}
// Cambridge marks senses from its other dictionaries with a dataset id; unknown ids stay as given.
function sourceLabel(id:string):string {
  const names:Record<string,string>={cald4:'剑桥高阶英语',cacd:'剑桥美式英语',cbed:'剑桥商务英语'};
  return Object.hasOwn(names,id.toLowerCase())?names[id.toLowerCase()]:id;
}
function shortcutNotice(error:unknown):string {
  const value=typeof error==='string'?error:'';
  if(value.includes('占用'))return '这组快捷键已被占用，请换一组。';
  if(value.includes('未响应')||value.includes('重新启动'))return '快捷键暂不可用，请重启 WordSide。';
  if(['修饰键重复','只能包含','请使用 Ctrl','需包含 Ctrl'].some(part=>value.includes(part)))return '请使用 Ctrl 或 Alt 搭配一个字母、数字或功能键。';
  return '快捷键未能保存，请稍后再试。';
}
