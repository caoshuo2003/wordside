//! System registration lives on the capture thread's message queue.
use super::shortcut::{Hotkey,HotkeyStatus};
use std::sync::{Arc,Mutex,mpsc::{sync_channel,channel,SyncSender,Sender,Receiver}};
use std::time::Duration;
#[derive(Default)]#[repr(C)]struct Msg{window:isize,message:u32,wparam:usize,lparam:isize,time:u32,x:i32,y:i32,private:u32}
#[derive(Default,Clone,Copy)]#[repr(C)]struct Rect{left:i32,top:i32,right:i32,bottom:i32}
#[derive(Default)]#[repr(C)]struct GuiInfo{size:u32,flags:u32,active:isize,focus:isize,capture:isize,menu:isize,moving:isize,caret:isize,rect:Rect}
#[derive(Clone,Copy)]pub struct Trigger{pub window:isize,pub focus:isize,pub x:i32,pub y:i32}
#[link(name="user32")]extern "system"{
 fn RegisterHotKey(window:isize,id:i32,modifiers:u32,key:u32)->i32;fn UnregisterHotKey(window:isize,id:i32)->i32;
 fn PeekMessageW(message:*mut Msg,window:isize,min:u32,max:u32,remove:u32)->i32;fn PostThreadMessageW(thread:u32,message:u32,wparam:usize,lparam:isize)->i32;
 fn MsgWaitForMultipleObjectsEx(count:u32,handles:*const isize,timeout:u32,wake:u32,flags:u32)->u32;
 fn GetForegroundWindow()->isize;fn GetCursorPos(point:*mut Point)->i32;fn GetGUIThreadInfo(thread:u32,info:*mut GuiInfo)->i32;fn GetWindowThreadProcessId(window:isize,pid:*mut u32)->u32;
}
#[derive(Default)]#[repr(C)]struct Point{x:i32,y:i32}
#[link(name="kernel32")]extern "system"{fn GetCurrentThreadId()->u32;fn GetLastError()->u32;}
const WAKE:u32=0x8000+0x42;
enum Command{Prepare(Option<Hotkey>,Sender<Result<u64,String>>),Finish(u64,bool,Option<Sender<Result<(),String>>>)}
#[derive(Default)]pub struct Control{endpoint:Mutex<Option<(SyncSender<Command>,u32)>>,status:Mutex<HotkeyStatus>}
impl Control{
 pub fn set_error(&self,error:String){self.status.lock().unwrap().error=Some(error);}
 pub fn status(&self)->HotkeyStatus{self.status.lock().unwrap().clone()}
 fn send(&self,command:Command)->Result<(),String>{
  let endpoint=self.endpoint.lock().unwrap();let(sender,thread)=endpoint.as_ref().ok_or("快捷键服务正在启动，请稍后重试。")?;
  sender.try_send(command).map_err(|_|"快捷键服务忙，请稍后重试。")?;
  if unsafe{PostThreadMessageW(*thread,WAKE,0,0)}==0{return Err("快捷键服务未响应，请重新启动 WordSide。".into());}Ok(())
 }
 pub fn prepare(self:&Arc<Self>,key:Option<Hotkey>)->Result<Prepared,String>{
  let(tx,rx)=channel();self.send(Command::Prepare(key,tx))?;
  let token=rx.recv_timeout(Duration::from_secs(2)).map_err(|_|"快捷键更新超时，请稍后重试。")??;
  Ok(Prepared{control:self.clone(),token,finished:false})
 }
}
pub struct Prepared{control:Arc<Control>,token:u64,finished:bool}
impl Prepared{
 pub fn commit(mut self)->Result<(),String>{let(tx,rx)=channel();self.control.send(Command::Finish(self.token,true,Some(tx)))?;let result=rx.recv_timeout(Duration::from_secs(2)).map_err(|_|"设置已保存，但快捷键状态未确认；请重新启动 WordSide。")?;self.finished=true;result}
}
impl Drop for Prepared{fn drop(&mut self){if !self.finished{let _=self.control.send(Command::Finish(self.token,false,None));}}}
struct Registered{id:i32,key:Hotkey}
struct Pending{token:u64,next:Option<Registered>,unchanged:bool}
trait Registry{fn register(&self,id:i32,key:&Hotkey)->Result<(),u32>;fn unregister(&self,id:i32);}
struct SystemRegistry;
impl Registry for SystemRegistry{
 fn register(&self,id:i32,key:&Hotkey)->Result<(),u32>{if unsafe{RegisterHotKey(0,id,key.modifiers|0x4000,key.key)}!=0{Ok(())}else{Err(unsafe{GetLastError()})}}
 fn unregister(&self,id:i32){unsafe{UnregisterHotKey(0,id);}}
}
pub struct Driver{registry:Box<dyn Registry>,control:Arc<Control>,commands:Receiver<Command>,active:Option<Registered>,pending:Option<Pending>,serial:u64}
impl Driver{
 pub fn new(control:Arc<Control>,initial:Option<Hotkey>)->Self{Self::with_registry(control,initial,Box::new(SystemRegistry))}
 fn with_registry(control:Arc<Control>,initial:Option<Hotkey>,registry:Box<dyn Registry>)->Self{
  unsafe{let mut message=Msg::default();PeekMessageW(&mut message,0,0,0,0);}
  let(tx,commands)=sync_channel(8);*control.endpoint.lock().unwrap()=Some((tx,unsafe{GetCurrentThreadId()}));
  let mut driver=Self{registry,control,commands,active:None,pending:None,serial:0};
  match driver.prepare(initial){Ok(token)=>{let _=driver.finish(token,true);},Err(error)=>{driver.control.status.lock().unwrap().error=Some(error);}}
  driver
 }
 fn update_status(&self){*self.control.status.lock().unwrap()=HotkeyStatus{active:self.active.is_some(),shortcut:self.active.as_ref().map(|r|r.key.text.clone()).unwrap_or_default(),error:None};}
 fn prepare(&mut self,key:Option<Hotkey>)->Result<u64,String>{
  if self.pending.is_some(){return Err("另一个快捷键设置正在保存，请稍后重试。".into());}
  let unchanged=self.active.as_ref().map(|r|&r.key)==key.as_ref();let mut next=None;
  if !unchanged{if let Some(key)=key{
   let id=if self.active.as_ref().is_some_and(|r|r.id==0x4101){0x4102}else{0x4101};
   if let Err(code)=self.registry.register(id,&key){
    let reason=if code==1409{"这组快捷键已被其他程序占用，请换一组。"}else{"无法注册这组快捷键，请换一组。"};return Err(format!("{reason}{}",if self.active.is_some(){"原快捷键仍保留。"}else{""}));
   }next=Some(Registered{id,key});
  }}
  self.serial=self.serial.wrapping_add(1);let token=self.serial;self.pending=Some(Pending{token,next,unchanged});Ok(token)
 }
 fn finish(&mut self,token:u64,commit:bool)->Result<(),String>{
  if !self.pending.as_ref().is_some_and(|p|p.token==token){return Err("快捷键设置已过期，请重新保存。".into());}
  let pending=self.pending.take().unwrap();
  if commit&&!pending.unchanged{if let Some(old)=self.active.take(){self.registry.unregister(old.id);}self.active=pending.next;}
  else if let Some(next)=pending.next{self.registry.unregister(next.id);}
  if commit{self.update_status();}Ok(())
 }
 pub fn input_signature(&self)->u64{self.active.as_ref().map(|value|u64::from(value.key.key)|(u64::from(value.key.modifiers)<<32)).unwrap_or(0)}
 pub fn matches_input(&self,key:u32,modifiers:u32)->bool{self.active.as_ref().is_some_and(|value|value.key.key==key&&value.key.modifiers==modifiers)}
 pub fn drain(&mut self)->Vec<Trigger>{
  while let Ok(command)=self.commands.try_recv(){match command{
   Command::Prepare(key,reply)=>{let result=self.prepare(key);let token=result.as_ref().ok().copied();if reply.send(result).is_err(){if let Some(token)=token{let _=self.finish(token,false);}}},
   Command::Finish(token,commit,reply)=>{let result=self.finish(token,commit);if let Some(reply)=reply{let _=reply.send(result);}},
  }}
  let mut events=Vec::new();unsafe{let mut message=Msg::default();while PeekMessageW(&mut message,0,0,0,1)!=0{
   if message.message!=0x312{continue;}let Some(active)=self.active.as_ref()else{continue;};
   if message.wparam!=active.id as usize||((message.lparam as u32>>16)&0xffff)!=active.key.key||message.lparam as u32&0xf!=active.key.modifiers{continue;}
   let window=GetForegroundWindow();let mut point=Point::default();GetCursorPos(&mut point);
   let mut pid=0;let thread=GetWindowThreadProcessId(window,&mut pid);let mut gui=GuiInfo{size:std::mem::size_of::<GuiInfo>() as u32,..GuiInfo::default()};
   let focus=if GetGUIThreadInfo(thread,&mut gui)!=0{gui.focus}else{0};events.push(Trigger{window,focus,x:point.x,y:point.y});
  }}events
 }
 pub fn wait(timeout:u64){unsafe{MsgWaitForMultipleObjectsEx(0,std::ptr::null(),timeout.min(u64::from(u32::MAX)) as u32,0x04ff,0x0004);}}
}
impl Drop for Driver{fn drop(&mut self){if let Some(active)=self.active.take(){self.registry.unregister(active.id);}if let Some(pending)=self.pending.take(){if let Some(next)=pending.next{self.registry.unregister(next.id);}}*self.control.endpoint.lock().unwrap()=None;}}

#[cfg(test)]mod tests{
 use super::*;use std::{rc::Rc,cell::RefCell,collections::HashMap};
 #[derive(Default)]struct Reservations{keys:HashMap<i32,Hotkey>}
 struct FakeRegistry(Rc<RefCell<Reservations>>);
 impl Registry for FakeRegistry{
  fn register(&self,id:i32,key:&Hotkey)->Result<(),u32>{let mut state=self.0.borrow_mut();if state.keys.values().any(|value|value==key){return Err(1409);}state.keys.insert(id,key.clone());Ok(())}
  fn unregister(&self,id:i32){self.0.borrow_mut().keys.remove(&id);}
 }
 fn key(value:&str)->Hotkey{super::super::shortcut::parse(value).unwrap().unwrap()}
 #[test]fn registration_is_transactional_and_conflict_retains_the_old_key(){
  let reservations=Rc::new(RefCell::new(Reservations::default()));let control=Arc::new(Control::default());
  let mut driver=Driver::with_registry(control.clone(),Some(key("Ctrl+Alt+D")),Box::new(FakeRegistry(reservations.clone())));
  assert_eq!(control.status().shortcut,"Ctrl+Alt+D");
  let pending=driver.prepare(Some(key("Ctrl+Shift+F9"))).unwrap();assert_eq!(reservations.borrow().keys.len(),2);assert_eq!(control.status().shortcut,"Ctrl+Alt+D");
  driver.finish(pending,false).unwrap();assert_eq!(reservations.borrow().keys.len(),1);assert_eq!(control.status().shortcut,"Ctrl+Alt+D");
  reservations.borrow_mut().keys.insert(123,key("Ctrl+Alt+K"));assert!(driver.prepare(Some(key("Ctrl+Alt+K"))).unwrap_err().contains("占用"));assert_eq!(control.status().shortcut,"Ctrl+Alt+D");
  let pending=driver.prepare(Some(key("Ctrl+Shift+F9"))).unwrap();driver.finish(pending,true).unwrap();assert_eq!(control.status().shortcut,"Ctrl+Shift+F9");assert_eq!(reservations.borrow().keys.len(),2);
  let pending=driver.prepare(None).unwrap();driver.finish(pending,true).unwrap();assert!(!control.status().active);assert_eq!(reservations.borrow().keys.len(),1);
  drop(driver);assert_eq!(reservations.borrow().keys.len(),1,"external registrations are untouched");
 }
 #[test]fn recognized_hotkey_input_is_independent_of_automatic_mode(){
  let reservations=Rc::new(RefCell::new(Reservations::default()));let control=Arc::new(Control::default());
  let driver=Driver::with_registry(control,Some(key("Alt+D")),Box::new(FakeRegistry(reservations)));
  assert!(driver.matches_input(0x44,1));assert!(!driver.matches_input(0x44,0));assert!(!driver.matches_input(0x44,3));assert!(!driver.matches_input(0x43,1));
 }
 #[test]fn same_key_does_not_duplicate_registration_and_stale_tokens_are_rejected(){
  let reservations=Rc::new(RefCell::new(Reservations::default()));let control=Arc::new(Control::default());
  let mut driver=Driver::with_registry(control.clone(),Some(key("Alt+F8")),Box::new(FakeRegistry(reservations.clone())));
  let pending=driver.prepare(Some(key("Alt+F8"))).unwrap();assert_eq!(reservations.borrow().keys.len(),1);assert!(driver.finish(pending.wrapping_add(1),true).is_err());driver.finish(pending,true).unwrap();
  drop(driver);assert!(reservations.borrow().keys.is_empty());
 }
}
