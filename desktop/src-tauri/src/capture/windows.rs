use super::*;
use super::policy::{permits_source,copy_chord};
use super::compatibility::{Capture,Input,Intent,Source,Snapshot,allows_fallback};
use super::input::{Observer,Kind,Event};
use super::timing::{Activity,Released};
use super::windows_text::{TextOrigin,TextRequest,focus_matches};
use super::{selection::Selection,probe::{Worker,ProbeRequest},hotkey::Driver};
    use crate::{State,hide_popup};
    use std::{sync::{Arc,atomic::{Ordering,AtomicU64},mpsc::{channel,Receiver,TryRecvError}},time::{Duration,Instant}};
    use tauri::{Emitter,Manager,PhysicalPosition};
    type Handle=isize;
    #[derive(Clone,Copy,Default)] #[repr(C)] struct Point{x:i32,y:i32}
    #[derive(Default)] #[repr(C)] struct Rect{left:i32,top:i32,right:i32,bottom:i32}
    #[repr(C)] struct MonitorInfo{size:u32,monitor:Rect,work:Rect,flags:u32}
    #[derive(Default)] #[repr(C)] struct GuiInfo{size:u32,flags:u32,active:Handle,focus:Handle,capture:Handle,menu:Handle,moving:Handle,caret:Handle,rect:Rect}
    #[link(name="user32")]
    extern "system" {
        fn GetAsyncKeyState(key:i32)->i16;
        fn GetDoubleClickTime()->u32;
        fn GetCursorPos(point:*mut Point)->i32;
        fn GetForegroundWindow()->Handle;
        fn GetAncestor(window:Handle,flags:u32)->Handle;
        fn GetWindowThreadProcessId(window:Handle,pid:*mut u32)->u32;
        fn GetClipboardSequenceNumber()->u32;
        fn GetGUIThreadInfo(thread:u32,info:*mut GuiInfo)->i32;
        fn GetClipboardOwner()->Handle;
        fn GetWindowRect(window:Handle,rect:*mut Rect)->i32;
        fn MonitorFromPoint(point:Point,flags:u32)->Handle;
        fn GetMonitorInfoW(monitor:Handle,info:*mut MonitorInfo)->i32;
        fn ShowWindow(window:Handle,command:i32)->i32;
        fn IsWindowVisible(window:Handle)->i32;
    }
    fn pid(window:Handle)->u32{let mut value=0;unsafe{GetWindowThreadProcessId(window,&mut value);}value}
    fn source_process(window:Handle)->Option<u32>{
        let id=pid(window);permits_source(id,std::process::id()).then_some(id)
    }
    fn inside(window:Handle,p:Point)->bool{let mut r=Rect::default();unsafe{GetWindowRect(window,&mut r)!=0&&p.x>=r.left&&p.x<r.right&&p.y>=r.top&&p.y<r.bottom}}
    fn show(app:&AppHandle,point:Point,serial:u64,manual:bool,expected_epoch:u64,source:Handle,source_pid:u32,focus:Handle,compatibility:Option<Snapshot>,input_guard:Option<(Arc<AtomicU64>,u64)>,release:Option<(Activity,u64)>,payload:serde_json::Value)->Receiver<bool>{
        let(tx,reply)=channel(); let handle=app.clone();let _=app.run_on_main_thread(move||{
        let consumed=(||{let app=&handle;
        let state=app.state::<State>();
        if state.capture_epoch.load(Ordering::SeqCst)!=expected_epoch||state.capture_serial.load(Ordering::SeqCst)!=serial||root_window(unsafe{GetForegroundWindow()})!=source||pid(source)!=source_pid{return true;}
        if payload.get("word").is_some()&&!focus_matches(source,focus){return true;}
        if input_guard.as_ref().is_some_and(|(generation,expected)|generation.load(Ordering::SeqCst)!=*expected){return true;}
        if compatibility.as_ref().is_some_and(|value|!value.valid_at(unsafe{GetClipboardSequenceNumber()},Instant::now())){return true;}
        if !state.ready.load(Ordering::SeqCst)||(!manual&&!state.enabled.load(Ordering::SeqCst)){return true;}
        if !publication_ready(release.as_ref()) || !sampled_released() { return false; }
        if let Some(window)=app.get_webview_window("popup"){
            if !state.pinned.load(Ordering::SeqCst) {
                let scale=window.monitor_from_point(point.x as f64,point.y as f64).ok().flatten().map(|m|m.scale_factor()).unwrap_or_else(||window.scale_factor().unwrap_or(1.0));
                let mut info=MonitorInfo{size:std::mem::size_of::<MonitorInfo>() as u32,monitor:Rect::default(),work:Rect::default(),flags:0};
                let ok=unsafe{GetMonitorInfoW(MonitorFromPoint(point,2),&mut info)};
                if ok!=0{
                    let preferred=state.settings.lock().unwrap().popup_size;
                    let (width,height)=preferred.fit(scale,info.work.right-info.work.left,info.work.bottom-info.work.top);
                    let (x,y)=place(point.x,point.y,width as i32,height as i32,(info.work.left,info.work.top,info.work.right,info.work.bottom));
                    let desired=crate::popup_geometry::PopupSize::from_physical(width,height,scale).unwrap_or(preferred);
                    // Moving can itself trigger DPI/resize events. Register the final
                    // size first, then always apply it in the target monitor's scale.
                    state.popup_geometry.lock().unwrap().programmatic(desired);
                    let _=window.set_position(PhysicalPosition::new(x,y));
                    let _=window.set_size(tauri::LogicalSize::new(width as f64/scale,height as f64/scale));
                }
            }
            let event=if payload.get("word").is_some(){"native-selection"}else{"native-selection-failed"};
            let mut payload=payload;payload["serial"]=serde_json::json!(serial);payload["manual"]=serde_json::json!(manual);
            let _=app.emit_to("popup",event,payload);
            // Win32 explicitly shows the card without activating it or stealing the terminal's focus.
            if let Ok(hwnd)=window.hwnd(){unsafe{ShowWindow(hwnd.0 as isize,4);}}
        }
        true})(); let _=tx.send(consumed);
        }); reply
    }

#[derive(Clone)]
struct Awaiting { serial: u64, pid: u32, window: Handle, point: Point, epoch: u64, manual: bool, focus: Handle, origin: TextOrigin, pointer_retry: bool, failure: Option<Selection>, submitted: bool, queued_at: Instant, release: Option<Activity>, guard: Option<(Arc<AtomicU64>,u64)> }
impl Awaiting {
    fn automatic(serial: u64, epoch: u64, intent: Intent, guard: Option<(Arc<AtomicU64>,u64)>, release: Option<Activity>) -> Self {
        // Automatic and manual lookup must first ask the same focused control.
        Self { serial, pid: intent.source.pid, window: intent.source.window, point: Point { x: intent.x, y: intent.y }, epoch,
            manual: false, focus: intent.source.focus, origin: TextOrigin::Focused, pointer_retry: intent.pointer, failure: None, submitted: false, queued_at: Instant::now(), release, guard }
    }
}
struct Candidate { window: Handle, pid: u32, point: Point, selection: Selection, publication: Option<Receiver<bool>>, release: Option<Activity>, epoch: u64, manual: bool, focus: Handle, after: Instant, compatibility: Option<Snapshot>, guard: Option<(Arc<AtomicU64>,u64)> }
fn publication_consumed(value:&mut Candidate)->bool {
    match value.publication.as_ref().map(|reply|reply.try_recv()) {
        Some(Ok(true)|Err(TryRecvError::Disconnected))=>true,
        Some(Ok(false))=>{value.publication=None; false},
        _=>false,
    }
}
fn root_window(window: Handle) -> Handle { let root = unsafe { GetAncestor(window, 2) }; if root != 0 { root } else { window } }
fn pressed(key: i32) -> bool { unsafe { GetAsyncKeyState(key) < 0 } }
fn sampled_released()->bool { !(1..256).any(pressed) }
fn release_stamp(observer:Option<&Observer>)->Option<u64> {
    observer.map(|observer|observer.activity().stamp()).unwrap_or_else(||sampled_released().then_some(0))
}
fn publication_ready(release:Option<&(Activity,u64)>)->bool {
    release.map(|(activity,stamp)|activity.stamp()==Some(*stamp)).unwrap_or_else(sampled_released)
}
fn current_source(window: Handle) -> Source {
    let pid = source_process(window).unwrap_or(0);
    let thread = unsafe { GetWindowThreadProcessId(window, std::ptr::null_mut()) };
    let mut info = GuiInfo { size: std::mem::size_of::<GuiInfo>() as u32, ..Default::default() };
    let focus = if thread != 0 && unsafe { GetGUIThreadInfo(thread, &mut info) } != 0 { info.focus } else { 0 };
    Source { window, pid, focus }
}
fn begin(app: &AppHandle, serial: u64, manual: bool, expected_epoch: u64) {
    let state = app.state::<State>(); state.popup_epoch.fetch_add(1, Ordering::SeqCst);
    let handle = app.clone(); let _ = app.run_on_main_thread(move || {
        if handle.state::<State>().capture_epoch.load(Ordering::SeqCst) != expected_epoch || handle.state::<State>().capture_serial.load(Ordering::SeqCst) != serial { return; }
        if !handle.state::<State>().pinned.load(Ordering::SeqCst) {if let Some(window) = handle.get_webview_window("popup") { super::hide(&window); }}
        let _ = handle.emit_to("popup", "native-capture-started", serde_json::json!({"serial":serial,"manual":manual}));
    });
}
fn invalidate(worker: &Worker, state: &State, serial: u64) { state.capture_serial.store(serial, Ordering::SeqCst); worker.cancel(serial); }
fn probe_request(value: &Awaiting) -> ProbeRequest {
    ProbeRequest { serial: value.serial, text: TextRequest { pid: value.pid, window: value.window, origin: value.origin, focus: value.focus } }
}
fn request(worker: &Worker, value: &mut Awaiting) { worker.submit_verified(probe_request(value), value.guard.clone(),value.release.clone()); value.submitted=true; }
fn same_manual(value:&Awaiting, source:Source, epoch:u64)->bool { value.manual && operation_valid(value,source,epoch,false,false) }
fn candidate(value:Awaiting, selection:Selection, now:Instant, compatibility:Option<Snapshot>)->Candidate {
    Candidate {window:value.window,pid:value.pid,point:value.point,selection,publication:None,release:value.release,epoch:value.epoch,manual:value.manual,focus:value.focus,after:now,compatibility,guard:value.guard}
}
fn sampled_press_invalidates(observed_events: bool, down: bool, was_down: bool) -> bool { !observed_events && down && !was_down }
fn reject_failure(failure: &Selection, capture: &mut Capture) {
    // A failed provider cannot authorize this attempt's clipboard fallback. It
    // must not erase an independent, still-valid gesture that a later direct
    // focused read may confirm. Protection/invalid text invalidate both paths.
    if matches!(failure, Selection::Protected | Selection::InvalidWord) { capture.clear(); }
}
fn operation_source_valid(value: &Awaiting, source: Source, epoch: u64, enabled: bool) -> bool {
    source.window==value.window && source.pid==value.pid && epoch==value.epoch && (value.manual||enabled)
        && !value.guard.as_ref().is_some_and(|(generation,expected)|generation.load(Ordering::SeqCst)!=*expected)
}
fn operation_valid(value: &Awaiting, source: Source, epoch: u64, enabled: bool, _pinned: bool) -> bool {
    operation_source_valid(value,source,epoch,enabled)
        && (value.manual && !value.submitted || value.focus==0 || source.focus==value.focus)
}
fn focus_failure(mut value: Awaiting, source: Source, epoch: u64, enabled: bool, serial: u64, now: Instant) -> Option<Candidate> {
    // A same-window programmatic focus change must reject the word, but need
    // not swallow a manual request. Only a real, unchanged input guard grants
    // this source-bound failure card; never use fallback or resubmit the copy.
    if !value.manual || !value.submitted || value.guard.is_none() || value.focus==source.focus
        || !operation_source_valid(&value,source,epoch,enabled) {return None;}
    value.serial=serial;value.focus=0;
    Some(candidate(value,Selection::SourceMismatch,now,None))
}
fn selection_requires_focus(selection: &Selection) -> bool {matches!(selection,Selection::Word(_))}
enum Preparation { Ready, Wait, Failed }
fn prepare_focus(value: &mut Awaiting, source: Source, now: Instant) -> Preparation {
    if !value.manual || value.submitted { return Preparation::Ready; }
    // WM_HOTKEY arrives before its modifiers are released. Menu/child focus can
    // settle during that release. Bind it once now, not at the keydown snapshot.
    if source.focus != 0 { value.focus=source.focus; return Preparation::Ready; }
    if now.saturating_duration_since(value.queued_at) < Duration::from_millis(750) { return Preparation::Wait; }
    // An unavailable focus must not become a wildcard authorizing a copy.
    // A source-bound failure card can still explain the failed request.
    value.focus=0; Preparation::Failed
}
enum Resolution { Ready(Candidate), Wait(Awaiting), Retry(Awaiting), Failed(Awaiting, Selection) }
fn finish_probe(mut value: Awaiting, selection: Selection, capture: &mut Capture, observed_events: bool, now: Instant) -> Resolution {
    match selection {
        Selection::Word(word) => Resolution::Ready(candidate(value,Selection::Word(word),now,None)),
        failure => {
            reject_failure(&failure, capture);
            if allows_fallback(&failure) && (!value.manual || observed_events) {
                value.failure = Some(failure); Resolution::Wait(value)
            } else { Resolution::Failed(value, failure) }
        }
    }
}
fn resolve_deferred(mut value: Awaiting, capture: &Capture, sequence: u32, now: Instant) -> Resolution {
    let failure = value.failure.take().expect("only a completed, unsupported read is deferred");
    let source = Source { window: value.window, pid: value.pid, focus: value.focus };
    if let Some(snapshot) = capture.recent.fallback(&failure, source, sequence, now) { return Resolution::Ready(compatible(&value, snapshot, now)); }
    if capture.recent.pending() { value.failure = Some(failure); return Resolution::Wait(value); }
    // Some browser documents expose their selected range at the mouse position
    // rather than the focused control. Try it once, only after the original copy
    // window ends without a usable word. Never replace a valid Pebrel selection.
    if !value.manual && value.pointer_retry && matches!(value.origin, TextOrigin::Focused) {
        value.pointer_retry = false; value.origin = TextOrigin::Pointer(value.point.x, value.point.y); value.submitted=false; return Resolution::Retry(value);
    }
    Resolution::Failed(value, failure)
}
fn apply_resolution(resolution: Resolution, awaiting: &mut Option<Awaiting>, pending: &mut Option<Candidate>) {
    match resolution {
        Resolution::Ready(value) => *pending=Some(value),
        Resolution::Wait(value)|Resolution::Retry(value) => *awaiting=Some(value),
        Resolution::Failed(value,failure) => {
            *pending=if value.manual {Some(candidate(value,failure,Instant::now(),None))} else {None};
        }
    }
}
fn compatible(value:&Awaiting, snapshot:Snapshot, now:Instant)->Candidate {
    candidate(value.clone(),Selection::Word(snapshot.word.clone()),now,Some(snapshot))
}

pub fn run(app: AppHandle) {
    let state = app.state::<State>(); let initial = super::shortcut::parse(&state.settings.lock().unwrap().lookup_hotkey);
    let invalid = initial.as_ref().err().cloned(); let mut keys = Driver::new(state.hotkeys.clone(), initial.unwrap_or(None));
    if let Some(error) = invalid { state.hotkeys.set_error(error); }
    let _ = app.emit("native-state", crate::info(&state)); drop(state);
    let observer = match Observer::new(keys.input_signature()) {
        Ok(value) => Some(value),
        Err(message) => { let _ = app.emit_to("main", "native-capture-feedback", serde_json::json!({"code":"capture_unavailable","message":message})); None }
    };
    let (worker, answers) = Worker::new(app.state::<State>().config.with_file_name("capture-status.json")); let mut capture = Capture::default();
    let mut capture_generation = 0; let mut released=Released::default();
    let mut candidate: Option<Candidate> = None; let mut awaiting: Option<Awaiting> = None;
    let mut observation: Option<(u64,Source,u64)> = None; let mut observed_attempt: Option<(Source,u32,u64)> = None;
    let mut serial = 0u64; let mut was_down = false; let mut was_copy = false;
    let mut previous_sequence = unsafe { GetClipboardSequenceNumber() }; let mut source = 0;
    let mut dismissal = crate::dismissal::Dismissal::default();
    let mut observed_epoch = app.state::<State>().capture_epoch.load(Ordering::SeqCst);
    loop {
        let state = app.state::<State>(); let popup = app.get_webview_window("popup");
        let popup_hwnd = popup.as_ref().and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize).unwrap_or(0);
        let enabled = state.enabled.load(Ordering::SeqCst); let visible = unsafe { IsWindowVisible(popup_hwnd) != 0 };
        Driver::wait(poll_interval(enabled, visible || awaiting.is_some() || candidate.is_some() || capture.recent.pending()));
        let shortcuts = keys.drain();
        if let Some(observer) = observer.as_ref() { observer.set_shortcut(keys.input_signature()); observer.refresh(); }
        let (lost, mut inputs) = observer.as_ref().map(Observer::drain).unwrap_or_default();
        let down = pressed(1); let copy = copy_chord(pressed(0x11), pressed(0x43), pressed(0x2D), pressed(0x12), pressed(0x5B) || pressed(0x5C));
        let foreground = root_window(unsafe { GetForegroundWindow() });
        let context = current_source(foreground); let mut point = Point::default(); unsafe { GetCursorPos(&mut point); }
        let state = app.state::<State>(); let enabled = state.enabled.load(Ordering::SeqCst); let pinned = state.pinned.load(Ordering::SeqCst);
        let now = Instant::now(); let input_stamp=release_stamp(observer.as_ref()); let idle=released.ready(input_stamp,now); let mut epoch = state.capture_epoch.load(Ordering::SeqCst);
        if epoch != observed_epoch {
            serial = serial.wrapping_add(1); invalidate(&worker, &state, serial); capture.clear(); awaiting = None; candidate = None; observed_epoch = epoch;
        }
        if lost {
            // Partial input histories cannot authorize a clipboard read.
            inputs.clear(); capture.clear(); serial = serial.wrapping_add(1); invalidate(&worker, &state, serial); awaiting = None; candidate = None;
        }
        let outside_press = inputs.iter().any(|event| event.kind == Kind::Down && !inside(popup_hwnd, Point { x: event.point.x, y: event.point.y }));
        let focus = if foreground == 0 || source == 0 { crate::dismissal::Focus::Unknown }
            else if foreground == source { crate::dismissal::Focus::Source }
            else if crate::dismissal::belongs_to_window(foreground, popup_hwnd) { crate::dismissal::Focus::Popup }
            else { crate::dismissal::Focus::Other };
        if dismissal.update(now, crate::dismissal::Snapshot { visible, pinned, press_started: outside_press || sampled_press_invalidates(observer.is_some(), down, was_down),
            mouse_down: down, inside_popup: !outside_press && inside(popup_hwnd, point), focus }) {
            hide_popup(app.clone()); source = 0; epoch = state.capture_epoch.load(Ordering::SeqCst); observed_epoch = epoch;
            serial = serial.wrapping_add(1); invalidate(&worker, &state, serial); capture.clear(); awaiting = None; candidate = None;
        }
        // Preserve the old automatic path if system hook registration failed. A
        // sampled history is never trusted for OFF/manual clipboard compatibility.
        if observer.is_none() && enabled && !awaiting.as_ref().is_some_and(|v| v.manual) {
            let event = |kind| Event { kind, window: foreground, point: super::input::Point { x: point.x, y: point.y }, sequence: previous_sequence, shift: pressed(0x10), at: now, generation: 0 };
            if down && !was_down { inputs.push(event(Kind::Down)); }
            if !down && was_down { inputs.push(event(Kind::Up)); }
            if copy && !was_copy { inputs.push(event(Kind::Key { code: 0x43, modifiers: 2 })); }
        }
        for event in inputs {
            let input = match event.kind {
                Kind::Down => Input::Down { window: event.window, x: event.point.x, y: event.point.y, sequence: event.sequence, shift: event.shift, at: event.at },
                Kind::Up => Input::Up { window: event.window, x: event.point.x, y: event.point.y, at: event.at },
                Kind::Other => Input::Changed,
                Kind::Key { code, modifiers } => {
                    if keys.matches_input(code, modifiers) { continue; }
                    if event.window == foreground && copy_chord(modifiers & 2 != 0, code == 0x43, code == 0x2D, modifiers & 1 != 0, modifiers & 8 != 0) {
                        Input::Copy { x: event.point.x, y: event.point.y, sequence: event.sequence, at: event.at }
                    } else { Input::Changed }
                }
            };
            capture_generation = event.generation;
            let automatic = enabled && shortcuts.is_empty()
                && !awaiting.as_ref().is_some_and(|v| v.manual) && !candidate.as_ref().is_some_and(|v| v.manual);
            let effect = capture.input(input, context, automatic, Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() })));
            if effect.invalidate {
                serial = serial.wrapping_add(1); state.capture_serial.store(serial, Ordering::SeqCst);
                // Passive observation must not wake an idle selection worker on every keystroke.
                if awaiting.is_some() { worker.cancel(serial); }
                awaiting = None; candidate = None;
            }
            if let Some(intent) = effect.lookup {
                let value = Awaiting::automatic(serial, epoch, intent, observer.as_ref().map(|observer| (observer.guard().0, capture_generation)),observer.as_ref().map(Observer::activity));
                awaiting = Some(value);
            }
        }
        if observer.as_ref().is_some_and(|value|value.generation()!=capture_generation) { capture.clear(); }
        // The coordinator must never call GetClipboardData: delayed rendering
        // can block it even though the input hooks have their own thread.
        for reply in worker.observed() {
            if !observation.as_ref().is_some_and(|(token,_,_)|*token==reply.serial) {continue;}
            let(_,observed_source,generation)=observation.take().unwrap();
            let unchanged=observer.as_ref().is_none_or(|observer|observer.generation()==generation);
            if observed_source==context && unchanged {
                if let (Selection::Word(word),Some(sequence))=(reply.selection,reply.clipboard_sequence) {
                    if unsafe{GetClipboardSequenceNumber()}==sequence {
                        capture.recent.accept(context,sequence,true,&word,Instant::now());
                    }
                }
            }
        }
        let sequence=unsafe{GetClipboardSequenceNumber()}; let observed_at=Instant::now();
        capture.recent.observe(context,sequence,observed_at);
        let generation=observer.as_ref().map(Observer::generation).unwrap_or(0);
        let wants=capture.recent.wants_clipboard(context,sequence,observed_at);
        if observation.as_ref().is_some_and(|(_,source,expected)|!wants || *source!=context || *expected!=generation) {
            worker.cancel_observed(observation.take().unwrap().0);
        }
        if wants && observation.is_none() && observed_attempt!=Some((context,sequence,generation)) {
            let owner=unsafe{GetClipboardOwner()};
            if owner!=0 && pid(owner)!=context.pid && root_window(owner)!=context.window {capture.recent.clear();}
            else {
                let text=TextRequest{pid:context.pid,window:context.window,focus:context.focus,origin:TextOrigin::ObservedClipboard(sequence)};
                let deadline=capture.recent.clipboard_deadline(context,sequence,observed_at).expect("fresh gesture has its original deadline");
                let token=worker.observe_clipboard(text,observer.as_ref().map(Observer::guard),deadline);
                observation=Some((token,context,generation)); observed_attempt=Some((context,sequence,generation));
            }
        }
        for event in shortcuts {
            if awaiting.as_ref().is_some_and(|value|same_manual(value,context,epoch))
                || candidate.as_ref().is_some_and(|value|value.manual && value.window==context.window && value.pid==context.pid && value.focus==context.focus
                    && value.epoch==epoch && !value.guard.as_ref().is_some_and(|(generation,expected)|generation.load(Ordering::SeqCst)!=*expected)) { continue; }
            serial = serial.wrapping_add(1); invalidate(&worker, &state, serial); capture.manual(); awaiting = None; candidate = None;
            let window = root_window(event.window); if window != foreground { continue; }
            if let Some(id) = source_process(window) {
                let value = Awaiting { serial, pid: id, window, point: Point { x: event.x, y: event.y }, epoch, manual: true, focus: event.focus, origin: TextOrigin::Focused, pointer_retry: false, failure: None, submitted:false, queued_at: now, release:observer.as_ref().map(Observer::activity), guard: observer.as_ref().map(Observer::guard) };
                awaiting = Some(value);
            } else {
                let _ = app.emit_to("main", "native-capture-feedback", serde_json::json!({"code":"select_first","message":"请先在其他应用中选中文字，再按主动查词快捷键。"}));
            }
        }
        while let Ok(result) = answers.try_recv() {
            if result.serial != serial { continue; }
            if let Some(value) = awaiting.take() {
                if result.serial != value.serial {continue;}
                let latest=current_source(root_window(unsafe{GetForegroundWindow()}));
                if !operation_valid(&value,latest,epoch,enabled,pinned) {
                    serial=serial.wrapping_add(1);invalidate(&worker,&state,serial);
                    candidate=focus_failure(value,latest,epoch,enabled,serial,now);
                    if let Some(value)=candidate.as_ref(){begin(&app,serial,true,value.epoch);}
                    continue;
                }
                let resolution = finish_probe(value, result.selection, &mut capture, observer.is_some(), now);
                apply_resolution(resolution, &mut awaiting, &mut candidate);
            }
        }
        if awaiting.is_some() {
            // Decide cancellation and its feedback from one fresh snapshot. A
            // child focus may already have returned since this loop's context.
            let latest=current_source(root_window(unsafe{GetForegroundWindow()}));
            if awaiting.as_ref().is_some_and(|v| !operation_valid(v,latest,epoch,enabled,pinned) || sampled_press_invalidates(observer.is_some(),down,was_down)) {
                let value=awaiting.take().unwrap();
                serial=serial.wrapping_add(1);invalidate(&worker,&state,serial);capture.clear();
                candidate=focus_failure(value,latest,epoch,enabled,serial,now);
                if let Some(value)=candidate.as_ref(){begin(&app,serial,true,value.epoch);}
            }
        }
        if !idle && awaiting.as_ref().is_some_and(|value|!value.manual && value.submitted && value.failure.is_none()) {
            serial=serial.wrapping_add(1); invalidate(&worker,&state,serial);
            if let Some(value)=awaiting.as_mut(){value.serial=serial;value.submitted=false;}
        }
        if idle && release_stamp(observer.as_ref())==input_stamp { if let Some(value)=awaiting.as_mut() { if !value.submitted && value.failure.is_none() {
            match prepare_focus(value,context,Instant::now()) {
                Preparation::Ready=>{begin(&app,value.serial,value.manual,value.epoch); request(&worker,value);},
                Preparation::Wait=>{},
                Preparation::Failed=>{begin(&app,value.serial,value.manual,value.epoch); value.failure=Some(Selection::SourceMismatch);},
            }
        } } }
        if awaiting.as_ref().is_some_and(|v| v.failure.is_some()) {
            let resolution = resolve_deferred(awaiting.take().unwrap(), &capture, unsafe{GetClipboardSequenceNumber()}, Instant::now());
            apply_resolution(resolution, &mut awaiting, &mut candidate);
        }
        if let Some(value) = candidate.as_mut() {
            let selection_source = Source { window: value.window, pid: value.pid, focus: value.focus };
            let expired = value.compatibility.as_ref().is_some_and(|v| !capture.recent.current(v.revision, selection_source, unsafe { GetClipboardSequenceNumber() }, Instant::now()));
            let input_changed = value.guard.as_ref().is_some_and(|(generation,expected)|generation.load(Ordering::SeqCst)!=*expected);
            if foreground != value.window || pid(foreground) != value.pid || state.capture_epoch.load(Ordering::SeqCst) != value.epoch
                || (selection_requires_focus(&value.selection) && !focus_matches(value.window, value.focus)) || (!value.manual && !enabled) || expired || input_changed { candidate = None; }
            else if publication_consumed(value) { candidate=None; }
            else {
                if idle && release_stamp(observer.as_ref())==input_stamp && Instant::now()>=value.after && awaiting.is_none() && value.publication.is_none() {
                    source=value.window;
                    let payload=match &value.selection { Selection::Word(word)=>serde_json::json!({"word":word}), failure=>{
                        let(code,message)=failure.failure().expect("failed capture has a user message"); serde_json::json!({"code":code,"message":message})
                    }};
                    let release=value.release.as_ref().and_then(|activity|input_stamp.map(|stamp|(activity.clone(),stamp)));
                    value.publication=Some(show(&app,value.point,serial,value.manual,value.epoch,value.window,value.pid,value.focus,value.compatibility.clone(),value.guard.clone(),release,payload));
                }
            }
        }
        was_down = down; was_copy = copy; previous_sequence = sequence;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Source { Source { window: 20, pid: 200, focus: 21 } }
    fn operation() -> Awaiting { Awaiting::automatic(1, 1, Intent { source: source(), x: 40, y: 10, pointer: true }, None, None) }
    fn selected(capture: &mut Capture, now: Instant) {
        capture.recent.begin(source(), 7, now); assert!(capture.recent.accept(source(), 8, true, "terminal", now));
    }
    fn drag(capture: &mut Capture, now: Instant, automatic: bool) -> Option<Awaiting> {
        capture.input(Input::Down { window: 20, x: 10, y: 10, sequence: 7, shift: false, at: now }, source(), automatic, Duration::from_millis(500));
        capture.input(Input::Up { window: 20, x: 40, y: 10, at: now + Duration::from_millis(10) }, source(), automatic, Duration::from_millis(500))
            .lookup.map(|intent| Awaiting::automatic(1, 1, intent, None, None))
    }
    fn deferred(resolution: Resolution) -> Awaiting {
        match resolution { Resolution::Wait(value) => value, _ => panic!("the completed read should await only its authorized selection") }
    }
    fn ready(resolution: Resolution) -> Candidate {
        match resolution { Resolution::Ready(value) => value, _ => panic!("the authorized word should be ready exactly once") }
    }
    #[test] fn submitted_manual_focus_change_rejects_the_word_but_keeps_one_failure_card() {
        let mut value=operation();value.manual=true;value.submitted=true;
        value.guard=Some((Arc::new(AtomicU64::new(10)),10));
        let result=focus_failure(value,Source{focus:22,..source()},1,false,2,Instant::now()).unwrap();
        assert_eq!(result.selection,Selection::SourceMismatch);assert_eq!(result.focus,0);
        assert!(result.compatibility.is_none());assert!(result.manual);
        assert!(!selection_requires_focus(&result.selection),"a failure may explain the rejected query in the same source window");
        assert!(selection_requires_focus(&Selection::Word("never_display_this_old_word".into())));
    }
    #[test] fn manual_focus_return_before_cancellation_preserves_the_submitted_request() {
        let mut value=operation();value.manual=true;value.submitted=true;
        value.guard=Some((Arc::new(AtomicU64::new(10)),10));
        let earlier=Source{focus:22,..source()};
        assert!(!operation_valid(&value,earlier,1,false,false));
        let latest=source();
        assert!(operation_valid(&value,latest,1,false,false),"use the fresh source before canceling, not the earlier loop snapshot");
        assert!(focus_failure(value,latest,1,false,2,Instant::now()).is_none());
    }
    #[test] fn focus_failure_never_survives_new_input_other_source_or_absent_observer() {
        let mut value=operation();value.manual=true;value.submitted=true;
        let generation=Arc::new(AtomicU64::new(10));value.guard=Some((generation.clone(),10));
        let changed=Source{focus:22,..source()};let now=Instant::now();
        for context in [Source{window:30,..changed},Source{pid:300,..changed},source()] {
            assert!(focus_failure(value.clone(),context,1,false,2,now).is_none());
        }
        assert!(focus_failure(value.clone(),changed,2,false,2,now).is_none());
        generation.store(11,Ordering::SeqCst);assert!(focus_failure(value.clone(),changed,1,false,2,now).is_none());
        value.guard=None;assert!(focus_failure(value.clone(),changed,1,false,2,now).is_none());
        value.manual=false;assert!(focus_failure(value,changed,1,true,2,now).is_none());
    }
    #[test] fn queued_manual_survives_same_window_focus_transition_until_release() {
        let mut value=operation(); value.manual=true; value.submitted=false;
        assert!(operation_valid(&value,Source{focus:22,..source()},1,false,false),
            "a hotkey queued while Alt is held must not be silently lost before release");
        assert!(operation_valid(&value,Source{focus:0,..source()},1,false,false),
            "a transient unavailable focus must wait instead of consuming the one press");
        value.submitted=true;
        assert!(!operation_valid(&value,Source{focus:22,..source()},1,false,false),
            "once submitted, a changed control must still invalidate the result");
    }
    #[test] fn manual_focus_is_bound_once_at_submission_and_unknown_focus_has_a_deadline() {
        let mut value=operation(); value.manual=true; let at=value.queued_at;
        assert!(matches!(prepare_focus(&mut value,Source{focus:0,..source()},at),Preparation::Wait));
        assert!(matches!(prepare_focus(&mut value,Source{focus:22,..source()},at+Duration::from_millis(80)),Preparation::Ready));
        assert_eq!(probe_request(&value).text.focus,22);
        value.submitted=true;
        assert!(!operation_valid(&value,source(),1,false,false));
        assert!(operation_valid(&value,Source{focus:22,..source()},1,false,false));
        for changed in [Source{window:30,..source()},Source{pid:300,..source()}] {
            value.submitted=false;
            assert!(!operation_valid(&value,changed,1,false,false));
        }
        value.submitted=false;
        assert!(matches!(prepare_focus(&mut value,Source{focus:0,..source()},at+Duration::from_millis(751)),Preparation::Failed));
        assert_eq!(value.focus,0);
    }
    #[test] fn one_hotkey_queues_until_all_keys_are_released_and_does_not_restart_on_a_second_press() {
        let activity=Activity::default(); let mut manual=operation(); manual.manual=true; manual.release=Some(activity.clone());
        let mut gate=Released::default(); let now=Instant::now();
        activity.changed(true); assert!(!gate.ready(activity.stamp(),now));
        assert!(!manual.submitted); assert!(same_manual(&manual,source(),1));
        activity.changed(false); assert!(!gate.ready(activity.stamp(),now+Duration::from_millis(100)));
        assert!(gate.ready(activity.stamp(),now+Duration::from_millis(140)));
        let mut capture=Capture::default(); let candidate=ready(finish_probe(manual.clone(),Selection::Word("runtime".into()),&mut capture,true,now));
        assert!(candidate.manual); assert_eq!(candidate.selection,Selection::Word("runtime".into()));
        manual.submitted=true; assert!(!same_manual(&manual,Source{focus:22,..source()},1)); assert!(!same_manual(&manual,source(),2));
    }
    #[test] fn failure_and_success_use_the_same_release_checked_publication_path() {
        for selection in [Selection::Word("runtime".into()),Selection::ClipboardUnavailable,Selection::SourceMismatch] {
            let activity=Activity::default(); let mut manual=operation(); manual.manual=true; manual.release=Some(activity.clone());
            let mut capture=Capture::default(); let now=Instant::now(); let mut awaiting=None; let mut pending=None;
            apply_resolution(finish_probe(manual,selection.clone(),&mut capture,true,now),&mut awaiting,&mut pending);
            let candidate=pending.unwrap(); assert_eq!(candidate.selection,selection);
            let stamp=activity.stamp().unwrap(); assert!(publication_ready(Some(&(activity.clone(),stamp))));
            activity.changed(true); assert!(!publication_ready(Some(&(activity.clone(),stamp))));
            activity.changed(false); assert!(!publication_ready(Some(&(activity.clone(),stamp))),"a release after main-thread queuing must defer, not publish with an old fence");
        }
    }
    #[test] fn main_thread_deferral_keeps_the_candidate_for_a_later_publication() {
        let mut value=candidate(operation(),Selection::Word("runtime".into()),Instant::now(),None);
        let(tx,reply)=channel(); value.publication=Some(reply);
        assert!(!publication_consumed(&mut value)); tx.send(false).unwrap();
        assert!(!publication_consumed(&mut value)); assert!(value.publication.is_none());
        let(tx,reply)=channel(); value.publication=Some(reply); tx.send(true).unwrap();
        assert!(publication_consumed(&mut value));
    }
    #[test] fn api6_regression_automatic_provider_failure_keeps_fresh_selection_for_manual() {
        let now = Instant::now(); let mut capture = Capture::default(); selected(&mut capture, now);
        assert!(matches!(finish_probe(operation(), Selection::SourceMismatch, &mut capture, true, now), Resolution::Failed(_, Selection::SourceMismatch)), "the failed automatic attempt must not display");
        let mut manual = operation(); manual.manual = true; manual.pointer_retry = false;
        // The physical source/focus and clipboard are unchanged. A later direct
        // focused read reports Unsupported, exactly as it does with automatic OFF.
        let value = deferred(finish_probe(manual, Selection::Unsupported, &mut capture, true, now));
        let result = ready(resolve_deferred(value, &capture, 8, now));
        assert!(result.manual); assert_eq!(result.selection, Selection::Word("terminal".into()));
    }
    #[test] fn api6_regression_mouse_up_event_does_not_cancel_selection_using_old_button_sample() {
        let now = Instant::now(); let mut capture = Capture::default(); let value = drag(&mut capture, now, true).unwrap();
        // The LL hook wakes capture before the asynchronous button-state update.
        if sampled_press_invalidates(true, true, false) { capture.clear(); }
        assert!(capture.recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(15)), "a recorded up must not be mistaken for a new down");
        let value = deferred(finish_probe(value, Selection::Unsupported, &mut capture, true, now));
        assert_eq!(ready(resolve_deferred(value, &capture, 8, now)).selection, Selection::Word("terminal".into()));
        assert!(!sampled_press_invalidates(true, true, false));
        assert!(sampled_press_invalidates(false, true, false), "sampled mode still cancels on a new press");
    }
    #[test] fn api6_recorded_mouse_up_cannot_hide_existing_card_using_a_stale_button_sample() {
        let now = Instant::now(); let mut dismissal = crate::dismissal::Dismissal::default();
        assert!(!dismissal.update(now, crate::dismissal::Snapshot { visible: true, pinned: false,
            press_started: sampled_press_invalidates(true, true, false), mouse_down: true, inside_popup: false, focus: crate::dismissal::Focus::Source }), "an observed up must not trigger hide_popup/capture.clear");
        assert!(dismissal.update(now, crate::dismissal::Snapshot { visible: true, pinned: false,
            press_started: true, mouse_down: true, inside_popup: false, focus: crate::dismissal::Focus::Source }), "an actual outside down still dismisses the old card");
    }
    #[test] fn api6_automatic_and_manual_probe_the_same_real_focus_before_pointer_retry() {
        let automatic = probe_request(&operation()); let mut value = operation(); value.manual = true; value.pointer_retry = false;
        let manual = probe_request(&value);
        assert!(matches!(automatic.text.origin, TextOrigin::Focused)); assert!(matches!(manual.text.origin, TextOrigin::Focused));
        assert_eq!((automatic.text.window, automatic.text.pid, automatic.text.focus), (manual.text.window, manual.text.pid, manual.text.focus));
        assert_eq!(automatic.text.focus, 21, "automatic lookup must retain the native focus");
    }
    #[test] fn api6_automatic_unsupported_reads_before_or_after_late_copy_yield_one_guarded_word() {
        for failure in [Selection::NoSelection, Selection::Unsupported] { for copied in [false, true] {
            let now = Instant::now(); let mut capture = Capture::default(); let value = drag(&mut capture, now, true).unwrap();
            if copied { assert!(capture.recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(15))); }
            let value = deferred(finish_probe(value, failure.clone(), &mut capture, true, now + Duration::from_millis(20)));
            let value = if !copied {
                let value = deferred(resolve_deferred(value, &capture, 7, now + Duration::from_millis(20)));
                assert!(capture.recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(25))); value
            } else { value };
            let result = ready(resolve_deferred(value, &capture, 8, now + Duration::from_millis(30)));
            assert!(!result.manual); assert_eq!(result.selection, Selection::Word("terminal".into())); assert_eq!(result.focus, 21);
            assert_eq!(result.compatibility.unwrap().sequence, 8);
        } }
    }
    #[test] fn api6_transient_provider_errors_abort_only_current_attempt_before_or_after_copy() {
        for failure in [Selection::SourceMismatch, Selection::AccessDenied, Selection::Timeout, Selection::ReadError("provider".into())] { for copied in [false, true] {
            let now = Instant::now(); let mut capture = Capture::default(); let value = drag(&mut capture, now, true).unwrap();
            if copied { assert!(capture.recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(15))); }
            assert!(matches!(finish_probe(value, failure.clone(), &mut capture, true, now), Resolution::Failed(_, _)), "{failure:?}, copied={copied}");
            if !copied { assert!(capture.recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(25))); }
            capture.manual(); let mut manual = operation(); manual.manual = true; manual.pointer_retry = false;
            let value = deferred(finish_probe(manual, Selection::Unsupported, &mut capture, true, now));
            assert_eq!(ready(resolve_deferred(value, &capture, 8, now + Duration::from_millis(30))).selection, Selection::Word("terminal".into()));
        } }
    }
    #[test] fn api6_protected_or_invalid_selection_clears_both_capture_paths_before_or_after_copy() {
        for failure in [Selection::Protected, Selection::InvalidWord] { for copied in [false, true] {
            let now = Instant::now(); let mut capture = Capture::default(); let value = drag(&mut capture, now, true).unwrap();
            if copied { assert!(capture.recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(15))); }
            assert!(matches!(finish_probe(value, failure.clone(), &mut capture, true, now), Resolution::Failed(_, _)));
            assert!(!capture.recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(25)));
            assert!(capture.recent.fallback(&Selection::Unsupported, source(), 8, now).is_none());
        } }
    }
    #[test] fn api6_browser_pointer_retry_is_once_only_and_never_overrides_valid_terminal_selection() {
        let now = Instant::now(); let mut capture = Capture::default(); let value = drag(&mut capture, now, true).unwrap();
        let value = deferred(finish_probe(value, Selection::Unsupported, &mut capture, true, now));
        let value = deferred(resolve_deferred(value, &capture, 7, now + Duration::from_millis(200)));
        let end = now + Duration::from_millis(361); capture.recent.observe(source(), 7, end);
        let value = match resolve_deferred(value, &capture, 7, end) { Resolution::Retry(value) => value, _ => panic!("mouse selection should retry point after the bounded copy window") };
        assert!(matches!(probe_request(&value).text.origin, TextOrigin::Pointer(40, 10))); assert!(!value.pointer_retry);
        let value = deferred(finish_probe(value, Selection::Unsupported, &mut capture, true, end));
        assert!(matches!(resolve_deferred(value, &capture, 7, end), Resolution::Failed(_, Selection::Unsupported)), "do not resubmit in a loop");
        selected(&mut capture, now); let value = deferred(finish_probe(operation(), Selection::Unsupported, &mut capture, true, now));
        assert_eq!(ready(resolve_deferred(value, &capture, 8, now)).selection, Selection::Word("terminal".into()), "a captured Pebrel word must not cause a second point request");
        let value = deferred(finish_probe(operation(), Selection::NoSelection, &mut capture, true, now));
        assert_eq!(ready(resolve_deferred(value, &capture, 8, now)).selection, Selection::Word("terminal".into()));
    }
    #[test] fn api6_manual_off_remains_independent_and_does_not_retry_pointer_or_sampled_clipboard() {
        let now = Instant::now(); let mut capture = Capture::default(); assert!(drag(&mut capture, now, false).is_none());
        assert!(capture.recent.accept(source(), 8, true, "runtime", now + Duration::from_millis(15)));
        let mut manual = operation(); manual.manual = true; manual.pointer_retry = false;
        assert!(operation_valid(&manual, source(), 1, false, false));
        let value = deferred(finish_probe(manual.clone(), Selection::Unsupported, &mut capture, true, now));
        let result = ready(resolve_deferred(value, &capture, 8, now)); assert!(result.manual); assert_eq!(result.selection, Selection::Word("runtime".into()));
        assert!(matches!(finish_probe(manual.clone(), Selection::Unsupported, &mut capture, false, now), Resolution::Failed(_, _)), "without actual input observation, OFF/manual never trusts sampled history");
        capture.clear(); let value = deferred(finish_probe(manual, Selection::Unsupported, &mut capture, true, now));
        assert!(matches!(resolve_deferred(value, &capture, 8, now), Resolution::Failed(_, _)), "manual lookup has no point fallback");
    }
    #[test] fn api6_direct_words_win_and_late_copy_does_not_generate_a_second_result() {
        let now = Instant::now(); let mut capture = Capture::default(); let value = drag(&mut capture, now, true).unwrap();
        let result = ready(finish_probe(value, Selection::Word("runtime".into()), &mut capture, true, now));
        assert_eq!(result.selection, Selection::Word("runtime".into())); assert!(result.compatibility.is_none());
        assert!(capture.recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(15)));
        // The copy remains a passive record; it no longer creates a second
        // automatic candidate independently of the completed probe request.
        assert_eq!(result.selection, Selection::Word("runtime".into()));
    }
    #[test] fn api6_source_focus_epoch_mode_or_new_input_change_invalidates_in_flight_operation() {
        let value = operation(); assert!(operation_valid(&value, source(), 1, true, false));
        for changed in [Source { window: 30, ..source() }, Source { pid: 300, ..source() }, Source { focus: 22, ..source() }] {
            assert!(!operation_valid(&value, changed, 1, true, false));
        }
        assert!(!operation_valid(&value, source(), 2, true, false));
        assert!(!operation_valid(&value, source(), 1, false, false)); assert!(operation_valid(&value, source(), 1, true, true), "pin must not pause automatic lookup");
        let mut manual=value.clone();manual.manual=true;assert!(operation_valid(&manual,source(),1,false,true));
        let generation = Arc::new(AtomicU64::new(10)); let mut value = value; value.guard = Some((generation.clone(), 10));
        assert!(operation_valid(&value, source(), 1, true, false)); generation.fetch_add(1, Ordering::SeqCst);
        assert!(!operation_valid(&value, source(), 1, true, false));
    }
    #[test] fn api6_preserved_provider_failure_record_cannot_survive_physical_source_or_sequence_change() {
        for changed in [Source { window: 30, ..source() }, Source { pid: 300, ..source() }, Source { focus: 22, ..source() }] {
            let now = Instant::now(); let mut capture = Capture::default(); selected(&mut capture, now);
            assert!(matches!(finish_probe(operation(), Selection::SourceMismatch, &mut capture, true, now), Resolution::Failed(_, _)));
            capture.recent.observe(changed, 8, now);
            assert!(capture.recent.fallback(&Selection::Unsupported, source(), 8, now).is_none());
        }
        let now = Instant::now(); let mut capture = Capture::default(); selected(&mut capture, now);
        assert!(matches!(finish_probe(operation(), Selection::SourceMismatch, &mut capture, true, now), Resolution::Failed(_, _)));
        capture.recent.observe(source(), 9, now);
        assert!(capture.recent.fallback(&Selection::Unsupported, source(), 8, now).is_none());
    }
}
