//! Recoverable UIA/text reader and isolated clipboard backup/restore. The parent owns the transaction.
use super::{windows_text::{TextRequest,selected_text},selection::Selection};
use serde::{Serialize,Deserialize};
use std::{io::{BufRead,BufReader,Read,Write},process::{Child,ChildStdin,Command,Stdio},sync::{Arc,Mutex,Condvar,mpsc::{channel,Receiver},atomic::{AtomicU64,Ordering}},thread::{self,JoinHandle},time::{Duration,Instant}};
use std::os::windows::{process::CommandExt,io::AsRawHandle};
use ::windows::{core::Interface,Win32::{Foundation::{CloseHandle,HANDLE},System::{Com::{CoInitializeEx,CoUninitialize,CoCreateInstance,COINIT_MULTITHREADED,CLSCTX_INPROC_SERVER},JobObjects::{CreateJobObjectW,SetInformationJobObject,AssignProcessToJobObject,JOBOBJECT_EXTENDED_LIMIT_INFORMATION,JobObjectExtendedLimitInformation,JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE}},UI::Accessibility::{IUIAutomation,IUIAutomation2,CUIAutomation}}};
const PREFIX:&str="WORDSIDEPROBE ";
#[derive(Clone,Serialize,Deserialize)]pub struct ProbeRequest{pub serial:u64,pub text:TextRequest}
#[derive(Serialize,Deserialize)]pub struct ProbeResult{pub serial:u64,pub selection:Selection,#[serde(default)]pub retryable:bool,#[serde(default)]native_verified:bool,#[serde(default,skip_serializing_if="Option::is_none")]pub clipboard_sequence:Option<u32>,#[serde(default,skip_serializing_if="Option::is_none")]clipboard_failure:Option<super::windows_copy::BackupFailure>,#[serde(default,skip_serializing_if="Option::is_none")]clipboard_ole_formats:Option<usize>,#[serde(default,skip_serializing_if="Option::is_none")]clipboard_restore:Option<super::windows_copy::RestoreState>}
impl ProbeResult{fn selection(serial:u64,selection:Selection)->Self{Self{serial,selection,retryable:false,native_verified:false,clipboard_sequence:None,clipboard_failure:None,clipboard_ole_formats:None,clipboard_restore:None}}}
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
 fn exchange(&mut self,request:&ProbeRequest,latest:&AtomicU64,timeout:Duration)->Option<ProbeResult>{
  let payload=serde_json::to_vec(request).ok()?;
  if self.input.write_all(&payload).and_then(|_|self.input.write_all(b"\n")).and_then(|_|self.input.flush()).is_err(){return Some(ProbeResult::selection(request.serial,Selection::ReadError("Selection helper input".into())));}
  let until=Instant::now()+timeout;
  loop{
   if latest.load(Ordering::SeqCst)!=request.serial{return None;}
   let remaining=until.saturating_duration_since(Instant::now());if remaining.is_zero(){return Some(ProbeResult::selection(request.serial,Selection::Timeout));}
   match self.replies.recv_timeout(remaining.min(Duration::from_millis(15))){
    Ok(Ok(reply))if reply.serial==request.serial=>return Some(reply),
    Ok(_)=>return Some(ProbeResult::selection(request.serial,Selection::ReadError("Selection helper response".into()))),
    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)=>return Some(ProbeResult::selection(request.serial,Selection::ReadError("Selection helper exited".into()))),
    Err(std::sync::mpsc::RecvTimeoutError::Timeout)=>{},
   }
  }
 }
 #[cfg(test)] fn execute(&mut self,request:&ProbeRequest,latest:&AtomicU64,timeout:Duration)->Option<Selection>{self.exchange(request,latest,timeout).map(|reply|reply.selection)}
}
impl Drop for Helper{fn drop(&mut self){self._job.take();let _=self.child.kill();let _=self.child.wait();if let Some(reader)=self.reader.take(){let _=reader.join();}}}
struct Work { request: ProbeRequest, guard: Option<(Arc<AtomicU64>,u64)>, release: Option<super::timing::Activity>, verify: bool, deadline: Option<Instant>, point:Option<(i32,i32)> }
fn observation_budget(deadline: Option<Instant>, now: Instant) -> Option<Duration> {
 let remaining=deadline?.saturating_duration_since(now);
 (!remaining.is_zero()).then_some(remaining.min(Duration::from_millis(200)))
}
fn retry_allowed(selection: &Selection, input_attempted: bool) -> bool {
 // A new provider/helper must pass every protection and source check again.
 // A failed read itself never grants permission to copy.
 !input_attempted && matches!(selection,Selection::NoSelection|Selection::SourceMismatch|Selection::Timeout|Selection::ReadError(_))
}
// Resolve read-only provider paths before a single optional copy transaction.
fn read_provider(request:&ProbeRequest, point:Option<(i32,i32)>, current:impl Fn()->bool,
 mut read:impl FnMut(&ProbeRequest)->Option<ProbeResult>)->Option<ProbeResult> {
 if !current(){return None;}
 let focused=read(request)?;
 if !current(){return None;}
 if let Some((x,y))=point.filter(|_|matches!(request.text.origin,super::windows_text::TextOrigin::Focused)
     && matches!(focused.selection,Selection::Unsupported|Selection::Unreported)) {
  let mut pointer=request.clone();pointer.text.origin=super::windows_text::TextOrigin::Pointer(x,y);
  let answer=read(&pointer)?;
  if !current(){return None;}
  // A second provider finding no range cannot erase a positively read-only
  // document's unreported selection. It still needs the one copy check.
  // Explicit protection/source/access failures remain authoritative.
  return Some(if matches!(answer.selection,Selection::NoSelection|Selection::Unsupported|Selection::Unreported) {focused}else{answer});
 }
 Some(focused)
}
#[derive(Default)]struct Mailbox{pending:Option<Work>,observed:Option<Work>,closed:bool}
pub struct Worker{mailbox:Arc<(Mutex<Mailbox>,Condvar)>,latest:Arc<AtomicU64>,observed_latest:Arc<AtomicU64>,observations:Receiver<ProbeResult>,join:Option<JoinHandle<()>>}
impl Worker{
 pub fn new(status_path:std::path::PathBuf)->(Self,Receiver<ProbeResult>){Self::start_recorded(std::env::current_exe().unwrap_or_default(),vec!["--wordside-selection-helper".into()],Duration::from_millis(900),Duration::from_secs(5),Some(status_path))}
 #[cfg(test)]fn start(program:std::path::PathBuf,args:Vec<String>,timeout:Duration,idle:Duration)->(Self,Receiver<ProbeResult>){Self::start_recorded(program,args,timeout,idle,None)}
 fn start_recorded(program:std::path::PathBuf,args:Vec<String>,timeout:Duration,idle:Duration,status_path:Option<std::path::PathBuf>)->(Self,Receiver<ProbeResult>){
  let mailbox=Arc::new((Mutex::new(Mailbox::default()),Condvar::new()));let latest=Arc::new(AtomicU64::new(0));
  let observed_latest=Arc::new(AtomicU64::new(0));let observed_current=observed_latest.clone();
  let(observed_tx,observations)=channel();
  let work=mailbox.clone();let current=latest.clone();let(tx,answers)=channel();
  let join=thread::spawn(move||{
   let mut helper:Option<Helper>=None;let mut backup_helper:Option<Helper>=None;
   let recorder=super::diagnostics::FailureRecorder::new(status_path);
   loop{
    let(lock,wake)=&*work;let mut guard=lock.lock().unwrap();
    while guard.pending.is_none()&&guard.observed.is_none()&&!guard.closed{
     let(state,wait)=wake.wait_timeout(guard,idle).unwrap();guard=state;if wait.timed_out(){helper=None;backup_helper=None;}
    }
    if guard.closed{break;}let work_item=guard.pending.take().or_else(||guard.observed.take()).unwrap();let request=work_item.request;drop(guard);
    let observing=matches!(request.text.origin,super::windows_text::TextOrigin::ObservedClipboard(_));
    let active=if observing {&observed_current}else{&current};
    if active.load(Ordering::SeqCst)!=request.serial || work_item.guard.as_ref().is_some_and(|(generation,expected)|generation.load(Ordering::SeqCst)!=*expected){continue;}
    let started=Instant::now();
    if observing && observation_budget(work_item.deadline,started).is_none(){continue;}
    let mut report=super::diagnostics::FailureReport::new();let mut input_attempted=false;
    if helper.is_none(){match Helper::spawn(&program,&args){Ok(value)=>helper=Some(value),Err(error)=>{
     let mut reply=ProbeResult::selection(request.serial,Selection::ReadError(error));
     reply.retryable=!observing && work_item.verify && retry_allowed(&reply.selection,false);
     if !observing {report.provider="read_error";report.finish(&recorder,Some(&reply.selection),started);}
     let _=if observing {observed_tx.send(reply)}else{tx.send(reply)};continue;
    }}}
    if observing {
     // Passive copy-on-select reads used to run on the hotkey coordinator.
     // Serialize them with our transactions here, in a killable helper, so a
     // stalled delayed-rendering provider cannot swallow later key presses.
     let Some(budget)=observation_budget(work_item.deadline,Instant::now())else{continue;};
     let reply=helper.as_mut().unwrap().exchange(&request,active,budget);
     if reply.as_ref().is_none_or(|reply|matches!(reply.selection,Selection::Timeout|Selection::ReadError(_))){helper=None;}
     if active.load(Ordering::SeqCst)==request.serial && !work_item.guard.as_ref().is_some_and(|(generation,expected)|generation.load(Ordering::SeqCst)!=*expected) {
      if let Some(reply)=reply {let _=observed_tx.send(reply);}
     }
     continue;
    }
    let provider=read_provider(&request,work_item.point,||current.load(Ordering::SeqCst)==request.serial && work_item.guard.as_ref().is_none_or(|(generation,expected)|generation.load(Ordering::SeqCst)==*expected),|request|helper.as_mut().unwrap().exchange(request,&current,timeout.saturating_sub(started.elapsed())));
    let native_verified=provider.as_ref().is_some_and(|reply|reply.native_verified);
    let mut outcome=provider.map(|reply|reply.selection);
    if outcome.is_none()||matches!(outcome,Some(Selection::Timeout|Selection::ReadError(_))){helper=None;}
    report.provider=super::diagnostics::code(outcome.as_ref());
    if work_item.verify && !native_verified && outcome.as_ref().is_some_and(super::copy_policy::should_verify) {
     let valid=||current.load(Ordering::SeqCst)==request.serial && work_item.guard.as_ref()
      .is_some_and(|(generation,expected)|generation.load(Ordering::SeqCst)==*expected);
     // Backup collection can itself invoke delayed rendering in another app.
     // Isolate it before issuing any copy; keep it alive while the text reader
     // is disposable, then request bounded restoration through its own channel.
     if !valid(){report.finish(&recorder,None,started);continue;}
     // All passive reads and copy/restore now run on this same worker.
     // There is no clipboard API on the hotkey coordinator and no concurrent
     // passive materialization during a backup/copy/restore transaction.
     if !valid(){report.finish(&recorder,None,started);continue;}
     report.phase=super::diagnostics::Phase::Backup;
     if backup_helper.is_none(){backup_helper=Helper::spawn(&program,&args).ok();}
     let mut backup_request=request.clone();backup_request.text.origin=super::windows_text::TextOrigin::Backup;
     let prepared=backup_helper.as_mut().and_then(|backup|backup.exchange(&backup_request,&current,Duration::from_millis(600)));
     report.backup=prepared.as_ref().and_then(|reply|reply.clipboard_failure);
     outcome=if let Some(before)=prepared.as_ref().and_then(|reply|reply.clipboard_sequence){
      report.phase=super::diagnostics::Phase::Copy;
      let verified=super::windows_copy::verify(&request.text,before,valid,||work_item.release.as_ref().and_then(super::timing::Activity::stamp),|sequence|{
       if helper.is_none(){helper=Helper::spawn(&program,&args).ok();}
       let Some(reader)=helper.as_mut() else{return Some(super::copy_policy::CopiedText{selection:Selection::CopyUnavailable,sequence:None});};
       let mut copy_request=request.clone();copy_request.text.origin=super::windows_text::TextOrigin::Clipboard(sequence);
       // Once copying has started, cancellation suppresses publication but
       // must not discard materialization metadata needed for restoration.
       let cleanup_serial=AtomicU64::new(request.serial);
       let result=reader.exchange(&copy_request,&cleanup_serial,Duration::from_millis(350));
       if result.as_ref().is_none_or(|reply|matches!(reply.selection,Selection::Timeout|Selection::ReadError(_))){helper=None;}
       result.map(|reply|super::copy_policy::CopiedText{selection:reply.selection,sequence:reply.clipboard_sequence})
      },|sequence|{
       let Some(backup)=backup_helper.as_mut() else{return None;};
       let mut restore_request=request.clone();restore_request.text.origin=super::windows_text::TextOrigin::Restore(sequence);
       // Restoration must run even after the original serial was cancelled.
       let cleanup_serial=AtomicU64::new(request.serial);
       let result=backup.exchange(&restore_request,&cleanup_serial,Duration::from_millis(800));
       let state=result.and_then(|reply|matches!(reply.selection,Selection::NoSelection).then_some(reply.clipboard_restore).flatten());
       if state.is_none(){backup_helper=None;}
       state
      });
      input_attempted=verified.diagnostics.input_attempted();
      report.copy=Some(verified.diagnostics);verified.selection
     }else{
      let failure=prepared.map(|reply|reply.selection);
      if failure.is_none()||matches!(failure,Some(Selection::Timeout|Selection::ReadError(_))){backup_helper=None;}
      failure.map(|_|Selection::ClipboardUnavailable)
     };
    }
    report.finish(&recorder,outcome.as_ref(),started);
    // A source/focus check can reject a copy between coordinator polls. If
    // this serial is still current, silence would strand its awaiting request.
    // A superseded request or invalid input guard may be discarded; an
    // otherwise current request must always receive a terminal reply.
    if current.load(Ordering::SeqCst)==request.serial {
     let mut reply=ProbeResult::selection(request.serial,outcome.unwrap_or(Selection::SourceMismatch));
     reply.retryable=work_item.verify && retry_allowed(&reply.selection,input_attempted);
     let _=tx.send(reply);
    }
   }
  });
  (Self{mailbox,latest,observed_latest,observations,join:Some(join)},answers)
 }
 #[cfg(test)]pub fn submit(&self,request:ProbeRequest){self.enqueue(Work{request,guard:None,release:None,verify:false,deadline:None,point:None});}
 #[cfg(test)] pub fn submit_verified(&self,request:ProbeRequest,guard:Option<(Arc<AtomicU64>,u64)>,release:Option<super::timing::Activity>){self.submit_verified_at(request,guard,release,None);}
 pub fn submit_verified_at(&self,request:ProbeRequest,guard:Option<(Arc<AtomicU64>,u64)>,release:Option<super::timing::Activity>,point:Option<(i32,i32)>){self.enqueue(Work{request,verify:guard.is_some(),point,guard,release,deadline:None});}
 pub fn observe_clipboard(&self,text:TextRequest,guard:Option<(Arc<AtomicU64>,u64)>,deadline:Instant)->u64 {
  let mut mailbox=self.mailbox.0.lock().unwrap();let serial=self.observed_latest.fetch_add(1,Ordering::SeqCst).wrapping_add(1);
  mailbox.observed=Some(Work{request:ProbeRequest{serial,text},guard,release:None,verify:false,deadline:Some(deadline),point:None});self.mailbox.1.notify_one();serial
 }
 pub fn observed(&self)->impl Iterator<Item=ProbeResult>+'_ {self.observations.try_iter()}
 pub fn cancel_observed(&self,serial:u64) {
  let _=self.observed_latest.compare_exchange(serial,serial.wrapping_add(1),Ordering::SeqCst,Ordering::SeqCst);
  let mut mailbox=self.mailbox.0.lock().unwrap();if mailbox.observed.as_ref().is_some_and(|work|work.request.serial==serial){mailbox.observed=None;}
  self.mailbox.1.notify_one();
 }
 fn enqueue(&self,work:Work){let mut guard=self.mailbox.0.lock().unwrap();self.latest.store(work.request.serial,Ordering::SeqCst);guard.pending=Some(work);self.mailbox.1.notify_one();}
 pub fn cancel(&self,serial:u64){let mut guard=self.mailbox.0.lock().unwrap();self.latest.store(serial,Ordering::SeqCst);guard.pending=None;self.mailbox.1.notify_one();}
}
impl Drop for Worker{fn drop(&mut self){self.latest.fetch_add(1,Ordering::SeqCst);self.observed_latest.fetch_add(1,Ordering::SeqCst);self.mailbox.0.lock().unwrap().closed=true;self.mailbox.1.notify_one();if let Some(join)=self.join.take(){let _=join.join();}}}
fn serve(mut read:impl FnMut(&IUIAutomation,&TextRequest)->Selection){unsafe{
 let mut com_initialized=false;
 {
  // Clipboard-only helpers must not start UI Automation. Cold UIA activation
  // could consume their backup deadline even when the clipboard is simple.
  let mut automation:Option<IUIAutomation>=None;
  let stdin=std::io::stdin();let mut input=stdin.lock();let mut backup:Option<super::windows_copy::Backup>=None;
  loop{
   let mut line=Vec::new();if input.by_ref().take(4096).read_until(b'\n',&mut line).ok().filter(|n|*n>0).is_none()||line.len()>=4096{break;}
   let Ok(request)=serde_json::from_slice::<ProbeRequest>(&line)else{break;};
   let reply=match request.text.origin {
    super::windows_text::TextOrigin::Backup=>{
     let clipboard_failure=match super::windows_copy::Backup::take(){Ok(value)=>{backup=Some(value);None},Err(error)=>{backup=None;Some(error)}};
     ProbeResult{serial:request.serial,retryable:false,native_verified:false,selection:if backup.is_some(){Selection::NoSelection}else{Selection::ClipboardUnavailable},clipboard_sequence:backup.as_ref().map(|value|value.before),clipboard_failure,clipboard_ole_formats:backup.as_ref().map(|value|value.ole_formats),clipboard_restore:None}
    },
    super::windows_text::TextOrigin::Restore(sequence)=>{
     let state=backup.take().and_then(|mut value|value.restore(sequence,&request.text).ok());
     let mut reply=ProbeResult::selection(request.serial,if state.is_some(){Selection::NoSelection}else{Selection::ClipboardRestoreFailed});
     reply.clipboard_restore=state;reply
    },
    super::windows_text::TextOrigin::Clipboard(sequence)|super::windows_text::TextOrigin::ObservedClipboard(sequence)=>{
     let copied=if matches!(request.text.origin,super::windows_text::TextOrigin::ObservedClipboard(_)) {
      super::windows_copy::read_observed(&request.text,sequence)
     }else{super::windows_copy::read_copied(&request.text,sequence)};
     ProbeResult{serial:request.serial,retryable:false,native_verified:false,selection:copied.selection,clipboard_sequence:copied.sequence,clipboard_failure:None,clipboard_ole_formats:None,clipboard_restore:None}
    },
    _=>{
     let native=super::windows_text::native_selection(&request.text);
     if let Some(selection)=native {
      let mut reply=ProbeResult::selection(request.serial,selection);
      reply.native_verified=matches!(reply.selection,Selection::Word(_)); reply
     }else {
     if automation.is_none(){
      if !com_initialized { if CoInitializeEx(None,COINIT_MULTITHREADED).is_ok(){com_initialized=true;}else{break;} }
      if let Ok(created)=CoCreateInstance::<_,IUIAutomation>(&CUIAutomation,None,CLSCTX_INPROC_SERVER){
       if let Ok(timed)=created.cast::<IUIAutomation2>(){let _=timed.SetConnectionTimeout(500);let _=timed.SetTransactionTimeout(500);}
       automation=Some(created);
      }
     }
     ProbeResult::selection(request.serial,match automation.as_ref(){Some(automation)=>read(automation,&request.text),None=>Selection::ReadError("Start selection automation".into())})
     }
    },
   };
   let Ok(json)=serde_json::to_string(&reply)else{break;};let mut output=std::io::stdout().lock();if writeln!(output,"{PREFIX}{json}").and_then(|_|output.flush()).is_err(){break;}
  }
 }
 if com_initialized{CoUninitialize();}
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

#[cfg(test)]mod protocol_tests{
 use super::*;
 use super::super::windows_text::TextOrigin;
 fn request(serial:u64,pid:u32,origin:TextOrigin)->ProbeRequest{ProbeRequest{serial,text:TextRequest{pid,window:0,focus:0,origin}}}
 fn arguments()->Vec<String>{vec!["--ignored".into(),"--exact".into(),"capture::probe::protocol_tests::protocol_process".into(),"--nocapture".into()]}
 // Private synthetic IPC peer only: no COM, window, clipboard, hook or input API.
 #[test]#[ignore="synthetic subprocess for protocol/recovery tests"]fn protocol_process(){
  let stdin=std::io::stdin();
  for line in stdin.lock().lines(){
   let request:ProbeRequest=serde_json::from_str(&line.unwrap()).unwrap();
   if request.text.pid==u32::MAX{thread::sleep(Duration::from_secs(60));}
   let mut reply=ProbeResult::selection(request.serial,Selection::Word("runtime".into()));
   if request.text.pid==42 {reply.native_verified=true;}
   if request.text.pid==43 {reply.selection=Selection::ReadError("Synthetic provider restart".into());}
   match request.text.origin{
    TextOrigin::Backup=>{reply.selection=Selection::NoSelection;reply.clipboard_sequence=Some(77);},
    TextOrigin::Restore(sequence)=>{reply.selection=Selection::NoSelection;reply.clipboard_restore=Some(if sequence.is_some(){super::super::windows_copy::RestoreState::Restored}else{super::super::windows_copy::RestoreState::Skipped});},
    TextOrigin::Clipboard(sequence)|TextOrigin::ObservedClipboard(sequence)=>reply.clipboard_sequence=Some(sequence.wrapping_add(1)),
    _=>{},
   }
   println!("{PREFIX}{}",serde_json::to_string(&reply).unwrap());std::io::stdout().flush().unwrap();
  }
 }
 #[test] fn manual_provider_empty_pointer_preserves_document_verification_but_not_denials() {
  for first in [Selection::Unreported,Selection::Unsupported] {
   for second in [Selection::NoSelection,Selection::Unsupported,Selection::Unreported] {
    let request=request(1,42,TextOrigin::Focused);let mut calls=0;
    let result=read_provider(&request,Some((40,10)),||true,|request|{calls+=1;Some(ProbeResult::selection(request.serial,if calls==1 {first.clone()}else{second.clone()}))}).unwrap();
    assert_eq!(calls,2);assert_eq!(result.selection,first);assert!(super::super::copy_policy::should_verify(&result.selection));
   }
   for denied in [Selection::Protected,Selection::AccessDenied,Selection::SourceMismatch,Selection::InvalidWord] {
    let request=request(1,42,TextOrigin::Focused);let mut calls=0;
    let result=read_provider(&request,Some((40,10)),||true,|request|{calls+=1;Some(ProbeResult::selection(request.serial,if calls==1 {first.clone()}else{denied.clone()}))}).unwrap();
    assert_eq!(result.selection,denied,"pointer refusal cannot be replaced by the earlier result");
   }
  }
 }
 #[test] fn manual_provider_exact_selection_and_empty_editor_never_borrow_pointer() {
  for first in [Selection::Word("runtime".into()),Selection::NoSelection,Selection::Protected,Selection::AccessDenied,Selection::SourceMismatch,Selection::InvalidWord] {
   let request=request(1,42,TextOrigin::Focused);let mut calls=0;
   let result=read_provider(&request,Some((40,10)),||true,|request|{calls+=1;Some(ProbeResult::selection(request.serial,first.clone()))}).unwrap();
   assert_eq!(calls,1);assert_eq!(result.selection,first);
  }
 }
 #[test] fn manual_provider_new_input_before_or_during_read_never_grants_pointer_or_copy() {
  let request=request(1,42,TextOrigin::Focused);let mut calls=0;
  assert!(read_provider(&request,Some((40,10)),||false,|_|{calls+=1;None}).is_none());assert_eq!(calls,0);
  let current=std::cell::Cell::new(true);
  assert!(read_provider(&request,Some((40,10)),||current.get(),|request|{calls+=1;current.set(false);Some(ProbeResult::selection(request.serial,Selection::Unsupported))}).is_none());
  assert_eq!(calls,1);
 }
 #[test]fn manual_regression_bound_selection_position_is_read_before_copy(){
  let request=request(1,42,TextOrigin::Focused); let mut paths=Vec::new();
  let result=read_provider(&request,Some((40,10)),||true,|request|{
   paths.push(request.text.origin);
   let selection=if matches!(request.text.origin,TextOrigin::Pointer(40,10)){Selection::Word("runtime".into())}else{Selection::Unsupported};
   Some(ProbeResult::selection(request.serial,selection))
  }).unwrap();
  assert_eq!(result.selection,Selection::Word("runtime".into()),"manual lookup must use the same authorized selection position as automatic lookup, before copying");
  assert_eq!(paths.len(),2);
 }
 #[test]fn capture_race_native_selection_completes_without_clipboard_backup_or_input(){
  let(worker,answers)=Worker::start(std::env::current_exe().unwrap(),arguments(),Duration::from_millis(300),Duration::from_millis(300));
  // HWND=0 would reject a real copy transaction; native provenance must keep
  // this path independent of any backup, clipboard state or synthetic input.
  worker.submit_verified(request(1,42,TextOrigin::Focused),Some((Arc::new(AtomicU64::new(0)),0)),None);
  let reply=answers.recv_timeout(Duration::from_secs(3)).unwrap();
  assert_eq!(reply.selection,Selection::Word("runtime".into())); assert!(!reply.retryable);
 }
 #[test]fn notepad_regression_source_rejection_completes_the_current_request_without_a_second_hotkey(){
  let(worker,answers)=Worker::start(std::env::current_exe().unwrap(),arguments(),Duration::from_millis(300),Duration::from_millis(300));
  let generation=Arc::new(AtomicU64::new(0));
  // The private peer supplies a synthetic backup. PID zero makes the native
  // transaction reject before sending input or accessing the system clipboard.
  worker.submit_verified(request(1,0,TextOrigin::Focused),Some((generation.clone(),0)),Some(super::super::timing::Activity::default()));
  let reply=answers.recv_timeout(Duration::from_secs(3)).expect("an unchanged serial must receive a terminal result after source rejection");
  assert_eq!(reply.serial,1); assert_eq!(reply.selection,Selection::SourceMismatch); assert!(reply.retryable,"source rejection before copying can be re-read");
  worker.submit(request(2,1,TextOrigin::Focused));
  assert_eq!(answers.recv_timeout(Duration::from_secs(3)).unwrap().selection,Selection::Word("runtime".into()));
 }
 #[test]fn zotero_regression_provider_error_can_restart_without_copy_authorization(){
  let(worker,answers)=Worker::start(std::env::current_exe().unwrap(),arguments(),Duration::from_millis(300),Duration::from_millis(300));
  let guard=Arc::new(AtomicU64::new(0));
  worker.submit_verified(request(1,43,TextOrigin::Focused),Some((guard.clone(),0)),None);
  let reply=answers.recv_timeout(Duration::from_secs(3)).unwrap();
  assert!(matches!(reply.selection,Selection::ReadError(_)));
  assert!(reply.retryable,"a provider failure before any copy may be read once again");
  assert!(!super::super::copy_policy::should_verify(&reply.selection),"the error itself must never authorize a copy");
  worker.submit_verified(request(2,42,TextOrigin::Focused),Some((guard,0)),None);
  let reply=answers.recv_timeout(Duration::from_secs(3)).unwrap();
  assert_eq!(reply.selection,Selection::Word("runtime".into()));assert!(!reply.retryable);
 }
 #[test]fn zotero_regression_helper_start_failure_has_a_terminal_retryable_reply(){
  let absent=std::env::current_exe().unwrap().with_file_name("wordside-absent-synthetic-helper.exe");assert!(!absent.exists());
  let(worker,answers)=Worker::start(absent,vec![],Duration::from_millis(300),Duration::from_millis(300));
  worker.submit_verified(request(1,43,TextOrigin::Focused),Some((Arc::new(AtomicU64::new(0)),0)),None);
  let reply=answers.recv_timeout(Duration::from_secs(3)).unwrap();
  assert!(matches!(reply.selection,Selection::ReadError(_)));assert!(reply.retryable);
 }
 #[test]fn notepad_regression_copy_attempts_never_authorize_another_transaction(){
  for selection in [Selection::NoSelection,Selection::SourceMismatch,Selection::Timeout,Selection::ReadError("provider".into())] {
   assert!(retry_allowed(&selection,false));
   assert!(!retry_allowed(&selection,true),"a copied/partially copied/timed-out read must never repeat the copy or backup");
  }
  for selection in [Selection::Protected,Selection::AccessDenied,Selection::InvalidWord,Selection::ClipboardRestoreFailed,Selection::CopyUnavailable] {
   assert!(!retry_allowed(&selection,false));
  }
 }
 #[test]fn restore_peer_preserves_completed_and_safely_skipped_states(){
  use super::super::windows_copy::RestoreState;
  let mut peer=Helper::spawn(&std::env::current_exe().unwrap(),&arguments()).unwrap();let latest=AtomicU64::new(1);
  for (sequence,state) in [(Some(78),RestoreState::Restored),(None,RestoreState::Skipped)] {
   let reply=peer.exchange(&request(1,1,TextOrigin::Restore(sequence)),&latest,Duration::from_secs(2)).unwrap();
   assert_eq!(reply.selection,Selection::NoSelection);assert_eq!(reply.clipboard_restore,Some(state));
  }
 }
 #[test]fn blocked_backup_peer_is_reaped_before_copy_and_next_peer_recovers(){
  let program=std::env::current_exe().unwrap();let latest=AtomicU64::new(1);
  let mut peer=Helper::spawn(&program,&arguments()).unwrap();
  let at=Instant::now();
  let reply=peer.exchange(&request(1,u32::MAX,TextOrigin::Backup),&latest,Duration::from_millis(150)).unwrap();
  assert_eq!(reply.selection,Selection::Timeout);assert_eq!(reply.clipboard_sequence,None);
  drop(peer);assert!(at.elapsed()<Duration::from_secs(2));
  let mut peer=Helper::spawn(&program,&arguments()).unwrap();
  let reply=peer.exchange(&request(1,1,TextOrigin::Backup),&latest,Duration::from_secs(2)).unwrap();
  assert_eq!(reply.clipboard_sequence,Some(77));
  assert_eq!(peer.execute(&request(1,1,TextOrigin::Restore(Some(78))),&latest,Duration::from_secs(2)),Some(Selection::NoSelection));
 }
 #[test]fn separate_reader_cancellation_does_not_destroy_backup_and_cleanup_ignores_old_serial(){
  let program=std::env::current_exe().unwrap();let serial=AtomicU64::new(3);
  let mut backup=Helper::spawn(&program,&arguments()).unwrap();
  assert_eq!(backup.exchange(&request(3,1,TextOrigin::Backup),&serial,Duration::from_secs(2)).unwrap().clipboard_sequence,Some(77));
  let mut reader=Helper::spawn(&program,&arguments()).unwrap();
  assert_eq!(reader.execute(&request(3,u32::MAX,TextOrigin::Clipboard(78)),&serial,Duration::from_millis(150)),Some(Selection::Timeout));
  drop(reader);serial.store(4,Ordering::SeqCst);
  let cleanup=AtomicU64::new(3);
  assert_eq!(backup.execute(&request(3,1,TextOrigin::Restore(Some(78))),&cleanup,Duration::from_secs(2)),Some(Selection::NoSelection));
 }
 #[test]fn copied_text_reply_preserves_the_final_materialized_sequence(){
  let mut reader=Helper::spawn(&std::env::current_exe().unwrap(),&arguments()).unwrap();let serial=AtomicU64::new(5);
  serial.store(6,Ordering::SeqCst); // The UI has already cancelled request 5.
  let cleanup_serial=AtomicU64::new(5);
  let reply=reader.exchange(&request(5,1,TextOrigin::Clipboard(78)),&cleanup_serial,Duration::from_secs(2)).unwrap();
  assert_eq!(reply.selection,Selection::Word("runtime".into()));assert_eq!(reply.clipboard_sequence,Some(79));
  assert_ne!(reply.serial,serial.load(Ordering::SeqCst),"cancelled results are cleanup evidence only");
 }
 #[test]fn helper_start_failure_replaces_the_previous_failure_snapshot(){
  let temp=tempfile::tempdir().unwrap();let path=temp.path().join("capture-status.json");
  std::fs::write(&path,"previous_failure").unwrap();
  let(worker,answers)=Worker::start_recorded(temp.path().join("missing-helper.exe"),arguments(),Duration::from_millis(300),Duration::from_millis(300),Some(path.clone()));
  worker.submit(request(1,1,TextOrigin::Focused));
  assert!(matches!(answers.recv_timeout(Duration::from_secs(2)).unwrap().selection,Selection::ReadError(_)));
  let saved=std::fs::read_to_string(path).unwrap();let report:serde_json::Value=serde_json::from_str(&saved).unwrap();
  assert_eq!(report["phase"],"provider");assert_eq!(report["provider"],"read_error");assert_eq!(report["outcome"],"read_error");assert!(!saved.contains("missing-helper"));
 }
 #[test]fn passive_budget_never_restarts_the_original_gesture_deadline(){
  let now=Instant::now();
  assert_eq!(observation_budget(Some(now+Duration::from_millis(350)),now),Some(Duration::from_millis(200)));
  assert_eq!(observation_budget(Some(now+Duration::from_millis(350)),now+Duration::from_millis(340)),Some(Duration::from_millis(10)));
  assert_eq!(observation_budget(Some(now),now),None);assert_eq!(observation_budget(None,now),None);
 }
 #[test]fn passive_task_expiring_behind_a_blocked_lookup_is_not_sent_to_the_peer(){
  let(worker,answers)=Worker::start(std::env::current_exe().unwrap(),arguments(),Duration::from_millis(250),Duration::from_millis(300));
  worker.submit(request(1,u32::MAX,TextOrigin::Focused));thread::sleep(Duration::from_millis(50));
  worker.observe_clipboard(request(0,1,TextOrigin::ObservedClipboard(7)).text,None,Instant::now()+Duration::from_millis(30));
  assert_eq!(answers.recv_timeout(Duration::from_secs(3)).unwrap().selection,Selection::Timeout);
  worker.submit(request(2,1,TextOrigin::Focused));
  assert_eq!(answers.recv_timeout(Duration::from_secs(3)).unwrap().serial,2);
  thread::sleep(Duration::from_millis(100));assert_eq!(worker.observed().count(),0,"an expired queued observation must not send a provider request or reply");
 }
 #[test]fn blocked_passive_provider_does_not_block_enqueue_and_next_lookup_recovers(){
  let(worker,answers)=Worker::start(std::env::current_exe().unwrap(),arguments(),Duration::from_millis(300),Duration::from_millis(300));
  let at=Instant::now();
  let token=worker.observe_clipboard(request(0,u32::MAX,TextOrigin::ObservedClipboard(7)).text,None,Instant::now()+Duration::from_millis(350));
  assert!(at.elapsed()<Duration::from_millis(100),"coordinator only queues; it never calls a clipboard provider");
  thread::sleep(Duration::from_millis(50));
  worker.submit(request(1,1,TextOrigin::Focused));
  let reply=answers.recv_timeout(Duration::from_secs(3)).unwrap();
  assert_eq!(reply.serial,1);assert_eq!(reply.selection,Selection::Word("runtime".into()));
  let observed:Vec<_>=worker.observed().collect();assert_eq!(observed.len(),1);
  assert_eq!(observed[0].serial,token);assert_eq!(observed[0].selection,Selection::Timeout);
 }
 #[test]fn obsolete_passive_read_is_cancelled_without_cancelling_the_new_hotkey(){
  let(worker,answers)=Worker::start(std::env::current_exe().unwrap(),arguments(),Duration::from_millis(300),Duration::from_millis(300));
  let token=worker.observe_clipboard(request(0,u32::MAX,TextOrigin::ObservedClipboard(7)).text,None,Instant::now()+Duration::from_millis(350));
  thread::sleep(Duration::from_millis(50));worker.cancel_observed(token);
  worker.submit(request(1,1,TextOrigin::Focused));
  assert_eq!(answers.recv_timeout(Duration::from_secs(3)).unwrap().selection,Selection::Word("runtime".into()));
  assert_eq!(worker.observed().count(),0);
  let token=worker.observe_clipboard(request(0,1,TextOrigin::ObservedClipboard(8)).text,None,Instant::now()+Duration::from_millis(350));
  let until=Instant::now()+Duration::from_secs(2);let reply=loop {
   if let Some(reply)=worker.observed().next(){break reply;}
   assert!(Instant::now()<until);thread::sleep(Duration::from_millis(5));
  };
  assert_eq!(reply.serial,token);assert_eq!(reply.clipboard_sequence,Some(9));
 }
 #[test]fn normal_worker_survives_timeout_without_native_fixture_or_desktop(){
  let(worker,answers)=Worker::start(std::env::current_exe().unwrap(),arguments(),Duration::from_millis(200),Duration::from_millis(300));
  worker.submit(request(1,u32::MAX,TextOrigin::Focused));
  assert_eq!(answers.recv_timeout(Duration::from_secs(3)).unwrap().selection,Selection::Timeout);
  worker.submit(request(2,1,TextOrigin::Focused));
  let reply=answers.recv_timeout(Duration::from_secs(3)).unwrap();
  assert_eq!(reply.serial,2);assert_eq!(reply.selection,Selection::Word("runtime".into()));
 }
}
