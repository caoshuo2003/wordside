if(nativeAPI){
  document.body.classList.add(isPopup?'native-popup':'native-main');
  if(!isPopup)get('native-capture-toggle').append(enabled.closest('label')!);
  const note=get('native-status');
  const hotkeyInput=get<HTMLInputElement>('lookup-hotkey');
  const hotkeyStatus=get('shortcut-status');const hotkeyBadge=get('shortcut-badge');const record=get<HTMLButtonElement>('record-hotkey');
  let recording=false, captureSerial=0, captureActive=false;
  const closeBehavior=get<HTMLSelectElement>('close-behavior');const closeStatus=get('close-status');
  let closePending=false,closeConfirmed=false,confirmedCloseBehavior=closeBehavior.value,lastSettingsRevision=-1;
  let appearancePending=0;
  let appearanceQueue:Promise<void>=Promise.resolve();
  function hotkeyMessage(message:string,error=false){hotkeyStatus.textContent=message;hotkeyStatus.hidden=!message;hotkeyStatus.dataset.state=error?'error':'ok';}
  function stopRecording(){recording=false;record.textContent='录入按键';record.setAttribute('aria-pressed','false');}
  function acceptsCapture(payload:{serial?:number}){
    if(payload.serial===undefined)return true;
    return payload.serial>=captureSerial&&(payload.serial>captureSerial||captureActive);
  }
  function applyNative(info:NativeInfo){
    if(info.settings_revision!==undefined){
      if(info.settings_revision<lastSettingsRevision)return;
      lastSettingsRevision=info.settings_revision;
    }
    const wasEnabled=enabled.checked;enabled.checked=info.settings.enabled;
    if(wasEnabled&&!enabled.checked&&!manualLookup){prepareCapture();if(!pinned)card.hidden=true;}
    if(!appearancePending){
      theme.value=['light','dark','system'].includes(info.settings.theme)?info.settings.theme:'system';applyTheme();
      applyPalette(info.settings.palette||'green');remember('theme',theme.value);remember('palette',selectedPalette);
    }
    updateStatus();
    note.textContent=info.version?'版本 '+info.version:'';
    confirmedCloseBehavior=info.settings.close_behavior==='exit'?'exit':'tray';closeConfirmed=true;
    if(!closePending){closeBehavior.value=confirmedCloseBehavior;closeBehavior.disabled=false;}
    if(!recording)hotkeyInput.value=info.settings.lookup_hotkey??'Ctrl+Alt+A';
    const status=info.hotkey;hotkeyBadge.dataset.state=status?.error?'error':status?.active?'active':'inactive';
    hotkeyBadge.textContent=status?.error?'需要调整':status?.active?'已启用':'未启用';
    if(!recording)hotkeyMessage((status?.error?shortcutNotice(status.error):'')||(status?.active||!hotkeyInput.value?'':'正在启用快捷键…'),Boolean(status?.error));

  }
  record.addEventListener('click',()=>{
    if(recording){stopRecording();hotkeyMessage('');return;}
    recording=true;record.textContent='取消录入';record.setAttribute('aria-pressed','true');hotkeyMessage('按下组合键，Esc 取消。');hotkeyInput.focus();
  });
  document.addEventListener('keydown',event=>{
    if(!recording)return;event.preventDefault();event.stopImmediatePropagation();
    if(event.key==='Escape'){stopRecording();hotkeyMessage('已取消录入。');return;}
    if(['Control','Alt','Shift','Meta'].includes(event.key))return;
    const key=event.code.startsWith('Key')?event.code.slice(3):event.code.startsWith('Digit')?event.code.slice(5):event.key.toUpperCase();
    if(event.metaKey||(!event.ctrlKey&&!event.altKey)||!/^([A-Z0-9]|F([1-9]|1[0-9]|2[0-4]))$/.test(key)||key==='F12'){
      hotkeyMessage('请使用 Ctrl 或 Alt 搭配字母、数字或功能键（F12 除外）。',true);return;
    }
    hotkeyInput.value=[event.ctrlKey?'Ctrl':'',event.altKey?'Alt':'',event.shiftKey?'Shift':'',key].filter(Boolean).join('+');stopRecording();hotkeyMessage('保存后生效。');
  },true);
  get('save-hotkey').addEventListener('click',()=>{
    stopRecording();const button=get<HTMLButtonElement>('save-hotkey');button.disabled=true;
    void nativeAPI!.core.invoke<NativeInfo>('save_lookup_hotkey',{hotkey:hotkeyInput.value}).then(info=>{applyNative(info);}).catch(async error=>{
      try{applyNative(await nativeAPI!.core.invoke<NativeInfo>('get_settings'));}catch{}
      hotkeyMessage(shortcutNotice(error),true);
    }).finally(()=>{button.disabled=false;});
  });
  enabled.addEventListener('change',()=>{
    void nativeAPI!.core.invoke<NativeInfo>('set_enabled',{enabled:enabled.checked}).then(applyNative).catch(async()=>{try{applyNative(await nativeAPI!.core.invoke<NativeInfo>('get_settings'));}catch{}note.textContent='自动划词设置未保存，请稍后再试。';});
  });
  function saveAppearance(){
    // Snapshot each choice and serialize saves so delayed responses cannot undo a newer choice.
    const preference={theme:theme.value,palette:selectedPalette};appearancePending++;
    appearanceQueue=appearanceQueue.then(async()=>{
      let result:NativeInfo|undefined,error:unknown;
      try{result=await nativeAPI!.core.invoke<NativeInfo>('save_appearance',preference);}
      catch(failure){error=failure;try{result=await nativeAPI!.core.invoke<NativeInfo>('get_settings');}catch{}}
      appearancePending--;
      if(result)applyNative(result);
      if(error&&!appearancePending)note.textContent='外观设置未保存，请稍后再试。';
    });
    return appearanceQueue;
  }
  theme.addEventListener('change',()=>void saveAppearance());
  palette.addEventListener('change',()=>void saveAppearance());
  closeBehavior.addEventListener('change',()=>{
    if(closePending||isPopup||!closeConfirmed)return;closePending=true;closeBehavior.disabled=true;closeStatus.hidden=true;
    void nativeAPI!.core.invoke<NativeInfo>('save_close_behavior',{behavior:closeBehavior.value}).then(info=>{
      closePending=false;applyNative(info);closeBehavior.value=confirmedCloseBehavior;
    }).catch(async()=>{
      closePending=false;try{applyNative(await nativeAPI!.core.invoke<NativeInfo>('get_settings'));}catch{}
      closeBehavior.value=confirmedCloseBehavior;closeStatus.textContent='关闭方式未保存，请稍后再试。';closeStatus.hidden=false;
    }).finally(()=>{closePending=false;closeBehavior.disabled=!closeConfirmed;});
  });
  if(isPopup){
    const drag=get('popup-drag-area');
    drag.addEventListener('pointerdown',event=>{
      if(event.button!==0||!nativeAPI!.window)return;event.preventDefault();
      void nativeAPI!.window.getCurrentWindow().startDragging().catch(()=>{drag.title='暂时无法拖动，请重试';});
    });
    for(const edge of card.querySelectorAll<HTMLElement>('[data-resize]'))edge.addEventListener('pointerdown',event=>{
      if(event.button!==0||!nativeAPI!.window)return;event.preventDefault();event.stopPropagation();
      void nativeAPI!.window.getCurrentWindow().startResizeDragging(edge.dataset.resize!).catch(()=>{edge.title='暂时无法调整大小，请重试';});
    });
  }
  pin.addEventListener('click',()=>{if(isPopup)void nativeAPI!.core.invoke('set_pinned',{pinned});});
  document.addEventListener('click',event=>{
    const anchor=(event.target as Element).closest<HTMLAnchorElement>('a[href]');
    if(anchor?.href.startsWith('https://dictionary.cambridge.org/')){event.preventDefault();void nativeAPI!.core.invoke('open_source',{url:anchor.href}).catch(()=>{if(isPopup)get('source-link').textContent='链接暂不可用';else note.textContent='暂时无法打开原词典。';});}
  });
  void (async()=>{
    await nativeAPI!.event.listen<NativeInfo>('native-state',event=>applyNative(event.payload));
    await nativeAPI!.event.listen('native-hidden',()=>{captureActive=false;closeCard(true);});
    await nativeAPI!.event.listen<{serial:number;manual:boolean}>('native-capture-started',event=>{
      if(!isPopup||(!event.payload.manual&&!enabled.checked)||event.payload.serial<=captureSerial)return;captureSerial=event.payload.serial;captureActive=true;manualLookup=event.payload.manual;prepareCapture();
    });
    await nativeAPI!.event.listen<{word:string;serial?:number;manual?:boolean}>('native-selection',event=>{
      if(!isPopup||(!event.payload.manual&&!enabled.checked)||!acceptsCapture(event.payload))return;
      if(event.payload.serial!==undefined)captureSerial=event.payload.serial;captureActive=true;manualLookup=Boolean(event.payload.manual);void lookup(event.payload.word,0,0);
    });
    await nativeAPI!.event.listen<CaptureFailure>('native-selection-failed',event=>{
      if(!isPopup||!acceptsCapture(event.payload))return;if(event.payload.serial!==undefined)captureSerial=event.payload.serial;captureActive=true;showCaptureFailure(event.payload);
    });
    await nativeAPI!.event.listen<{code?:string}>('native-capture-feedback',event=>{note.textContent=event.payload.code==='select_first'?'先在其他应用选中单词，再按快捷键。':event.payload.code==='popup_size_save_failed'?'窗口大小未保存，请稍后重试。':'自动取词暂不可用，手动查词仍可使用。';});
    applyNative(await nativeAPI!.core.invoke<NativeInfo>('get_settings'));
    if(isPopup)await nativeAPI!.core.invoke('popup_ready');
  })().catch(()=>{note.textContent='WordSide 暂未就绪，请重新打开。';});
}
