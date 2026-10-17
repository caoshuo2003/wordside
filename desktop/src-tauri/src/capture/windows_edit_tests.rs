//! Only private hidden controls and synthetic text. No foreground, input,
//! clipboard, UIA, user process or persisted preference access.
use super::*;
use std::{io::{BufRead,Write},process::{Child,Command,Stdio},sync::mpsc,time::Duration};
#[derive(Default)] #[repr(C)] struct Msg { window:isize,message:u32,wparam:usize,lparam:isize,time:u32,x:i32,y:i32,private:u32 }
#[link(name="user32")] extern "system" {
 fn CreateWindowExW(ex:u32,class:*const u16,title:*const u16,style:u32,x:i32,y:i32,w:i32,h:i32,parent:isize,menu:isize,module:isize,param:*const std::ffi::c_void)->isize;
 fn DestroyWindow(window:isize)->i32;
 fn SendMessageW(window:isize,message:u32,wparam:usize,lparam:isize)->isize;
 fn GetMessageW(msg:*mut Msg,window:isize,min:u32,max:u32)->i32;
 fn TranslateMessage(msg:*const Msg)->i32;
 fn DispatchMessageW(msg:*const Msg)->isize;
 fn PostThreadMessageW(thread:u32,message:u32,wparam:usize,lparam:isize)->i32;
}
#[link(name="kernel32")] extern "system" {
 fn LoadLibraryExW(name:*const u16,file:isize,flags:u32)->isize;
 fn FreeLibrary(module:isize)->i32;
 fn GetCurrentThreadId()->u32;
}
fn wide(text:&str)->Vec<u16>{text.encode_utf16().chain(Some(0)).collect()}
#[test] #[ignore="private hidden native-control fixture"] fn fixture_process() {
 if std::env::var("WORDSIDE_HIDDEN_EDIT_FIXTURE").as_deref()!=Ok("1"){return;}
 let(tx,rx)=mpsc::channel();
 let join=std::thread::spawn(move||unsafe{
  let module=LoadLibraryExW(wide("msftedit.dll").as_ptr(),0,0x800); assert_ne!(module,0);
  // No WS_VISIBLE, activation, focus, accessibility or real desktop observation.
  let root=CreateWindowExW(0,wide("STATIC").as_ptr(),wide("").as_ptr(),0x80000000,0,0,600,200,0,0,0,std::ptr::null());
  let create=|class:&str,style:u32|CreateWindowExW(0,wide(class).as_ptr(),wide("").as_ptr(),0x40000000|style,0,0,500,100,root,0,0,std::ptr::null());
  let rich=create("RICHEDIT50W",0x84);let edit=create("EDIT",0x84);let password=create("EDIT",0x20);
  assert!(root!=0&&rich!=0&&edit!=0&&password!=0);
  tx.send((root,rich,edit,password,GetCurrentThreadId())).unwrap();
  let mut msg=Msg::default();while GetMessageW(&mut msg,0,0,0)>0{TranslateMessage(&msg);DispatchMessageW(&msg);}
  DestroyWindow(root);FreeLibrary(module);
 });
 let(root,rich,edit,password,thread)=rx.recv_timeout(Duration::from_secs(3)).unwrap();
 println!("HIDDEN_EDIT {root} {rich} {edit} {password}");std::io::stdout().flush().unwrap();
 for line in std::io::stdin().lock().lines(){
  let value:serde_json::Value=serde_json::from_str(&line.unwrap()).unwrap();
  if value["exit"]==true {break;}
  let handle=match value["control"].as_str().unwrap(){"rich"=>rich,"edit"=>edit,"password"=>password,_=>panic!("unknown private control")};
  let text=value["text"].as_str().unwrap();
  unsafe{SendMessageW(handle,0x000c,0,wide(text).as_ptr() as isize);SendMessageW(handle,0xb1,value["start"].as_u64().unwrap() as usize,value["end"].as_i64().unwrap() as isize);}
  println!("HIDDEN_READY");std::io::stdout().flush().unwrap();
 }
 unsafe{PostThreadMessageW(thread,0x12,0,0);}join.join().unwrap();
}
struct Fixture { child:Child, output:std::io::BufReader<std::process::ChildStdout>, root:isize,rich:isize,edit:isize,password:isize }
impl Fixture {
 fn new()->Self {
  use std::os::windows::process::CommandExt;
  let mut child=Command::new(std::env::current_exe().unwrap()).args(["--ignored","--exact","capture::windows_edit::tests::fixture_process","--nocapture"])
   .env("WORDSIDE_HIDDEN_EDIT_FIXTURE","1").creation_flags(0x08000000).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
  let mut output=std::io::BufReader::new(child.stdout.take().unwrap());let mut line=String::new();
  let handles=loop {
   assert!(output.read_line(&mut line).unwrap()>0);
   if let Some(row)=line.trim().strip_prefix("HIDDEN_EDIT "){break row.split_whitespace().map(|item|item.parse::<isize>().unwrap()).collect::<Vec<_>>();}
   line.clear();
  };
  Self{child,output,root:handles[0],rich:handles[1],edit:handles[2],password:handles[3]}
 }
 fn select(&mut self,control:&str,text:&str,start:u32,end:u32)->isize {
  let input=self.child.stdin.as_mut().unwrap();writeln!(input,"{}",serde_json::json!({"control":control,"text":text,"start":start,"end":end})).unwrap();input.flush().unwrap();
  let mut line=String::new();loop {assert!(self.output.read_line(&mut line).unwrap()>0);if line.trim()=="HIDDEN_READY"{break;}line.clear();}
  match control {"rich"=>self.rich,"edit"=>self.edit,"password"=>self.password,_=>unreachable!()}
 }
}
impl Drop for Fixture {
 fn drop(&mut self){if let Some(mut input)=self.child.stdin.take(){let _=writeln!(input,"{{\"exit\":true}}");}let _=std::io::copy(&mut self.output,&mut std::io::sink());let _=self.child.wait();}
}
#[test] fn hidden_rich_edit_selected_word_uses_internal_line_coordinates() {unsafe{
 let mut fixture=Fixture::new();
 for prefix in ["".to_string(),"first\r\nsecond\r\n".into(),"中文🧪\r\n".into(),"line\r\n".repeat(15_000)] {
  let start=prefix.replace("\r\n","\r").encode_utf16().count() as u32;
  let text=format!("{prefix}-workspace suffix");let window=fixture.select("rich",&text,start,start+10);
  assert_eq!(selected_edit(window,fixture.root),Some(Selection::Word("workspace".into())),"RichEdit range after {} UTF-16 units",start);
  assert_eq!(range(window).unwrap(),(start,start+10),"read must leave the range unchanged");
 }
}}
#[test] fn hidden_rich_edit_preserves_selected_paragraph_edge_whitespace() {
 let mut fixture=Fixture::new();
 for(text,start,end) in [("runtime\r\nnext",0,8),("before\r\nruntime",6,14),("runtime\r\n\r\nnext",0,9)] {
  let window=fixture.select("rich",text,start,end);
  assert_eq!(selected_edit(window,fixture.root),Some(Selection::Word("runtime".into())),"range {start}..{end} in {text:?}");
 }
}
#[test] fn hidden_native_controls_reject_empty_password_foreign_and_invalid_ranges() {
 let mut fixture=Fixture::new();
 for control in ["rich","edit"] {
  let window=fixture.select(control,"my runtime notes",3,10);
  assert_eq!(selected_edit(window,fixture.root),Some(Selection::Word("runtime".into())));
  assert_eq!(selected_edit(window,fixture.root+1),Some(Selection::SourceMismatch));
  let window=fixture.select(control,"my runtime notes",3,3);assert_eq!(selected_edit(window,fixture.root),Some(Selection::NoSelection));
  let window=fixture.select(control,"one\r\ntwo",0,if control=="rich"{7}else{8});
  assert_eq!(selected_edit(window,fixture.root),Some(Selection::InvalidWord));
 }
 let window=fixture.select("password","secretword",0,10);
 assert_eq!(selected_edit(window,fixture.root),Some(Selection::Protected));
 assert_eq!(selected_edit(fixture.root,fixture.root),None,"container titles are not selection text");
}

#[test] fn hidden_rich_edit_reads_selection_across_a_soft_wrapped_line() { unsafe {
 let mut fixture=Fixture::new();
 let text="workspace".repeat(30);
 // Scalar-only message sent to our own hidden control in its child process.
 // With no target device and width zero, RichEdit wraps to its client width.
 let window=fixture.select("rich",&text,0,0);
 SendMessageW(window,0x0448,0,0); // EM_SETTARGETDEVICE
 let boundary=message(window,EM_LINEINDEX,1,0).unwrap();
 assert!(boundary>3&&boundary+5<text.len(),"fixture must have a real soft wrap: {boundary}");
 let(start,end)=(boundary-3,boundary+5);
 SendMessageW(window,0x00b1,start,end as isize);
 assert_eq!(selected_edit(window,fixture.root),Some(Selection::Word(text[start..end].into())));
 assert_eq!(range(window).unwrap(),(start as u32,end as u32));
} }
#[test] fn hidden_rich_edit_returns_exact_text_or_a_bounded_rejection_for_a_very_long_line() { unsafe {
 let mut fixture=Fixture::new();
 let text=format!("{}runtime"," ".repeat(70_000));
 let window=fixture.select("rich",&text,70_000,70_007);
 SendMessageW(window,0x0448,0,1); // EM_SETTARGETDEVICE: no wrapping
 assert_eq!(range(window).unwrap(),(70_000,70_007),"fixture must retain full DWORD offsets");
 // RichEdit may internally split a very long paragraph even without wrapping.
 let result=selected_edit(window,fixture.root);
 assert!(matches!(&result,Some(Selection::Word(word)) if word=="runtime") || result==Some(Selection::Unsupported),"expected the exact selection or a safe bounded rejection, got {result:?}");
 assert_eq!(range(window).unwrap(),(70_000,70_007));
} }
