//! A recoverable UIA client. No Tauri windows, clipboard or input in the helper.
use super::{windows_text::{TextRequest,selected_text},selection::Selection};
use serde::{Serialize,Deserialize};
use std::{io::{BufRead,BufReader,Read,Write},process::{Child,ChildStdin,Command,Stdio},sync::{Arc,Mutex,Condvar,mpsc::{channel,Receiver},atomic::{AtomicU64,Ordering}},thread::{self,JoinHandle},time::{Duration,Instant}};
use std::os::windows::{process::CommandExt,io::AsRawHandle};
use ::windows::{core::Interface,Win32::{Foundation::{CloseHandle,HANDLE},System::{Com::{CoInitializeEx,CoUninitialize,CoCreateInstance,COINIT_MULTITHREADED,CLSCTX_INPROC_SERVER},JobObjects::{CreateJobObjectW,SetInformationJobObject,AssignProcessToJobObject,JOBOBJECT_EXTENDED_LIMIT_INFORMATION,JobObjectExtendedLimitInformation,JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE}},UI::Accessibility::{IUIAutomation,IUIAutomation2,CUIAutomation}}};
const PREFIX:&str="WORDSIDEPROBE ";
#[derive(Clone,Serialize,Deserialize)]pub struct ProbeRequest{pub serial:u64,pub text:TextRequest}
#[derive(Serialize,Deserialize)]pub struct ProbeResult{pub serial:u64,pub selection:Selection}
struct Job(HANDLE);
impl Job{
 fn own(child:&Child)->Result<Self,String>{unsafe{
  let handle=CreateJobObjectW(None,None).map_err(|_|"Create selection job")?;let job=Self(handle);
  let mut info=JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();info.BasicLimitInformation.LimitFlags=JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
  SetInformationJobObject(handle,JobObjectExtendedLimitInformation,&info as *const _ as _,std::mem::size_of_val(&info) as u32).map_err(|_|"Configure selection job")?;
  AssignProcessToJobObject(handle,HANDLE(child.as_raw_handle())).map_err(|_|"Own selection helper")?;Ok(job)
 }}
}
impl Drop for Job{fn drop(&mut self){unsafe{let _=CloseHandle(self.0);}}}
struct Helper{child:Child,input:ChildStdin,replies:Receiver<Result<ProbeResult,()>>,reader:Option<JoinHandle<()>>,_job:Option<Job>}
impl Helper{
 fn spawn(program:&std::path::Path,args:&[String])->Result<Self,String>{
  let mut child=Command::new(program).args(args).creation_flags(0x08000000).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|_|"Start selection helper")?;
  let job=match Job::own(&child){Ok(job)=>job,Err(error)=>{let _=child.kill();let _=child.wait();return Err(error);}};
  let input=child.stdin.take().ok_or("Helper input")?;let output=child.stdout.take().ok_or("Helper output")?;
  let(tx,replies)=channel();let reader=thread::spawn(move||{
   let mut output=BufReader::new(output);
   loop{
    let mut line=Vec::new();match output.by_ref().take(8192).read_until(b'\n',&mut line){Ok(0)|Err(_)=>break,Ok(_)=>{}}
    if line.len()>=8192&&!line.ends_with(b"\n"){let _=tx.send(Err(()));break;}
    if let Some(json)=line.strip_prefix(PREFIX.as_bytes()){
     let value=serde_json::from_slice(json).map_err(|_|());let failed=value.is_err();if tx.send(value).is_err()||failed{break;}
    }
   }
  });
  Ok(Self{child,input,replies,reader:Some(reader),_job:Some(job)})
 }
 fn execute(&mut self,request:&ProbeRequest,latest:&AtomicU64,timeout:Duration)->Option<Selection>{
  let payload=serde_json::to_vec(request).ok()?;
  if self.input.write_all(&payload).and_then(|_|self.input.write_all(b"\n")).and_then(|_|self.input.flush()).is_err(){return Some(Selection::ReadError("Selection helper input".into()));}
  let until=Instant::now()+timeout;
  loop{
   if latest.load(Ordering::SeqCst)!=request.serial{return None;}
   let remaining=until.saturating_duration_since(Instant::now());if remaining.is_zero(){return Some(Selection::Timeout);}
   match self.replies.recv_timeout(remaining.min(Duration::from_millis(15))){
    Ok(Ok(reply))if reply.serial==request.serial=>return Some(reply.selection),
    Ok(_)=>return Some(Selection::ReadError("Selection helper response".into())),
    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)=>return Some(Selection::ReadError("Selection helper exited".into())),
    Err(std::sync::mpsc::RecvTimeoutError::Timeout)=>{},
   }
  }
 }
}
impl Drop for Helper{fn drop(&mut self){self._job.take();let _=self.child.kill();let _=self.child.wait();if let Some(reader)=self.reader.take(){let _=reader.join();}}}
#[derive(Default)]struct Mailbox{pending:Option<ProbeRequest>,closed:bool}
pub struct Worker{mailbox:Arc<(Mutex<Mailbox>,Condvar)>,latest:Arc<AtomicU64>,join:Option<JoinHandle<()>>}
impl Worker{
 pub fn new()->(Self,Receiver<ProbeResult>){Self::start(std::env::current_exe().unwrap_or_default(),vec!["--wordside-selection-helper".into()],Duration::from_millis(900),Duration::from_secs(5))}
 fn start(program:std::path::PathBuf,args:Vec<String>,timeout:Duration,idle:Duration)->(Self,Receiver<ProbeResult>){
  let mailbox=Arc::new((Mutex::new(Mailbox::default()),Condvar::new()));let latest=Arc::new(AtomicU64::new(0));
  let work=mailbox.clone();let current=latest.clone();let(tx,answers)=channel();
  let join=thread::spawn(move||{
   let mut helper:Option<Helper>=None;
   loop{
    let(lock,wake)=&*work;let mut guard=lock.lock().unwrap();
    while guard.pending.is_none()&&!guard.closed{
     let(state,wait)=wake.wait_timeout(guard,idle).unwrap();guard=state;if wait.timed_out(){helper=None;}
    }
    if guard.closed{break;}let request=guard.pending.take().unwrap();drop(guard);
    if current.load(Ordering::SeqCst)!=request.serial{continue;}
    let started=Instant::now();
    if helper.is_none(){match Helper::spawn(&program,&args){Ok(value)=>helper=Some(value),Err(error)=>{let _=tx.send(ProbeResult{serial:request.serial,selection:Selection::ReadError(error)});continue;}}}
    let outcome=helper.as_mut().unwrap().execute(&request,&current,timeout.saturating_sub(started.elapsed()));
    if outcome.is_none()||matches!(outcome,Some(Selection::Timeout|Selection::ReadError(_))){helper=None;}
    if current.load(Ordering::SeqCst)==request.serial{if let Some(selection)=outcome{let _=tx.send(ProbeResult{serial:request.serial,selection});}}
   }
  });
  (Self{mailbox,latest,join:Some(join)},answers)
 }
 pub fn submit(&self,request:ProbeRequest){let mut guard=self.mailbox.0.lock().unwrap();self.latest.store(request.serial,Ordering::SeqCst);guard.pending=Some(request);self.mailbox.1.notify_one();}
 pub fn cancel(&self,serial:u64){let mut guard=self.mailbox.0.lock().unwrap();self.latest.store(serial,Ordering::SeqCst);guard.pending=None;self.mailbox.1.notify_one();}
}
impl Drop for Worker{fn drop(&mut self){self.latest.fetch_add(1,Ordering::SeqCst);self.mailbox.0.lock().unwrap().closed=true;self.mailbox.1.notify_one();if let Some(join)=self.join.take(){let _=join.join();}}}
fn serve(mut read:impl FnMut(&IUIAutomation,&TextRequest)->Selection){unsafe{
 if CoInitializeEx(None,COINIT_MULTITHREADED).is_err(){return;}
 {
  let automation:Result<IUIAutomation,_>=CoCreateInstance(&CUIAutomation,None,CLSCTX_INPROC_SERVER);
  if let Ok(automation)=automation{
   if let Ok(timed)=automation.cast::<IUIAutomation2>(){let _=timed.SetConnectionTimeout(500);let _=timed.SetTransactionTimeout(500);}
   let stdin=std::io::stdin();let mut input=stdin.lock();
   loop{
    let mut line=Vec::new();if input.by_ref().take(4096).read_until(b'\n',&mut line).ok().filter(|n|*n>0).is_none()||line.len()>=4096{break;}
    let Ok(request)=serde_json::from_slice::<ProbeRequest>(&line)else{break;};
    let reply=ProbeResult{serial:request.serial,selection:read(&automation,&request.text)};
    let Ok(json)=serde_json::to_string(&reply)else{break;};let mut output=std::io::stdout().lock();if writeln!(output,"{PREFIX}{json}").and_then(|_|output.flush()).is_err(){break;}
   }
  }
 }
 CoUninitialize();
}}
pub fn helper_entry()->bool{if std::env::args().nth(1).as_deref()!=Some("--wordside-selection-helper"){return false;}serve(selected_text);true}
#[cfg(test)]mod tests{
 use super::*;use super::super::windows_text::{TextOrigin,tests::ExternalFixture};
 #[test]#[ignore="isolated helper entry used by recovery tests"]fn helper_process(){serve(|automation,request|{if request.pid==u32::MAX{thread::sleep(Duration::from_secs(60));}selected_text(automation,request)});}
 #[test]fn hung_helper_is_reaped_and_next_real_selection_recovers(){
  let fixture=ExternalFixture::new();let(worker,answers)=Worker::start(std::env::current_exe().unwrap(),vec!["--ignored".into(),"--exact".into(),"capture::probe::tests::helper_process".into(),"--nocapture".into()],Duration::from_millis(1200),Duration::from_millis(200));
  let request=|serial,pid|ProbeRequest{serial,text:TextRequest{pid,window:fixture.edit,origin:TextOrigin::Window,focus:0}};
  worker.submit(request(1,u32::MAX));assert_eq!(answers.recv_timeout(Duration::from_secs(4)).unwrap().selection,Selection::Timeout);
  worker.submit(request(2,fixture.pid));assert_eq!(answers.recv_timeout(Duration::from_secs(4)).unwrap().selection,Selection::Word("terminal".into()));
  thread::sleep(Duration::from_millis(300));worker.submit(request(3,fixture.pid));assert_eq!(answers.recv_timeout(Duration::from_secs(4)).unwrap().selection,Selection::Word("terminal".into()));
  // Superseding a blocked request hard-stops it instead of waiting for its deadline.
  worker.submit(request(4,u32::MAX));thread::sleep(Duration::from_millis(150));let at=Instant::now();worker.submit(request(5,fixture.pid));
  let reply=answers.recv_timeout(Duration::from_secs(4)).unwrap();assert_eq!(reply.serial,5);assert_eq!(reply.selection,Selection::Word("terminal".into()));assert!(at.elapsed()<Duration::from_millis(1200));
  worker.submit(ProbeRequest{serial:6,text:TextRequest{pid:fixture.pid,window:fixture.rename,origin:TextOrigin::Window,focus:0}});
  let reply=answers.recv_timeout(Duration::from_secs(4)).unwrap();assert_eq!(reply.serial,6);assert_eq!(reply.selection,Selection::Word("runtime".into()),"the recovered helper must support ordinary rename EDIT controls");
  drop(worker);
 }
}
