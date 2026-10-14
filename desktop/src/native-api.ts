interface NativeInfo { version?:string; settings_revision?:number; settings: {enabled:boolean; theme:string; palette?:string; lookup_hotkey?:string;close_behavior?:'tray'|'exit'}; capture:string; hotkey?:{active:boolean; shortcut:string; error?:string|null} }
interface NativeAPI {
  core:{invoke:<T>(command:string,args?:Record<string,unknown>)=>Promise<T>};
  window?:{getCurrentWindow:()=>{startDragging:()=>Promise<void>;startResizeDragging:(direction:string)=>Promise<void>}};
  event:{listen:<T>(event:string,callback:(event:{payload:T})=>void)=>Promise<()=>void>};
}
const nativeAPI=(window as unknown as {__TAURI__?:NativeAPI}).__TAURI__;
const isPopup=Boolean(nativeAPI && new URLSearchParams(location.search).has('popup'));
