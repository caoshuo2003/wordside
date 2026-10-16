use tauri::AppHandle;
mod policy;
mod compatibility;
mod copy_policy;
mod timing;
#[cfg(windows)]mod diagnostics;
#[cfg(windows)]mod windows_copy;
#[cfg(windows)]mod input;
pub mod shortcut;
pub mod selection;
#[cfg(windows)]pub(crate) mod probe;
#[cfg(windows)]pub(crate) mod hotkey;
pub use policy::selected_word;
#[cfg(windows)] mod windows;
#[cfg(windows)] pub(crate) mod windows_text;
#[cfg(windows)] mod windows_edit;
pub fn hide(window:&tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        #[link(name="user32")] extern "system" {fn ShowWindow(window:isize,command:i32)->i32;}
        if let Ok(hwnd)=window.hwnd(){unsafe{ShowWindow(hwnd.0 as isize,0);}}
    }
    #[cfg(not(windows))]
    {let _=window.hide();}
}

fn poll_interval(enabled:bool,popup_visible:bool)->u64{if !enabled&&!popup_visible{250}else{20}}
pub fn place(x:i32,y:i32,width:i32,height:i32,work:(i32,i32,i32,i32))->(i32,i32) {
    let (l,t,r,b)=work;
    let px=(x+14).min(r-width-8).max(l+8);
    let py=if y+18+height>b-8 {y-height-12} else {y+18};
    (px,py.min(b-height-8).max(t+8))
}
#[cfg(not(windows))]
pub fn start(_app:AppHandle) {}
#[cfg(windows)]
pub fn start(app:AppHandle) { std::thread::spawn(move||windows::run(app)); }

#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn global_sources_and_self_exclusion(){use policy::permits_source;assert!(permits_source(2,1));assert!(permits_source(3,1));assert!(!permits_source(1,1));assert!(!permits_source(0,1));}
    #[test]fn intentional_mouse_and_copy_inputs(){use policy::{mouse_selection,copy_chord};assert!(!mouse_selection((0,0),(2,1),false,false));assert!(mouse_selection((0,0),(5,0),false,false));assert!(mouse_selection((0,0),(0,0),true,false));assert!(mouse_selection((0,0),(0,0),false,true));assert!(copy_chord(true,true,false,false,false));assert!(copy_chord(true,false,true,false,false));assert!(!copy_chord(false,true,false,false,false));assert!(!copy_chord(true,true,false,true,false));assert!(!copy_chord(true,true,false,false,true));}
    #[test]fn words_and_phrases(){assert_eq!(selected_word(" `terminal,` ").as_deref(),Some("terminal"));assert_eq!(selected_word("run on the spot").as_deref(),Some("run on the spot"));assert!(selected_word("don't").is_some());assert_eq!(selected_word("terminal\r\n").as_deref(),Some("terminal"));}
    #[test]fn idle_polling(){assert_eq!(poll_interval(false,false),250);assert_eq!(poll_interval(true,false),20);assert_eq!(poll_interval(false,true),20);}
    #[test]fn punctuation_selection(){for s in ["“runtime”。","(runtime),","runtime()","--runtime","-runtime"]{assert_eq!(selected_word(s).as_deref(),Some("runtime"));}assert_eq!(selected_word("termnal").as_deref(),Some("termnal"));}
    #[test]fn rejects_logs_paths_and_commands(){for s in ["foo\nbar","C:\\Users\\Admin","https://example.com","rm -rf /","12345",""]{assert!(selected_word(s).is_none(),"{s}");}}
    #[test]fn limits_clipboard(){assert!(selected_word(&"x".repeat(81)).is_none());assert!(selected_word("one two three four five six").is_none());}
    #[test]fn monitor_edges_and_negative_coordinates(){assert_eq!(place(1910,1070,480,680,(0,0,1920,1080)),(1432,378));let (x,y)=place(-1900,100,480,680,(-1920,0,0,1080));assert!(x>=-1912&&x+480<=-8&&y>=8&&y+680<=1072);}
}
