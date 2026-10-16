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
function shortcutNotice(error:unknown):string {
  const value=typeof error==='string'?error:'';
  if(value.includes('占用'))return '这组快捷键已被占用，请换一组。';
  if(value.includes('未响应')||value.includes('重新启动'))return '快捷键暂不可用，请重启 WordSide。';
  if(['修饰键重复','只能包含','请使用 Ctrl','需包含 Ctrl'].some(part=>value.includes(part)))return '请使用 Ctrl 或 Alt 搭配一个字母、数字或功能键。';
  return '快捷键未能保存，请稍后再试。';
}
