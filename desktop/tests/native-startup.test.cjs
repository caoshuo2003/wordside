const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const ts=require('typescript');
const source=fs.readFileSync(require.resolve('../src/native.ts'),'utf8');
const script=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.None}}).outputText;
const flush=()=>new Promise(resolve=>setImmediate(resolve));

function fixture({popup=true,failSettings=0,failReady=0,failListener='',pendingListener='',frameDrawn=true}={}){
  const frameCalls=[],bodyClasses=new Set(),observers=[],mediaListeners=[];
  let styleVars={'--card-line':'#e0e7df','--card':'#fbfdfb','--line':'#d9e3db','--bg':'#f2f5f2','--text':'#293930'};
  const elements=new Map(),listeners=new Map(),windowEvents=new Map(),timers=new Map(),lookupCalls=[],registered=[];
  let timerId=0,settingsCalls=0,readyCalls=0,failedListener=false,pendingResolve;
  const readySnapshots=[];
  function element(id){
    if(!elements.has(id))elements.set(id,{
      value:id==='close-behavior'?'tray':'',checked:true,hidden:true,disabled:false,dataset:{},
      textContent:'',title:'',events:new Map(),
      addEventListener(name,fn){this.events.set(name,fn);},
      setAttribute(){},append(){},closest(){return this;},querySelectorAll(){return [];},focus(){}
    });
    return elements.get(id);
  }
  const enabled=element('enabled');
  let current={version:'0.1.3',settings_revision:0,settings:{enabled:false,theme:'system',palette:'green',lookup_hotkey:'Ctrl+Alt+A',close_behavior:'tray'},capture:'',hotkey:{active:true,shortcut:'Ctrl+Alt+A'}};
  const api={
    core:{async invoke(command,args){
      if(command==='set_window_frame'){frameCalls.push({...args});return frameDrawn;}
      if(command==='get_settings'){
        settingsCalls++;
        if(settingsCalls<=failSettings)throw new Error('temporary state unavailable');
        return current;
      }
      if(command==='popup_ready'){
        readyCalls++;
        readySnapshots.push({enabled:enabled.checked,listeners:[...listeners.values()].reduce((sum,set)=>sum+set.size,0)});
        if(readyCalls<=failReady)throw new Error('temporary ready failure');
      }
    }},
    event:{async listen(name,callback){
      if(name===failListener&&!failedListener){failedListener=true;throw new Error('temporary event failure');}
      if(name===pendingListener)await new Promise(resolve=>{pendingResolve=resolve;});
      if(!listeners.has(name))listeners.set(name,new Set());
      listeners.get(name).add(callback);
      registered.push({name,callback});
      return ()=>listeners.get(name).delete(callback);
    }}
  };
  const context={
    nativeAPI:api,isPopup:popup,get:element,enabled,
    theme:element('theme'),palette:element('palette'),pin:element('pin'),card:element('card'),
    pinned:false,manualLookup:false,selectedPalette:'green',
    document:{body:{classList:{add(name){bodyClasses.add(name);},contains(name){return bodyClasses.has(name);},toggle(name,force){if(force)bodyClasses.add(name);else bodyClasses.delete(name);}}},addEventListener(){},},
    getComputedStyle:()=>({getPropertyValue:name=>styleVars[name]??''}),
    MutationObserver:class{constructor(callback){observers.push(callback);}observe(){}},
    matchMedia:()=>({matches:false,addEventListener(name,callback){mediaListeners.push(callback);}}),
    window:{addEventListener(name,fn){windowEvents.set(name,fn);}},
    updateStatus(){},prepareCapture(){},closeCard(){},applyTheme(){},applyPalette(){},remember(){},
    shortcutNotice:value=>String(value),lookup(...args){lookupCalls.push(args);},showCaptureFailure(){},
    setTimeout(fn,delay){const id=++timerId;timers.set(id,{fn,delay});return id;},
    clearTimeout(id){timers.delete(id);},
  };
  vm.runInNewContext(script,context);
  return {
    enabled,element,readySnapshots,frameCalls,bodyClasses,
    restyle(vars,dark){styleVars={...styleVars,...vars};if(dark)bodyClasses.add('dark');for(const callback of observers)callback();},
    changeSystemTheme(){for(const callback of mediaListeners)callback();},listeners,timers,lookupCalls,registered,
    get settingsCalls(){return settingsCalls;},get readyCalls(){return readyCalls;},
    activeListeners(){return [...listeners.values()].reduce((sum,set)=>sum+set.size,0);},
    setSettings(value){current=value;},
    async retry(){const next=timers.entries().next().value;assert.ok(next,'initialization must schedule recovery');timers.delete(next[0]);next[1].fn();await flush();},
    unload(){windowEvents.get('beforeunload')?.();},
    resolveListener(){assert.ok(pendingResolve);pendingResolve();},
  };
}

test('fresh popup synchronizes saved defaults before announcing readiness',async()=>{
  const app=fixture();await flush();
  assert.equal(app.readyCalls,1);
  assert.deepEqual(app.readySnapshots,[{enabled:false,listeners:6}]);
  assert.equal(app.activeListeners(),6);
  assert.equal(app.timers.size,0,'healthy startup has no retry timer');
});
test('a transient settings failure recovers with the latest settings without restarting',async()=>{
  const app=fixture({failSettings:1});await flush();
  assert.equal(app.readyCalls,0);
  assert.equal(app.activeListeners(),0,'failed initialization releases partial subscriptions');
  app.setSettings({version:'0.1.3',settings_revision:1,settings:{enabled:true,theme:'dark',palette:'blue',lookup_hotkey:'Alt+D'},capture:'',hotkey:{active:true,shortcut:'Alt+D'}});
  await app.retry();
  assert.equal(app.settingsCalls,2);
  assert.equal(app.readyCalls,1);
  assert.equal(app.enabled.checked,true,'recovery must not restore stale defaults');
  assert.equal(app.activeListeners(),6);
  assert.equal(app.timers.size,0);
});
test('partial event registration is retried without duplicate capture callbacks',async()=>{
  const app=fixture({failListener:'native-selection-failed'});await flush();
  assert.equal(app.readyCalls,0);
  assert.equal(app.activeListeners(),0);
  await app.retry();
  assert.equal(app.readyCalls,1);
  assert.equal(app.activeListeners(),6);
  for(const callbacks of app.listeners.values())assert.equal(callbacks.size,1);
});
test('readiness acknowledgement failure also recovers without leaking listeners',async()=>{
  const app=fixture({failReady:1});await flush();
  assert.equal(app.readyCalls,1);
  assert.equal(app.activeListeners(),0);
  await app.retry();
  assert.equal(app.readyCalls,2);
  assert.equal(app.activeListeners(),6);
  assert.equal(app.timers.size,0);
});
test('recovery queries a captured word once and ignores callbacks from the failed attempt',async()=>{
  const app=fixture({failSettings:1});await flush();
  const stale=app.registered.find(item=>item.name==='native-selection').callback;
  await app.retry();
  const payload={word:'workspace',serial:1,manual:true};
  stale({payload});
  assert.equal(app.lookupCalls.length,0);
  for(const callback of app.listeners.get('native-selection'))callback({payload});
  assert.equal(app.lookupCalls.length,1);
  assert.equal(app.lookupCalls[0][0],'workspace');
});
test('recreated settings window recovers without announcing popup readiness',async()=>{
  const app=fixture({popup:false,failSettings:1});await flush();
  await app.retry();
  assert.equal(app.settingsCalls,2);
  assert.equal(app.readyCalls,0);
  assert.equal(app.element('native-status').textContent,'版本 0.1.3');
});
test('closing a failed window cancels recovery and releases subscriptions',async()=>{
  const app=fixture({failSettings:10});await flush();
  app.unload();
  assert.equal(app.timers.size,0);
  assert.equal(app.activeListeners(),0);
});
test('a listener that resolves after the window closes is immediately released',async()=>{
  const app=fixture({pendingListener:'native-hidden'});await flush();
  app.unload();app.resolveListener();await flush();
  assert.equal(app.activeListeners(),0);
  assert.equal(app.readyCalls,0);
  assert.equal(app.timers.size,0);
});
test('window frames follow the page palette and skip unchanged colors',async()=>{
  const popup=fixture();await flush();
  assert.deepEqual(popup.frameCalls,[{border:'#e0e7df',caption:'#fbfdfb',text:null,dark:false}]);
  assert.ok(popup.bodyClasses.has('system-frame'),'the Windows 11 border replaces the page border');
  popup.restyle({});await flush();
  assert.equal(popup.frameCalls.length,1,'class changes that keep the palette are ignored');
  popup.restyle({'--card-line':'#403c4c','--card':'#24232b'},true);await flush();
  assert.deepEqual(popup.frameCalls.at(-1),{border:'#403c4c',caption:'#24232b',text:null,dark:true});
  popup.changeSystemTheme();await flush();
  assert.equal(popup.frameCalls.length,3,'a system theme change re-applies the caption buttons');
  const main=fixture({popup:false,frameDrawn:false});await flush();
  assert.deepEqual(main.frameCalls,[{border:'#d9e3db',caption:'#f2f5f2',text:'#293930',dark:false}]);
  assert.ok(!main.bodyClasses.has('system-frame'),'older Windows keeps the page border');
});
