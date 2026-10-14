#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod selection_input;
mod popup_geometry;
mod window_lifecycle;
mod capture;
mod dismissal;
mod cache;
mod dictionary;
mod lookup;
use lookup::fetch_payload;
use std::{collections::HashMap,sync::{Arc,Weak}};
use std::{fs, io::Write, path::PathBuf, sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Mutex}, time::Duration};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, menu::{Menu, MenuItem, CheckMenuItem}, tray::TrayIconBuilder};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Settings { enabled: bool, theme: String, palette:String, lookup_hotkey:String, #[serde(default,deserialize_with="popup_geometry::deserialize_size")] popup_size:popup_geometry::PopupSize, #[serde(default,deserialize_with="window_lifecycle::deserialize_behavior")] close_behavior:window_lifecycle::CloseBehavior }
impl Default for Settings {
    fn default() -> Self { Self { enabled: false, theme: "system".into(), palette:"green".into(), lookup_hotkey:capture::shortcut::DEFAULT_HOTKEY.into(), popup_size:popup_geometry::PopupSize::default(), close_behavior:window_lifecycle::CloseBehavior::default() } }
}
fn decode_settings(bytes:&[u8])->Settings {
    // Obsolete api_base / copy_lookup / scope / process-list fields are ignored. Preserve the user's
    // switch and other preferences, including after saving the new schema.
    let mut settings:Settings=serde_json::from_slice(bytes).unwrap_or_default();
    if !valid_palette(&settings.palette){settings.palette="green".into();}
    settings
}

fn valid_palette(value:&str)->bool{["green","lavender","blue","stone"].contains(&value)}

struct State {
    settings: Mutex<Settings>, settings_revision:AtomicU64, config: PathBuf, popup_geometry:Mutex<popup_geometry::GeometryState>,
    #[cfg(windows)]hotkeys:Arc<capture::hotkey::Control>,
    hotkey_save:Mutex<()>,
    enabled: AtomicBool, pinned: AtomicBool, ready: AtomicBool,
    lookup_serial:AtomicU64, main_epoch: AtomicU64, popup_epoch: AtomicU64, capture_epoch: AtomicU64, capture_serial:AtomicU64,
    dictionary: dictionary::Backend,
    cache:Mutex<cache::Cache>, gates:Mutex<HashMap<String,Weak<tokio::sync::Mutex<()>>>>,
}
#[derive(Clone, Serialize)]
struct Info { settings: Settings, settings_revision:u64, capture: &'static str, version: &'static str, hotkey:capture::shortcut::HotkeyStatus }
fn info(state: &State) -> Info {
    #[cfg(windows)]let hotkey=state.hotkeys.status();
    #[cfg(not(windows))]let hotkey=capture::shortcut::HotkeyStatus{error:Some("此平台暂未接入全局快捷键。".into()),..Default::default()};
    // Read the preference snapshot and its revision under the same lock.
    let stored=state.settings.lock().unwrap();let settings=stored.clone();let settings_revision=state.settings_revision.load(Ordering::SeqCst);drop(stored);
    Info {hotkey,settings,settings_revision, version: env!("CARGO_PKG_VERSION"), capture: if cfg!(windows) { "Windows 选区 · 独立快捷键 / 自动划词（含复制兼容）" } else { "此平台暂不支持自动取词，可使用手动查询" } }
}
fn persist(state: &State, value: &Settings) -> Result<(), String> {
    persist_settings(&state.config,value)
}
fn persist_settings(config:&std::path::Path,value:&Settings)->Result<(),String>{
    let folder=config.parent().ok_or("设置目录不可用")?;
    fs::create_dir_all(folder).map_err(|e|e.to_string())?;
    let mut file=tempfile::NamedTempFile::new_in(folder).map_err(|e|e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    file.as_file().sync_all().map_err(|e|e.to_string())?;
    file.persist(config).map_err(|e|e.to_string())?;
    Ok(())
}
// Observe native sizes, not CSS card sizes. Save once after a resize burst.
fn popup_resized(window:&tauri::Window,size:tauri::PhysicalSize<u32>) {
    let Some(state)=window.try_state::<State>() else{return;};
    let scale=window.scale_factor().unwrap_or(1.0);
    let next=state.popup_geometry.lock().unwrap().observed(size.width,size.height,scale);
    let Some(next)=next else{return;};
    {let mut settings=state.settings.lock().unwrap();if settings.popup_size==next{return;}settings.popup_size=next;state.settings_revision.fetch_add(1,Ordering::SeqCst);}
    let token=state.popup_geometry.lock().unwrap().queue_save();
    if let Some(mut token)=token {
        let app=window.app_handle().clone();
        tauri::async_runtime::spawn(async move {loop {
            tokio::time::sleep(Duration::from_millis(350)).await;
            let state=app.state::<State>();
            let readiness=state.popup_geometry.lock().unwrap().ready(token);
            match readiness {Err(newer)=>{token=newer;continue;},Ok(())=>{}}
            let result=persist(&state,&state.settings.lock().unwrap());
            if result.is_ok(){state.popup_geometry.lock().unwrap().saved(token);}
            else if result.is_err(){let _=app.emit_to("main","native-capture-feedback",serde_json::json!({"code":"popup_size_save_failed"}));}
            break;
        }});
    }
}
fn flush_popup_size(state:&State) {
    if state.popup_geometry.lock().unwrap().dirty {let _=persist(state,&state.settings.lock().unwrap());}
}
fn epoch<'a>(state: &'a State, label: &str) -> &'a AtomicU64 { if label=="popup" { &state.popup_epoch } else { &state.main_epoch } }
fn apply_enabled(app: &tauri::AppHandle, value: bool) -> Result<Info,String> {
    let state=app.state::<State>();
    if value && !cfg!(windows) { return Err("当前平台尚未接入自动取词；可以手动查询。".into()); }
    let mut settings=state.settings.lock().unwrap();
    let mut next=settings.clone();next.enabled=value;persist(&state,&next)?;*settings=next;state.settings_revision.fetch_add(1,Ordering::SeqCst);drop(settings);
    state.enabled.store(value,Ordering::SeqCst);
    state.capture_epoch.fetch_add(1,Ordering::SeqCst);
    state.popup_epoch.fetch_add(1,Ordering::SeqCst);
    if !value && !state.pinned.load(Ordering::SeqCst) { let handle=app.clone();let _=app.run_on_main_thread(move||{if let Some(w)=handle.get_webview_window("popup"){capture::hide(&w);}}); }
    if let Some(tray)=app.tray_by_id("wordside") { let _=tray.set_tooltip(Some(if value { "WordSide · 自动划词已开启" } else { "WordSide · 自动划词已关闭" })); }
    let result=info(&state);let _=app.emit("native-state",&result);Ok(result)
}
#[tauri::command]
fn get_settings(state: tauri::State<State>) -> Info { info(&state) }
#[tauri::command]
fn set_enabled(app: tauri::AppHandle, enabled: bool) -> Result<Info,String> { apply_enabled(&app,enabled) }
#[tauri::command]
fn save_appearance(app:tauri::AppHandle,theme:String,palette:String)->Result<Info,String>{
    if !["system","dark","light"].contains(&theme.as_str()){return Err("无效主题".into());}
    if !valid_palette(&palette){return Err("无效配色".into());}
    let state=app.state::<State>();let mut settings=state.settings.lock().unwrap();let mut next=settings.clone();
    next.theme=theme;next.palette=palette;persist(&state,&next)?;*settings=next;state.settings_revision.fetch_add(1,Ordering::SeqCst);drop(settings);
    let result=info(&state);let _=app.emit("native-state",&result);Ok(result)
}
#[tauri::command]
fn save_close_behavior(app:tauri::AppHandle,behavior:String)->Result<Info,String>{
    let behavior=window_lifecycle::CloseBehavior::parse(&behavior).ok_or("请选择关闭方式")?;
    let state=app.state::<State>();let mut settings=state.settings.lock().unwrap();let mut next=settings.clone();
    next.close_behavior=behavior;persist(&state,&next)?;*settings=next;state.settings_revision.fetch_add(1,Ordering::SeqCst);drop(settings);
    let result=info(&state);let _=app.emit("native-state",&result);Ok(result)
}
#[tauri::command]
async fn save_lookup_hotkey(app:tauri::AppHandle,hotkey:String)->Result<Info,String>{
 let parsed=capture::shortcut::parse(&hotkey)?;
 if !cfg!(windows){return Err("此平台暂未接入全局快捷键，可在 WordSide 中手动输入。".into());}
 tauri::async_runtime::spawn_blocking(move||{
  let state=app.state::<State>();let _transaction=state.hotkey_save.lock().unwrap();
  let value=parsed.as_ref().map(|key|key.text.clone()).unwrap_or_default();
  #[cfg(windows)]let registration=state.hotkeys.prepare(parsed)?;
  let mut settings=state.settings.lock().unwrap();let mut next=settings.clone();next.lookup_hotkey=value;
  persist(&state,&next)?;*settings=next;state.settings_revision.fetch_add(1,Ordering::SeqCst);drop(settings);
  #[cfg(windows)]registration.commit()?;
  state.capture_epoch.fetch_add(1,Ordering::SeqCst);
  let result=info(&state);let _=app.emit("native-state",&result);Ok(result)
 }).await.map_err(|_|"快捷键更新任务未完成。".to_string())?
}
#[tauri::command]
fn cancel_lookup(window: tauri::WebviewWindow, state: tauri::State<State>) { epoch(&state,window.label()).fetch_add(1,Ordering::SeqCst); }
#[tauri::command]
fn popup_ready(window: tauri::WebviewWindow,state: tauri::State<State>) { if window.label()=="popup" {state.ready.store(true,Ordering::SeqCst);} }
#[tauri::command]
fn set_pinned(state: tauri::State<State>, pinned: bool) {state.pinned.store(pinned,Ordering::SeqCst);}
#[tauri::command]
fn hide_popup(app: tauri::AppHandle) {
    let state=app.state::<State>();state.capture_epoch.fetch_add(1,Ordering::SeqCst);state.popup_epoch.fetch_add(1,Ordering::SeqCst);state.pinned.store(false,Ordering::SeqCst);
    let handle=app.clone();let _=app.run_on_main_thread(move||{
        if let Some(w)=handle.get_webview_window("popup"){capture::hide(&w);}
        let _=handle.emit_to("popup","native-hidden",());
    });
}
#[tauri::command]
fn open_source(url: String) -> Result<(),String> {
    let parsed=reqwest::Url::parse(&url).map_err(|_|"无效链接")?;
    if parsed.scheme()!="https" || parsed.host_str()!=Some("dictionary.cambridge.org") {return Err("只允许打开 Cambridge 词典链接".into());}
    open::that(parsed.as_str()).map_err(|e|e.to_string())
}
#[tauri::command]
fn settle_lookup(state:tauri::State<State>,ticket:u64,accepted:bool){state.cache.lock().unwrap().settle(ticket,accepted);}
#[tauri::command]
async fn lookup(window:tauri::WebviewWindow,state:tauri::State<'_,State>,word:String,refresh:Option<bool>,language:Option<String>)->Result<serde_json::Value,String>{
    let word=capture::selected_word(&word).ok_or("请选择不超过 80 字符的单行英文单词或短语")?;
    let language=language.as_deref().unwrap_or("en-cn");
    if !["en-cn","en"].contains(&language){return Err("无效词库".into());}
    let label=window.label().to_string();
    let counter=epoch(&state,&label);let token=counter.fetch_add(1,Ordering::SeqCst)+1;
    let key=serde_json::to_string(&(word.to_lowercase(),language)).unwrap();
    let gate={let mut gates=state.gates.lock().unwrap();gates.retain(|_,g|g.strong_count()>0);if let Some(g)=gates.get(&key).and_then(Weak::upgrade){g}else{let g=Arc::new(tokio::sync::Mutex::new(()));gates.insert(key.clone(),Arc::downgrade(&g));g}};
    let job=async {
        let _guard=gate.lock().await;
        if !refresh.unwrap_or(false){if let Some(mut cached)=state.cache.lock().unwrap().get(&key){cached["cacheHit"]=serde_json::json!(true);return Ok(cached);}}
        let mut payload=fetch_payload(&state.dictionary,&word,language).await?;
        // Approve a complete API JSON payload only after frontend validation.
        // Receipts stay small and stale responses cannot alter newer cache data.
        if payload["entry"].is_object(){
            let ticket=state.lookup_serial.fetch_add(1,Ordering::SeqCst)+1;
            payload["cacheTicket"]=ticket.into();state.cache.lock().unwrap().insert_pending(key,payload.clone(),ticket);
        }
        Ok(payload)
    };
    tokio::select!{
        result=job => {if counter.load(Ordering::SeqCst)!=token {Err("查询已取消".into())}else{result}},
        _=async {loop {tokio::time::sleep(Duration::from_millis(30)).await;if counter.load(Ordering::SeqCst)!=token{break;}}} => Err("查询已取消".into()),
    }
}
fn show_main(app:&tauri::AppHandle){
    if let Some(w)=app.get_webview_window("main"){let _=w.show();let _=w.set_focus();return;}
    if let Some(config)=app.config().app.windows.iter().find(|c|c.label=="main"){
        if let Ok(builder)=tauri::WebviewWindowBuilder::from_config(app,config){if let Ok(w)=builder.build(){let _=w.show();let _=w.set_focus();}}
    }
}
fn main(){
    #[cfg(windows)]if std::env::args().nth(1).as_deref()==Some("--wordside-dictionary-licenses"){print!("{}",include_str!("../generated/THIRD-PARTY-LICENSES.txt"));return;}
    if dictionary_probe_entry(){return;}
    #[cfg(windows)]if capture::probe::helper_entry(){return;}
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app,_,_|show_main(app)))
        .invoke_handler(tauri::generate_handler![get_settings,set_enabled,save_appearance,save_close_behavior,save_lookup_hotkey,cancel_lookup,popup_ready,set_pinned,hide_popup,open_source,lookup,settle_lookup])
        .setup(|app|{
            let config=app.path().app_config_dir()?.join("settings.json");
            let mut settings=fs::read(&config).ok().map(|bytes|decode_settings(&bytes)).unwrap_or_default();
            if !cfg!(windows){settings.enabled=false;}
            let enabled=settings.enabled;
            app.manage(State{#[cfg(windows)]hotkeys:Arc::new(capture::hotkey::Control::default()),settings:Mutex::new(settings),settings_revision:AtomicU64::new(0),config,popup_geometry:Mutex::new(popup_geometry::GeometryState::default()),hotkey_save:Mutex::new(()),enabled:AtomicBool::new(enabled),pinned:AtomicBool::new(false),ready:AtomicBool::new(false),lookup_serial:AtomicU64::new(0),main_epoch:AtomicU64::new(0),popup_epoch:AtomicU64::new(0),capture_epoch:AtomicU64::new(0),capture_serial:AtomicU64::new(0),cache:Mutex::new(cache::Cache::default()),gates:Mutex::new(HashMap::new()),dictionary:dictionary::Backend::default()});
            let toggle=CheckMenuItem::with_id(app,"toggle","自动划词",true,enabled,None::<&str>)?;
            let show=MenuItem::with_id(app,"show","打开 WordSide",true,None::<&str>)?;
            let quit=MenuItem::with_id(app,"quit","退出",true,None::<&str>)?;
            let menu=Menu::with_items(app,&[&toggle,&show,&quit])?;
            let check=toggle.clone();
            TrayIconBuilder::with_id("wordside").icon(tauri::image::Image::from_bytes(include_bytes!("../icons/ciban-32.png"))?).menu(&menu).show_menu_on_left_click(false).tooltip("WordSide · 桌面划词词典")
            .on_tray_icon_event(|tray,event|{if window_lifecycle::opens_main(&event){show_main(tray.app_handle());}}).on_menu_event(move|app,event|match event.id.as_ref(){
                "toggle"=>{let value=!app.state::<State>().enabled.load(Ordering::SeqCst);if apply_enabled(app,value).is_ok(){let _=check.set_checked(value);}},
                "show"=>show_main(app),
                "quit"=>app.exit(0),_=>{}
            }).build(app)?;
            // Keep the check mark in sync when the main window changes the switch.
            let check=toggle.clone();let handle=app.handle().clone();
            use tauri::Listener;
            app.listen("native-state",move |_|{let _=check.set_checked(handle.state::<State>().enabled.load(Ordering::SeqCst));});
            capture::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window,event|{
            if window.label()=="popup" {match event {
                tauri::WindowEvent::Resized(size)=>popup_resized(window,*size),
                tauri::WindowEvent::ScaleFactorChanged{..}=>{
                    if let Some(state)=window.try_state::<State>(){state.popup_geometry.lock().unwrap().dpi_changed();}
                },
                _=>{}
            }}
            if let tauri::WindowEvent::CloseRequested{api,..}=event{
                let state=window.state::<State>();let behavior=state.settings.lock().unwrap().close_behavior;
                match window_lifecycle::close_action(window.label(),behavior){
                    window_lifecycle::CloseAction::HidePopup=>{api.prevent_close();hide_popup(window.app_handle().clone());},
                    window_lifecycle::CloseAction::ExitApplication=>{api.prevent_close();state.main_epoch.fetch_add(1,Ordering::SeqCst);window.app_handle().exit(0);},
                    // Release the settings WebView while keeping capture and the tray alive.
                    // show_main recreates it on a double click; no hidden settings view is retained.
                    window_lifecycle::CloseAction::CloseToTray=>{state.main_epoch.fetch_add(1,Ordering::SeqCst);},
                    window_lifecycle::CloseAction::CloseOther=>{},
                }
            }
        })
        .build(tauri::generate_context!()).expect("WordSide startup failed")
        .run(|app,event|{if matches!(event,tauri::RunEvent::Exit){let state=app.state::<State>();flush_popup_size(&state);state.dictionary.close();}});
}

#[cfg(test)]mod settings_tests{
 use super::*;
 #[test]fn close_preference_migrates_without_resetting_other_settings(){
  for value in [serde_json::json!(null),serde_json::json!("unknown"),serde_json::json!(123),serde_json::json!({"obsolete":true})]{
   let data=serde_json::json!({"enabled":true,"theme":"dark","palette":"blue","lookup_hotkey":"Alt+D","popup_size":{"width":520,"height":640},"close_behavior":value});
   let settings=decode_settings(&serde_json::to_vec(&data).unwrap());assert_eq!(settings.close_behavior,window_lifecycle::CloseBehavior::Tray);assert!(settings.enabled);assert_eq!(settings.theme,"dark");assert_eq!(settings.lookup_hotkey,"Alt+D");assert_eq!(settings.popup_size.width,520);
  }
  assert_eq!(decode_settings(br#"{"enabled":true,"lookup_hotkey":"Alt+D"}"#).close_behavior,window_lifecycle::CloseBehavior::Tray);
 }
 #[test]fn close_preference_is_atomic_and_survives_file_reload(){
  let folder=tempfile::tempdir().unwrap();let config=folder.path().join("owned-test-settings.json");
  let mut settings=decode_settings(br#"{"enabled":true,"theme":"dark","palette":"lavender","lookup_hotkey":"Alt+D","popup_size":{"width":520,"height":640}}"#);
  for behavior in [window_lifecycle::CloseBehavior::Exit,window_lifecycle::CloseBehavior::Tray]{
   settings.close_behavior=behavior;persist_settings(&config,&settings).unwrap();let read=decode_settings(&fs::read(&config).unwrap());
   assert_eq!(read.close_behavior,behavior);assert_eq!(read.lookup_hotkey,"Alt+D");assert!(read.enabled);assert_eq!(read.palette,"lavender");assert_eq!(read.popup_size.height,640);
  }
  assert_eq!(fs::read_dir(folder.path()).unwrap().count(),1,"temporary settings files are cleaned up");
 }
 #[test]fn popup_size_survives_restart_without_resetting_old_preferences(){
  let legacy=decode_settings(br#"{"enabled":true,"theme":"dark","lookup_hotkey":"Alt+D"}"#);assert_eq!(legacy.popup_size,popup_geometry::PopupSize::default());
  let settings=decode_settings(br#"{"enabled":true,"theme":"dark","palette":"blue","lookup_hotkey":"Alt+D","popup_size":{"width":520,"height":640}}"#);
  let again=decode_settings(&serde_json::to_vec(&settings).unwrap());assert_eq!(again.popup_size,popup_geometry::PopupSize{width:520,height:640});assert!(again.enabled);assert_eq!(again.lookup_hotkey,"Alt+D");assert_eq!(again.palette,"blue");
  for size in [serde_json::json!(null),serde_json::json!({"width":"bad","height":-1}),serde_json::json!({"width":999999,"height":0})] {
   let old=serde_json::json!({"enabled":true,"theme":"dark","lookup_hotkey":"Alt+D","popup_size":size});let parsed=decode_settings(&serde_json::to_vec(&old).unwrap());assert!(parsed.enabled);assert_eq!(parsed.theme,"dark");assert_eq!(parsed.lookup_hotkey,"Alt+D");assert_eq!(parsed.popup_size,popup_geometry::PopupSize::default());
  }
 }
 #[test]fn legacy_appearance_defaults_preserve_preferences(){
  let settings=decode_settings(br#"{"enabled":true,"theme":"dark","api_base":"https://example.com","copy_lookup":false,"lookup_hotkey":"Ctrl+Shift+K"}"#);
  assert_eq!(settings.palette,"green");assert_eq!(settings.theme,"dark");assert!(settings.enabled);assert_eq!(settings.lookup_hotkey,"Ctrl+Shift+K");
 }
 #[test]fn palettes_and_modes_survive_restart(){
  for palette in ["green","lavender","blue","stone"]{for theme in ["system","light","dark"]{
   let data=serde_json::json!({"palette":palette,"theme":theme,"enabled":true,"copy_lookup":false,"api_base":"https://example.com/api","lookup_hotkey":"Ctrl+Shift+F9"});
   let settings=decode_settings(&serde_json::to_vec(&data).unwrap());let saved=serde_json::to_vec(&settings).unwrap();let again=decode_settings(&saved);
   assert_eq!(again.palette,palette);assert_eq!(again.theme,theme);assert!(again.enabled);assert_eq!(again.lookup_hotkey,"Ctrl+Shift+F9");
  }}
 }
 #[test]fn unknown_palette_only_resets_palette(){
  let settings=decode_settings(br#"{"palette":"removed","theme":"dark","enabled":true,"api_base":"https://example.com","copy_lookup":false,"lookup_hotkey":"Ctrl+Shift+K"}"#);
  assert_eq!(settings.palette,"green");assert_eq!(settings.theme,"dark");assert!(settings.enabled);assert_eq!(settings.lookup_hotkey,"Ctrl+Shift+K");
 }

 #[test]fn defaults_disabled_without_application_filters(){let settings=Settings::default();assert!(!settings.enabled);assert_eq!(settings.lookup_hotkey,"Ctrl+Alt+A");let json=serde_json::to_value(&settings).unwrap();assert!(json.get("scope").is_none()&&json.get("terminals").is_none());}
 #[test]fn legacy_preferences_survive_filter_removal(){
  for old in [
   br#"{"enabled":true,"theme":"dark","terminals":["pebrel.exe"],"api_base":"https://example.com","lookup_hotkey":"Ctrl+Shift+K","copy_lookup":false}"#.as_slice(),
   br#"{"enabled":true,"scope":"allowlist","theme":"dark","terminals":["pebrel.exe"],"api_base":"https://example.com","lookup_hotkey":"Ctrl+Shift+K","copy_lookup":false}"#.as_slice(),
  ]{
   let settings=decode_settings(old);assert!(settings.enabled);assert_eq!(settings.theme,"dark");assert_eq!(settings.lookup_hotkey,"Ctrl+Shift+K");
   let json=serde_json::to_value(&settings).unwrap();assert!(json.get("scope").is_none()&&json.get("terminals").is_none(),"obsolete restrictions must not be saved");
   assert!(json.get("api_base").is_none(),"obsolete API must not be saved");
   let again=decode_settings(&serde_json::to_vec(&settings).unwrap());assert!(again.enabled,"saved settings without a scope must remain enabled after restart");assert_eq!(again.theme,settings.theme);assert_eq!(again.lookup_hotkey,settings.lookup_hotkey);
  }
 }
 #[test]fn disabled_preference_survives_obsolete_or_unknown_filters(){
  for data in [br#"{"enabled":false,"scope":"allowlist","terminals":[]}"#.as_slice(),br#"{"enabled":false,"scope":"unknown","terminals":{"obsolete":true}}"#.as_slice()]{let settings=decode_settings(data);assert!(!settings.enabled);assert!(!decode_settings(&serde_json::to_vec(&settings).unwrap()).enabled);}
 }
 #[test]fn shortcut_preference_survives_reload(){let settings=decode_settings(br#"{"lookup_hotkey":"Ctrl+Shift+F9","enabled":false,"scope":"all"}"#);assert_eq!(settings.lookup_hotkey,"Ctrl+Shift+F9");assert!(!settings.enabled);assert_eq!(decode_settings(&serde_json::to_vec(&settings).unwrap()).lookup_hotkey,settings.lookup_hotkey);assert_eq!(decode_settings(br#"{"lookup_hotkey":""}"#).lookup_hotkey,"");}
 #[test]fn obsolete_copy_choice_preserves_switch_and_hotkey(){
  for enabled in [true,false]{for old_copy in [serde_json::json!(false),serde_json::json!(true),serde_json::Value::Null,serde_json::json!({"obsolete":true})]{
   let data=serde_json::json!({"enabled":enabled,"copy_lookup":old_copy,"lookup_hotkey":"Alt+D","theme":"dark","palette":"lavender","api_base":"https://example.com/api"});
   let settings=decode_settings(&serde_json::to_vec(&data).unwrap());let saved=serde_json::to_value(&settings).unwrap();
   assert!(saved.get("copy_lookup").is_none(),"copy compatibility must not remain independently configurable");
   let again=decode_settings(&serde_json::to_vec(&saved).unwrap());assert_eq!(again.enabled,enabled);assert_eq!(again.lookup_hotkey,"Alt+D");assert_eq!(again.theme,"dark");assert_eq!(again.palette,"lavender");
  }}
 }

}

// Private headless acceptance entry: never initializes Tauri, UIA, tray or settings.
fn dictionary_probe_entry()->bool {
 let flag=std::env::args().nth(1);
 if flag.as_deref()==Some("--wordside-dictionary-parse") {
  // Development-only content comparison. Bounded files, no window or preferences.
  for path in std::env::args().skip(2) {
   let result=std::fs::File::open(&path).map_err(|_|"无法读取对照文件".to_string()).and_then(|file| {
    use std::io::Read;
    let mut bytes=Vec::new();file.take(4*1024*1024+1).read_to_end(&mut bytes).map_err(|_|"无法读取对照文件".to_string())?;
    if bytes.len()>4*1024*1024{return Err("对照文件过大".into());}
    String::from_utf8(bytes).map_err(|_|"对照文件编码无效".to_string())
   });
   let payload=match result {
    Ok(html)=>match dictionary::parser::parse_page(&html) {
     Ok(entry)=>serde_json::json!({"entry":entry}),
     Err(reason)=>serde_json::json!({"entry":null,"apiError":reason.message,"apiStatus":reason.status,"apiCode":reason.code}),
    },
    Err(error)=>serde_json::json!({"entry":null,"apiError":error}),
   };
   println!("{}",serde_json::json!({"file":path,"payload":payload}));
  }
  return true;
 }
 if flag.as_deref()!=Some("--wordside-dictionary-probe"){return false;}
 tauri::async_runtime::block_on(async {
  let backend=dictionary::Backend::default();
  println!("{}",serde_json::json!({"phase":"initial","backend":backend.diagnostic()}));
  for query in std::env::args().skip(2){
   let(language,raw)=query.split_once(':').unwrap_or(("en-cn",query.as_str()));
   let word=capture::selected_word(raw);
   let started=std::time::Instant::now();
   let payload=if let Some(word)=word.as_deref(){fetch_payload(&backend,word,language).await.unwrap_or_else(|reason|serde_json::json!({"entry":null,"apiError":reason}))}
    else{serde_json::json!({"entry":null,"apiError":"请选择单行英文单词或短语"})};
   println!("{}",serde_json::json!({"selection":raw,"word":word,"language":language,"payload":payload,"ms":started.elapsed().as_secs_f64()*1000.0,"backend":backend.diagnostic()}));
  }
  backend.close();println!("{}",serde_json::json!({"phase":"closed","backend":backend.diagnostic()}));
 });
 true
}
