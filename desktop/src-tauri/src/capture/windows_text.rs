use super::policy::selected_word;
pub use super::selection::Selection;
use serde::{Serialize,Deserialize};
use std::ffi::c_void;
use ::windows::Win32::{Foundation::{POINT,HWND},UI::Accessibility::{IUIAutomation,IUIAutomationElement,IUIAutomationTextPattern,UIA_TextPatternId}};
#[derive(Clone,Copy,Serialize,Deserialize)]pub enum TextOrigin{Pointer(i32,i32),Focused,#[cfg(test)]Window}
#[derive(Clone,Serialize,Deserialize)]pub struct TextRequest{pub pid:u32,pub window:isize,pub origin:TextOrigin,#[serde(default)]pub focus:isize}
type Handle=isize;
#[derive(Default)]#[repr(C)]struct FocusRect{left:i32,top:i32,right:i32,bottom:i32}
#[derive(Default)]#[repr(C)]struct GuiInfo{size:u32,flags:u32,active:Handle,focus:Handle,capture:Handle,menu:Handle,moving:Handle,caret:Handle,rect:FocusRect}
pub fn focus_matches(window:Handle,focus:Handle)->bool{unsafe{
 if focus==0{return true;}let thread=GetWindowThreadProcessId(window,std::ptr::null_mut());if thread==0{return false;}
 let mut info=GuiInfo{size:std::mem::size_of::<GuiInfo>() as u32,..Default::default()};GetGUIThreadInfo(thread,&mut info)!=0&&info.focus==focus
}}
#[derive(Default,Clone,Copy)]#[repr(C)]struct Coord{x:i16,y:i16}
#[derive(Default)]#[repr(C)]struct SmallRect{left:i16,top:i16,right:i16,bottom:i16}
#[derive(Default)]#[repr(C)]struct ConsoleSelection{flags:u32,anchor:Coord,rect:SmallRect}
#[repr(C)]struct ProcessEntry{size:u32,usage:u32,id:u32,heap:usize,module:u32,threads:u32,parent:u32,priority:i32,flags:u32,name:[u16;260]}
struct AttachedConsole;
impl Drop for AttachedConsole{fn drop(&mut self){unsafe{FreeConsole();}}}
#[link(name="user32")]extern "system"{fn GetClassNameW(window:Handle,name:*mut u16,max:i32)->i32;fn GetAncestor(window:Handle,flags:u32)->Handle;fn GetWindowThreadProcessId(window:Handle,pid:*mut u32)->u32;fn GetForegroundWindow()->Handle;fn GetGUIThreadInfo(thread:u32,info:*mut GuiInfo)->i32;}
#[link(name="kernel32")]extern "system"{
 fn AttachConsole(process:u32)->i32;fn FreeConsole()->i32;fn GetConsoleSelectionInfo(info:*mut ConsoleSelection)->i32;
 fn CreateFileW(name:*const u16,access:u32,share:u32,security:*const c_void,disposition:u32,flags:u32,template:Handle)->Handle;
 fn ReadConsoleOutputCharacterW(handle:Handle,text:*mut u16,count:u32,start:Coord,read:*mut u32)->i32;fn CloseHandle(handle:Handle)->i32;
 fn CreateToolhelp32Snapshot(flags:u32,process:u32)->Handle;fn Process32FirstW(snapshot:Handle,entry:*mut ProcessEntry)->i32;fn Process32NextW(snapshot:Handle,entry:*mut ProcessEntry)->i32;
}
fn console_selection(request:&TextRequest)->Selection{
 unsafe{
  let mut clients=vec![request.pid];let snapshot=CreateToolhelp32Snapshot(2,0);
  if snapshot!=-1{
   let mut entry:ProcessEntry=std::mem::zeroed();entry.size=std::mem::size_of::<ProcessEntry>() as u32;
   if Process32FirstW(snapshot,&mut entry)!=0{loop{if entry.parent==request.pid{clients.push(entry.id);}if Process32NextW(snapshot,&mut entry)==0{break;}}}CloseHandle(snapshot);
  }
  for client in clients{
   if AttachConsole(client)==0{continue;}let _attached=AttachedConsole;
   let mut info=ConsoleSelection::default();if GetConsoleSelectionInfo(&mut info)==0{return Selection::ReadError("GetConsoleSelectionInfo".into());}
   if info.flags&2==0{return Selection::NoSelection;}
   let rect=info.rect;let width=i32::from(rect.right)-i32::from(rect.left)+1;
   if rect.top!=rect.bottom||rect.left<0||rect.top<0||!(1..=80).contains(&width){return Selection::InvalidWord;}
   let path:Vec<u16>="CONOUT$\0".encode_utf16().collect();let output=CreateFileW(path.as_ptr(),0x80000000,3,std::ptr::null(),3,0,0);
   if output==-1{return Selection::ReadError("Open console output".into());}
   let mut text=[0u16;80];let mut read=0;
   let ok=ReadConsoleOutputCharacterW(output,text.as_mut_ptr(),width as u32,Coord{x:rect.left,y:rect.top},&mut read);CloseHandle(output);
   if ok==0||read!=width as u32{return Selection::ReadError("ReadConsoleOutputCharacterW".into());}
   return String::from_utf16(&text[..read as usize]).ok().and_then(|text|selected_word(&text)).map(Selection::Word).unwrap_or(Selection::InvalidWord);
  }
  Selection::ReadError("AttachConsole".into())
 }
}
fn uia_error(operation:&str,error: ::windows::core::Error)->Selection{
 let code=error.code().0 as u32;
 if code==::windows::Win32::UI::Accessibility::UIA_E_TIMEOUT{Selection::Timeout}
 else if code==0x80070005{Selection::AccessDenied}
 else if code==::windows::Win32::UI::Accessibility::UIA_E_ELEMENTNOTAVAILABLE{Selection::SourceMismatch}
 else{Selection::ReadError(format!("{operation}: 0x{code:08x}"))}
}

// Verify the unfiltered ancestor path belongs to this source before reading text.
// Browser renderers can have a different PID from the foreground root window.
unsafe fn from_element(automation:&IUIAutomation,first:IUIAutomationElement,window:Handle,original_focus:Handle)->Selection{
 let root=GetAncestor(window,2);let root=if root==0{window}else{root};
 let target=match automation.ElementFromHandle(HWND(root as *mut c_void)){Ok(value)=>value,Err(error)=>return uia_error("Source window",error)};
 let walker=match automation.RawViewWalker(){Ok(value)=>value,Err(error)=>return uia_error("RawViewWalker",error)};
 let mut cursor=first;let mut elements=Vec::new();let mut belongs=false;
 for _ in 0..32{
  match cursor.CurrentIsPassword(){Ok(value)if value.as_bool()=>return Selection::Protected,Ok(_)=>{},Err(error)=>return uia_error("IsPassword",error)}
  let native=match cursor.CurrentNativeWindowHandle(){
   Ok(handle)=>{let native=handle.0 as isize;if native!=0{let ancestor=GetAncestor(native,2);if native!=root&&ancestor!=root{return Selection::SourceMismatch;}}native},
   Err(error)=>return uia_error("NativeWindowHandle",error),
  };
  elements.push((cursor.clone(),native));
  match automation.CompareElements(&cursor,&target){Ok(value)if value.as_bool()=>{belongs=true;break;},Ok(_)=>{},Err(error)=>return uia_error("CompareElements",error)}
  match walker.GetParentElement(&cursor){Ok(parent)=>cursor=parent,Err(error)=>return uia_error("Parent element",error),}
 }
 if !belongs{return Selection::SourceMismatch;}
 if original_focus!=0{if let Some(selection)=super::windows_edit::selected_edit(original_focus,root){return selection;}}
 let mut supported=false;
 for (element,native) in elements{
  if native!=0{if let Some(selection)=super::windows_edit::selected_edit(native,root){return selection;}}
  let pattern=match element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId){
   Ok(value)=>value,Err(error)if matches!(error.code().0 as u32,0|0x80004002|::windows::Win32::UI::Accessibility::UIA_E_NOTSUPPORTED)=>continue,
   Err(error)=>return uia_error("TextPattern",error),
  };
  match pattern.SupportedTextSelection(){Ok(value)if value.0==0=>continue,Ok(_)=>{},Err(error)=>return uia_error("SupportedTextSelection",error)}
  supported=true;
  let ranges=match pattern.GetSelection(){Ok(value)=>value,Err(error)if error.code().0==0=>continue,Err(error)=>return uia_error("GetSelection",error)};
  let length=match ranges.Length(){Ok(value)=>value,Err(error)=>return uia_error("Selection count",error)};
  if length==0{continue;}if length!=1{return Selection::InvalidWord;}
  let range=match ranges.GetElement(0){Ok(value)=>value,Err(error)=>return uia_error("Selection range",error)};
  let text=match range.GetText(101){Ok(value)=>value.to_string(),Err(error)=>return uia_error("Selected text",error)};
  if text.is_empty(){continue;}if text.encode_utf16().count()>100{return Selection::InvalidWord;}
  return selected_word(&text).map(Selection::Word).unwrap_or(Selection::InvalidWord);
 }
 if supported{Selection::NoSelection}else{Selection::Unsupported}
}
pub fn selected_text(automation:&IUIAutomation,request:&TextRequest)->Selection{
 unsafe{
  let root=GetAncestor(request.window,2);let root=if root==0{request.window}else{root};let mut source_pid=0;GetWindowThreadProcessId(root,&mut source_pid);
  if request.pid==0||source_pid!=request.pid{return Selection::SourceMismatch;}
  if matches!(request.origin,TextOrigin::Focused){
   let foreground=GetForegroundWindow();let active=GetAncestor(foreground,2);if (if active==0{foreground}else{active})!=root{return Selection::SourceMismatch;}
   if !focus_matches(root,request.focus){return Selection::SourceMismatch;}
   if request.focus!=0{let focus_root=GetAncestor(request.focus,2);if request.focus!=root&&focus_root!=root{return Selection::SourceMismatch;}}
  }
  let mut class=[0u16;64];let length=GetClassNameW(request.window,class.as_mut_ptr(),64);
  if length>0&&String::from_utf16_lossy(&class[..length as usize])=="ConsoleWindowClass"{return console_selection(request);}
  let first=match request.origin{
   TextOrigin::Pointer(x,y)=>automation.ElementFromPoint(POINT{x,y}),
   TextOrigin::Focused=>automation.GetFocusedElement(),
   #[cfg(test)]TextOrigin::Window=>automation.ElementFromHandle(HWND(request.window as *mut c_void)),
  };
  match first{
   Ok(element)=>{
    let original=element.clone();let result=from_element(automation,element,request.window,if matches!(request.origin,TextOrigin::Focused){request.focus}else{0});
    if matches!(request.origin,TextOrigin::Focused)&&result!=Selection::Protected{
     if GetForegroundWindow()!=root||!focus_matches(root,request.focus){return Selection::SourceMismatch;}
     let current=match automation.GetFocusedElement(){Ok(value)=>value,Err(error)=>return uia_error("Current focus",error)};
     match automation.CompareElements(&original,&current){Ok(value)if value.as_bool()=>{},Ok(_)=>return Selection::SourceMismatch,Err(error)=>return uia_error("Focus identity",error)}
    }
    result
   },Err(error)=>uia_error("Source element",error)
  }
 }
}

#[cfg(test)]pub(crate) mod tests {
 use super::*;
 use std::{sync::mpsc,time::Duration,io::{BufRead,Write},process::{Command,Stdio,Child}};
 use ::windows::Win32::System::Com::{CoInitializeEx,CoUninitialize,CoCreateInstance,COINIT_MULTITHREADED,CLSCTX_INPROC_SERVER};
 use ::windows::Win32::UI::Accessibility::CUIAutomation;
 #[repr(C)]#[derive(Default)]struct Msg{window:isize,message:u32,wparam:usize,lparam:isize,time:u32,x:i32,y:i32,private:u32}
 #[link(name="user32")]extern "system"{
  fn CreateWindowExW(ex:u32,class:*const u16,name:*const u16,style:u32,x:i32,y:i32,width:i32,height:i32,parent:isize,menu:isize,instance:isize,param:*const c_void)->isize;
  fn DestroyWindow(window:isize)->i32;fn SendMessageW(window:isize,message:u32,wparam:usize,lparam:isize)->isize;
  fn GetMessageW(message:*mut Msg,window:isize,min:u32,max:u32)->i32;fn TranslateMessage(message:*const Msg)->i32;fn DispatchMessageW(message:*const Msg)->isize;
  fn PostThreadMessageW(thread:u32,message:u32,wparam:usize,lparam:isize)->i32;
  fn GetForegroundWindow()->isize;fn GetClipboardSequenceNumber()->u32;
 }
 #[link(name="kernel32")]extern "system"{fn GetCurrentThreadId()->u32;fn LoadLibraryExW(name:*const u16,file:isize,flags:u32)->isize;fn FreeLibrary(module:isize)->i32;}
 fn wide(value:&str)->Vec<u16>{value.encode_utf16().chain(Some(0)).collect()}
 struct Fixture{root:isize,other:isize,edit:isize,password:isize,rename:isize,thread:u32,join:Option<std::thread::JoinHandle<()>>,pauses:mpsc::Receiver<()>}
 impl Fixture{fn new()->Self{
  let(tx,rx)=mpsc::channel();let(pause_tx,pauses)=mpsc::channel();let join=std::thread::spawn(move||unsafe{
   let class=wide("STATIC");let title=wide("WordSide-isolated-selection-test");
   // Off-screen, tool/no-activate windows. No user focus, mouse or clipboard changes.
   let root=CreateWindowExW(0x08000080,class.as_ptr(),title.as_ptr(),0x90000000,-30000,-30000,400,220,0,0,0,std::ptr::null());
   let other=CreateWindowExW(0x08000080,class.as_ptr(),title.as_ptr(),0x90000000,-30000,-30200,400,160,0,0,0,std::ptr::null());
   let rich=LoadLibraryExW(wide("msftedit.dll").as_ptr(),0,0x800);assert_ne!(rich,0);
   let edit=CreateWindowExW(0,wide("RICHEDIT50W").as_ptr(),wide("terminal").as_ptr(),0x50000804,10,10,300,60,root,0,0,std::ptr::null());
   let password=CreateWindowExW(0,wide("EDIT").as_ptr(),wide("secretword").as_ptr(),0x50000020,10,90,300,30,root,0,0,std::ptr::null());
   let rename=CreateWindowExW(0,wide("EDIT").as_ptr(),wide("my runtime notes.txt").as_ptr(),0x50000080,10,135,300,30,root,0,0,std::ptr::null());
   SendMessageW(edit,0xB1,0,8);SendMessageW(password,0xB1,0,10);SendMessageW(rename,0xB1,3,10);
   tx.send((root,other,edit,password,rename,GetCurrentThreadId())).unwrap();
   let mut message=Msg::default();while GetMessageW(&mut message,0,0,0)>0{if message.message==0x8001{pause_tx.send(()).unwrap();std::thread::sleep(Duration::from_millis(650));continue;}TranslateMessage(&message);DispatchMessageW(&message);}
   DestroyWindow(root);DestroyWindow(other);FreeLibrary(rich);
  });
  let(root,other,edit,password,rename,thread)=rx.recv_timeout(Duration::from_secs(3)).unwrap();assert!(root!=0&&other!=0&&edit!=0&&password!=0&&rename!=0);
  Self{root,other,edit,password,rename,thread,join:Some(join),pauses}
 }
 fn pause(&self){unsafe{assert_ne!(PostThreadMessageW(self.thread,0x8001,0,0),0);}self.pauses.recv_timeout(Duration::from_secs(2)).unwrap();}
 }
 impl Drop for Fixture{fn drop(&mut self){unsafe{PostThreadMessageW(self.thread,0x12,0,0);}if let Some(join)=self.join.take(){join.join().unwrap();}}}
 // Probe an external process, matching the real capture boundary. RichEdit
 // provides the UIA text pattern; the password fixture stays a plain Win32 edit.
 // Keep native UIA fixture lifetimes separate so independent tests cannot
 // initialise or tear down identical off-screen providers at the same time.
 // Test-only isolation; this does not serialize product lookup requests.
 static NATIVE_FIXTURE:std::sync::Mutex<()>=std::sync::Mutex::new(());
 pub(crate) struct ExternalFixture{pub root:isize,pub other:isize,pub edit:isize,pub password:isize,pub rename:isize,pub pid:u32,output:std::io::BufReader<std::process::ChildStdout>,child:Child,_fixture_guard:std::sync::MutexGuard<'static,()>}
 impl ExternalFixture{pub fn new()->Self{
  let fixture_guard=NATIVE_FIXTURE.lock().unwrap_or_else(|poisoned|poisoned.into_inner());
  let mut child=Command::new(std::env::current_exe().unwrap()).args(["--ignored","--exact","capture::windows_text::tests::fixture_process","--nocapture"]).env("WORDSIDE_NATIVE_FIXTURE","1").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
  let mut output=std::io::BufReader::new(child.stdout.take().unwrap());let mut line=String::new();let mut handles=None;
  while output.read_line(&mut line).unwrap()>0{
   if let Some(data)=line.trim().strip_prefix("WORDSIDE_FIXTURE "){handles=Some(data.split_whitespace().map(|value|value.parse::<isize>().unwrap()).collect::<Vec<_>>());break;}
   line.clear();
  }
  let handles=handles.expect("native fixture must announce its windows");assert_eq!(handles.len(),5);let pid=child.id();
  Self{root:handles[0],other:handles[1],edit:handles[2],password:handles[3],rename:handles[4],pid,output,child,_fixture_guard:fixture_guard}
 }
 pub fn pause(&mut self){let input=self.child.stdin.as_mut().unwrap();input.write_all(b"pause\n").unwrap();input.flush().unwrap();let mut line=String::new();loop{assert!(self.output.read_line(&mut line).unwrap()>0);if line.trim()=="WORDSIDE_FIXTURE_PAUSED"{break;}line.clear();}}
 }
 impl Drop for ExternalFixture{fn drop(&mut self){if let Some(mut input)=self.child.stdin.take(){let _=input.write_all(b"exit\n");}let _=std::io::copy(&mut self.output,&mut std::io::sink());let _=self.child.wait();}}
 #[test]#[ignore="isolated subprocess used by the native selection probe"]fn fixture_process(){
  if std::env::var("WORDSIDE_NATIVE_FIXTURE").as_deref()!=Ok("1"){return;}
  let fixture=Fixture::new();println!("WORDSIDE_FIXTURE {} {} {} {} {}",fixture.root,fixture.other,fixture.edit,fixture.password,fixture.rename);std::io::stdout().flush().unwrap();
  let input=std::io::stdin();for line in input.lock().lines(){if line.unwrap()=="pause"{fixture.pause();println!("WORDSIDE_FIXTURE_PAUSED");std::io::stdout().flush().unwrap();}else{break;}}drop(fixture);
 }
 #[test]fn plain_rename_edit_keeps_only_selected_word(){unsafe{
  let fixture=ExternalFixture::new();CoInitializeEx(None,COINIT_MULTITHREADED).unwrap();
  {
   let automation:IUIAutomation=CoCreateInstance(&CUIAutomation,None,CLSCTX_INPROC_SERVER).unwrap();
   let request=TextRequest{pid:fixture.pid,window:fixture.rename,origin:TextOrigin::Window,focus:0};
   assert_eq!(selected_text(&automation,&request),Selection::Word("runtime".into()),"selected substring in a plain rename EDIT must be readable without copying");
   let root=automation.ElementFromHandle(HWND(fixture.root as *mut c_void)).unwrap();
   assert_eq!(from_element(&automation,root,fixture.root,fixture.rename),Selection::Word("runtime".into()),"original native focus must work even when the UIA element is its parent");
   let root=automation.ElementFromHandle(HWND(fixture.root as *mut c_void)).unwrap();
   assert_eq!(from_element(&automation,root,fixture.root,fixture.password),Selection::Protected,"native focus must retain password protection");
   let root=automation.ElementFromHandle(HWND(fixture.other as *mut c_void)).unwrap();
   assert_eq!(from_element(&automation,root,fixture.other,fixture.rename),Selection::SourceMismatch,"native focus cannot borrow another source root");
   let(mut start,mut end)=(0u32,0u32);SendMessageW(fixture.rename,0xb0,&mut start as *mut u32 as usize,&mut end as *mut u32 as isize);
   assert_eq!((start,end),(3,10),"reading must retain the exact original selection");
   let mut text=[0u16;128];let count=SendMessageW(fixture.rename,0xd,text.len(),text.as_mut_ptr() as isize);
   assert_eq!(String::from_utf16(&text[..count as usize]).unwrap(),"my runtime notes.txt","reading must not modify the filename");
  }
  CoUninitialize();assert_ne!(GetForegroundWindow(),fixture.root);drop(fixture);
 }}
 #[test]fn ordinary_edit_unicode_filter_password_and_long_offsets(){unsafe{
  let fixture=ExternalFixture::new();
  let read=||super::super::windows_edit::selected_edit(fixture.rename,fixture.root).unwrap();
  assert_eq!(super::super::windows_edit::selected_edit(fixture.edit,fixture.root),None,"RichEdit must retain the UIA route");
  assert_eq!(super::super::windows_edit::selected_edit(fixture.rename,fixture.other),Some(Selection::SourceMismatch));
  assert_eq!(super::super::windows_edit::selected_edit(fixture.password,fixture.root),Some(Selection::Protected));
  for(value,start,end,expected)in [
   ("中文🧪 runtime suffix.txt".to_string(),5,12,Selection::Word("runtime".into())),
   ("notes (runtime),.txt".to_string(),6,16,Selection::Word("runtime".into())),
   ("C:\\Users\\Admin".to_string(),0,14,Selection::InvalidWord),
   ("foo\r\nbar".to_string(),0,8,Selection::InvalidWord),
   ("x".repeat(81),0,81,Selection::InvalidWord),
   ("中文🧪 runtime".to_string(),2,3,Selection::InvalidWord),
   (" ".repeat(70_000)+"runtime suffix.txt",70_000,70_007,Selection::Word("runtime".into())),
  ]{
   SendMessageW(fixture.rename,0xc,0,wide(&value).as_ptr() as isize);SendMessageW(fixture.rename,0xb1,start,end);
   assert_eq!(read(),expected,"native EDIT range {start}..{end}");
  }
  SendMessageW(fixture.rename,0xb1,0,0);assert_eq!(read(),Selection::NoSelection,"an insertion point cannot borrow a previous or parent selection");
  SendMessageW(fixture.rename,0xcc,'*' as usize,0);SendMessageW(fixture.rename,0xb1,70_000,70_007);assert_eq!(read(),Selection::Protected,"runtime masking must prevent reading");
  assert_ne!(GetForegroundWindow(),fixture.root);drop(fixture);
 }}
 #[test]fn unresponsive_native_edit_times_out_then_recovers(){
  let mut fixture=ExternalFixture::new();fixture.pause();let started=std::time::Instant::now();
  assert_eq!(super::super::windows_edit::selected_edit(fixture.rename,fixture.root),Some(Selection::Timeout),"a paused external message loop must not hang the reader");
  assert!(started.elapsed()<Duration::from_millis(600),"the native message should observe its 150ms budget, not the entire source stall");
  std::thread::sleep(Duration::from_millis(700));
  assert_eq!(super::super::windows_edit::selected_edit(fixture.rename,fixture.root),Some(Selection::Word("runtime".into())),"selection remains readable when the same source resumes");
 }
 #[test]fn native_text_controls_selection_owner_password_and_filtering(){unsafe{
  let clipboard=GetClipboardSequenceNumber();let fixture=ExternalFixture::new();
  assert_ne!(GetForegroundWindow(),fixture.root,"offscreen fixture must not activate");assert_ne!(GetForegroundWindow(),fixture.other);
  CoInitializeEx(None,COINIT_MULTITHREADED).unwrap();
  {
   let automation:IUIAutomation=CoCreateInstance(&CUIAutomation,None,CLSCTX_INPROC_SERVER).unwrap();
   let read=|window|selected_text(&automation,&TextRequest{pid:fixture.pid,window,origin:TextOrigin::Window,focus:0});
   let edit=automation.ElementFromHandle(HWND(fixture.edit as *mut c_void)).unwrap();
   assert!(edit.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).is_ok(),"isolated RichEdit fixture must expose TextPattern");
   assert_eq!(read(fixture.edit),Selection::Word("terminal".into()));
   assert_eq!(read(fixture.other),Selection::Unsupported,"static control must be distinguished from an empty selectable control");
   assert_eq!(selected_text(&automation,&TextRequest{pid:fixture.pid.wrapping_add(1),window:fixture.edit,origin:TextOrigin::Window,focus:0}),Selection::SourceMismatch,"stale process identity must be refused");
   assert_eq!(selected_text(&automation,&TextRequest{pid:fixture.pid,window:fixture.edit,origin:TextOrigin::Focused,focus:fixture.other}),Selection::SourceMismatch,"manual capture cannot borrow an inactive or foreign focus");
   let element=automation.ElementFromHandle(HWND(fixture.edit as *mut c_void)).unwrap();
   assert_eq!(from_element(&automation,element,fixture.other,0),Selection::SourceMismatch,"other source window must not borrow this selection");
   assert_eq!(read(fixture.password),Selection::Protected);
   SendMessageW(fixture.edit,0xB1,0,0);assert_eq!(read(fixture.edit),Selection::NoSelection,"insertion point is not selected text");
   for(value,expected)in [("(runtime),",Selection::Word("runtime".into())),("foo\r\nbar",Selection::InvalidWord),("C:\\Users\\Admin",Selection::InvalidWord),("x".repeat(120).as_str(),Selection::InvalidWord)]{
    SendMessageW(fixture.edit,0x0C,0,wide(value).as_ptr() as isize);SendMessageW(fixture.edit,0xB1,0,-1);assert_eq!(read(fixture.edit),expected,"{value}");
   }
  }
  CoUninitialize();assert_ne!(GetForegroundWindow(),fixture.root);assert_ne!(GetForegroundWindow(),fixture.other);drop(fixture);
  if GetClipboardSequenceNumber()!=clipboard{eprintln!("Clipboard changed outside this read-only fixture during the probe.");}
 }}
}
