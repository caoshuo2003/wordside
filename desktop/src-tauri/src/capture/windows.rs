use super::*;
use super::policy::{permits_source,copy_chord};
use super::compatibility::{Capture,Input,Intent,Source,Snapshot,allows_fallback};
use super::input::{Observer,Kind,Event};
use super::windows_text::{TextOrigin,TextRequest,focus_matches};
use super::{selection::Selection,probe::{Worker,ProbeRequest},hotkey::Driver};
    use crate::{State,hide_popup};
    use std::{ffi::c_void,sync::{Arc,atomic::{Ordering,AtomicU64}},time::{Duration,Instant}};
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
        fn OpenClipboard(window:Handle)->i32;
        fn CloseClipboard()->i32;
        fn GetClipboardData(format:u32)->Handle;
        fn GetWindowRect(window:Handle,rect:*mut Rect)->i32;
        fn MonitorFromPoint(point:Point,flags:u32)->Handle;
        fn GetMonitorInfoW(monitor:Handle,info:*mut MonitorInfo)->i32;
        fn ShowWindow(window:Handle,command:i32)->i32;
        fn IsWindowVisible(window:Handle)->i32;
    }
    #[link(name="kernel32")]
    extern "system" {
        fn GlobalLock(handle:Handle)->*const c_void;
        fn GlobalUnlock(handle:Handle)->i32;
        fn GlobalSize(handle:Handle)->usize;

    }
    fn pid(window:Handle)->u32{let mut value=0;unsafe{GetWindowThreadProcessId(window,&mut value);}value}
    fn source_process(window:Handle)->Option<u32>{
        let id=pid(window);permits_source(id,std::process::id()).then_some(id)
    }
    fn clipboard()->Option<String>{
        unsafe{
            if OpenClipboard(0)==0{return None;}
            let handle=GetClipboardData(13);
            let result=if handle!=0{
                let size=GlobalSize(handle)/2;
                let ptr=GlobalLock(handle) as *const u16;
                if !ptr.is_null(){
                    let data=std::slice::from_raw_parts(ptr,size.min(4096));
                    let result=data.iter().position(|v|*v==0).and_then(|len|String::from_utf16(&data[..len]).ok());
                    GlobalUnlock(handle);result
                }else{None}
            }else{None};CloseClipboard();result
        }
    }
    fn inside(window:Handle,p:Point)->bool{let mut r=Rect::default();unsafe{GetWindowRect(window,&mut r)!=0&&p.x>=r.left&&p.x<r.right&&p.y>=r.top&&p.y<r.bottom}}
    fn show(app:&AppHandle,point:Point,serial:u64,manual:bool,expected_epoch:u64,source:Handle,source_pid:u32,focus:Handle,compatibility:Option<Snapshot>,input_guard:Option<(Arc<AtomicU64>,u64)>,payload:serde_json::Value){
        let handle=app.clone();let _=app.run_on_main_thread(move||{let app=&handle;
        let state=app.state::<State>();
        if state.capture_epoch.load(Ordering::SeqCst)!=expected_epoch||state.capture_serial.load(Ordering::SeqCst)!=serial||root_window(unsafe{GetForegroundWindow()})!=source||pid(source)!=source_pid{return;}
        if payload.get("word").is_some()&&!focus_matches(source,focus){return;}
        if input_guard.as_ref().is_some_and(|(generation,expected)|generation.load(Ordering::SeqCst)!=*expected){return;}
        if compatibility.as_ref().is_some_and(|value|!value.valid_at(unsafe{GetClipboardSequenceNumber()},Instant::now())){return;}
        if !state.ready.load(Ordering::SeqCst)||(!manual&&!state.enabled.load(Ordering::SeqCst)){return;}
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
        });
    }

#[derive(Clone)]
struct Awaiting { serial: u64, pid: u32, window: Handle, point: Point, epoch: u64, manual: bool, focus: Handle, origin: TextOrigin, pointer_retry: bool, failure: Option<Selection>, guard: Option<(Arc<AtomicU64>,u64)> }
impl Awaiting {
    fn automatic(serial: u64, epoch: u64, intent: Intent, guard: Option<(Arc<AtomicU64>,u64)>) -> Self {
        // Automatic and manual lookup must first ask the same focused control.
        Self { serial, pid: intent.source.pid, window: intent.source.window, point: Point { x: intent.x, y: intent.y }, epoch,
            manual: false, focus: intent.source.focus, origin: TextOrigin::Focused, pointer_retry: intent.pointer, failure: None, guard }
    }
}
struct Candidate { window: Handle, pid: u32, point: Point, word: String, epoch: u64, manual: bool, focus: Handle, after: Instant, compatibility: Option<Snapshot>, guard: Option<(Arc<AtomicU64>,u64)> }
fn root_window(window: Handle) -> Handle { let root = unsafe { GetAncestor(window, 2) }; if root != 0 { root } else { window } }
fn pressed(key: i32) -> bool { unsafe { GetAsyncKeyState(key) < 0 } }
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
fn request(worker: &Worker, value: &Awaiting) { worker.submit(probe_request(value)); }
fn sampled_press_invalidates(observed_events: bool, down: bool, was_down: bool) -> bool { !observed_events && down && !was_down }
fn reject_failure(failure: &Selection, capture: &mut Capture) {
    // A failed provider cannot authorize this attempt's clipboard fallback. It
    // must not erase an independent, still-valid gesture that a later direct
    // focused read may confirm. Protection/invalid text invalidate both paths.
    if matches!(failure, Selection::Protected | Selection::InvalidWord) { capture.clear(); }
}
fn operation_valid(value: &Awaiting, source: Source, epoch: u64, enabled: bool, _pinned: bool) -> bool {
    source.window == value.window && source.pid == value.pid && (value.focus == 0 || source.focus == value.focus)
        && epoch == value.epoch && (value.manual || enabled)
        && !value.guard.as_ref().is_some_and(|(generation, expected)| generation.load(Ordering::SeqCst) != *expected)
}
enum Resolution { Ready(Candidate), Wait(Awaiting), Retry(Awaiting), Failed(Awaiting, Selection) }
fn finish_probe(mut value: Awaiting, selection: Selection, capture: &mut Capture, observed_events: bool, now: Instant) -> Resolution {
    match selection {
        Selection::Word(word) => Resolution::Ready(Candidate { window: value.window, pid: value.pid, point: value.point, word, epoch: value.epoch,
            manual: value.manual, focus: value.focus, after: now + Duration::from_millis(if value.manual { 0 } else { 40 }), compatibility: None, guard: value.guard }),
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
        value.pointer_retry = false; value.origin = TextOrigin::Pointer(value.point.x, value.point.y); return Resolution::Retry(value);
    }
    Resolution::Failed(value, failure)
}
fn apply_resolution(resolution: Resolution, app: &AppHandle, worker: &Worker, awaiting: &mut Option<Awaiting>, candidate: &mut Option<Candidate>, source: &mut Handle) {
    match resolution {
        Resolution::Ready(value) => *candidate = Some(value),
        Resolution::Wait(value) => *awaiting = Some(value),
        Resolution::Retry(value) => { request(worker, &value); *awaiting = Some(value); }
        Resolution::Failed(value, failure) => {
            *candidate = None;
            if value.manual { if let Some((code, message)) = failure.failure() {
                *source = value.window;
                show(app, value.point, value.serial, true, value.epoch, value.window, value.pid, value.focus, None, value.guard, serde_json::json!({"code":code,"message":message}));
            } }
        }
    }
}
fn compatible(value: &Awaiting, snapshot: Snapshot, now: Instant) -> Candidate {
    Candidate { window: value.window, pid: value.pid, point: value.point, word: snapshot.word.clone(), epoch: value.epoch,
        manual: value.manual, focus: value.focus, after: now + Duration::from_millis(if value.manual { 0 } else { 40 }), compatibility: Some(snapshot), guard: value.guard.clone() }
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
    let (worker, answers) = Worker::new(); let mut capture = Capture::default();
    let mut capture_generation = 0;
    let mut candidate: Option<Candidate> = None; let mut awaiting: Option<Awaiting> = None;
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
        if let Some(observer) = observer.as_ref() { observer.set_shortcut(keys.input_signature()); }
        let (lost, mut inputs) = observer.as_ref().map(Observer::drain).unwrap_or_default();
        let down = pressed(1); let copy = copy_chord(pressed(0x11), pressed(0x43), pressed(0x2D), pressed(0x12), pressed(0x5B) || pressed(0x5C));
        let sequence = unsafe { GetClipboardSequenceNumber() }; let foreground = root_window(unsafe { GetForegroundWindow() });
        let context = current_source(foreground); let mut point = Point::default(); unsafe { GetCursorPos(&mut point); }
        let state = app.state::<State>(); let enabled = state.enabled.load(Ordering::SeqCst); let pinned = state.pinned.load(Ordering::SeqCst);
        let now = Instant::now(); let mut epoch = state.capture_epoch.load(Ordering::SeqCst);
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
                begin(&app, serial, false, epoch);
                let value = Awaiting::automatic(serial, epoch, intent, observer.as_ref().map(|observer| (observer.guard().0, capture_generation)));
                request(&worker, &value); awaiting = Some(value);
            }
        }
        if observer.as_ref().is_some_and(|value|value.generation()!=capture_generation) { capture.clear(); }
        capture.recent.observe(context, sequence, now);
        // A new selection's clipboard may arrive after its mouse-up or hotkey.
        // Only that preexisting bounded gesture grants permission to read it.
        if capture.recent.wants_clipboard(context, sequence, now) {
            let owner = unsafe { GetClipboardOwner() };
            let owner_matches = owner == 0 || pid(owner) == context.pid || root_window(owner) == context.window;
            if !owner_matches { capture.recent.clear(); }
            else if let Some(raw) = clipboard() {
                let read_at = Instant::now();
                if unsafe { GetClipboardSequenceNumber() } != sequence || root_window(unsafe { GetForegroundWindow() }) != foreground || current_source(foreground) != context {
                    capture.recent.clear();
                } else { capture.recent.accept(context, sequence, true, &raw, read_at); }
            }
        }
        for event in shortcuts {
            serial = serial.wrapping_add(1); invalidate(&worker, &state, serial); capture.manual(); awaiting = None; candidate = None;
            let window = root_window(event.window); if window != foreground { continue; }
            if let Some(id) = source_process(window) {
                begin(&app, serial, true, epoch);
                let value = Awaiting { serial, pid: id, window, point: Point { x: event.x, y: event.y }, epoch, manual: true, focus: event.focus, origin: TextOrigin::Focused, pointer_retry: false, failure: None, guard: observer.as_ref().map(Observer::guard) };
                request(&worker, &value); awaiting = Some(value);
            } else {
                let _ = app.emit_to("main", "native-capture-feedback", serde_json::json!({"code":"select_first","message":"请先在其他应用中选中文字，再按主动查词快捷键。"}));
            }
        }
        while let Ok(result) = answers.try_recv() {
            if result.serial != serial { continue; }
            if let Some(value) = awaiting.take() {
                if result.serial != value.serial || !operation_valid(&value, context, epoch, enabled, pinned) { continue; }
                let resolution = finish_probe(value, result.selection, &mut capture, observer.is_some(), now);
                apply_resolution(resolution, &app, &worker, &mut awaiting, &mut candidate, &mut source);
            }
        }
        if awaiting.as_ref().is_some_and(|v| !operation_valid(v, context, epoch, enabled, pinned) || !focus_matches(v.window, v.focus) || sampled_press_invalidates(observer.is_some(), down, was_down)) {
            serial = serial.wrapping_add(1); invalidate(&worker, &state, serial); awaiting = None; candidate = None; capture.clear();
        }
        if awaiting.as_ref().is_some_and(|v| v.failure.is_some()) {
            let resolution = resolve_deferred(awaiting.take().unwrap(), &capture, sequence, Instant::now());
            apply_resolution(resolution, &app, &worker, &mut awaiting, &mut candidate, &mut source);
        }
        if let Some(value) = candidate.as_ref() {
            let selection_source = Source { window: value.window, pid: value.pid, focus: value.focus };
            let expired = value.compatibility.as_ref().is_some_and(|v| !capture.recent.current(v.revision, selection_source, unsafe { GetClipboardSequenceNumber() }, Instant::now()));
            let input_changed = value.guard.as_ref().is_some_and(|(generation,expected)|generation.load(Ordering::SeqCst)!=*expected);
            if foreground != value.window || pid(foreground) != value.pid || state.capture_epoch.load(Ordering::SeqCst) != value.epoch
                || !focus_matches(value.window, value.focus) || (!value.manual && !enabled) || expired || input_changed { candidate = None; }
            else if !down && Instant::now() >= value.after && awaiting.is_none() {
                let value = candidate.take().unwrap(); source = value.window;
                show(&app, value.point, serial, value.manual, value.epoch, value.window, value.pid, value.focus, value.compatibility, value.guard, serde_json::json!({"word":value.word}));
            }
        }
        was_down = down; was_copy = copy; previous_sequence = sequence;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Source { Source { window: 20, pid: 200, focus: 21 } }
    fn operation() -> Awaiting { Awaiting::automatic(1, 1, Intent { source: source(), x: 40, y: 10, pointer: true }, None) }
    fn selected(capture: &mut Capture, now: Instant) {
        capture.recent.begin(source(), 7, now); assert!(capture.recent.accept(source(), 8, true, "terminal", now));
    }
    fn drag(capture: &mut Capture, now: Instant, automatic: bool) -> Option<Awaiting> {
        capture.input(Input::Down { window: 20, x: 10, y: 10, sequence: 7, shift: false, at: now }, source(), automatic, Duration::from_millis(500));
        capture.input(Input::Up { window: 20, x: 40, y: 10, at: now + Duration::from_millis(10) }, source(), automatic, Duration::from_millis(500))
            .lookup.map(|intent| Awaiting::automatic(1, 1, intent, None))
    }
    fn deferred(resolution: Resolution) -> Awaiting {
        match resolution { Resolution::Wait(value) => value, _ => panic!("the completed read should await only its authorized selection") }
    }
    fn ready(resolution: Resolution) -> Candidate {
        match resolution { Resolution::Ready(value) => value, _ => panic!("the authorized word should be ready exactly once") }
    }
    #[test] fn api6_regression_automatic_provider_failure_keeps_fresh_selection_for_manual() {
        let now = Instant::now(); let mut capture = Capture::default(); selected(&mut capture, now);
        assert!(matches!(finish_probe(operation(), Selection::SourceMismatch, &mut capture, true, now), Resolution::Failed(_, Selection::SourceMismatch)), "the failed automatic attempt must not display");
        let mut manual = operation(); manual.manual = true; manual.pointer_retry = false;
        // The physical source/focus and clipboard are unchanged. A later direct
        // focused read reports Unsupported, exactly as it does with automatic OFF.
        let value = deferred(finish_probe(manual, Selection::Unsupported, &mut capture, true, now));
        let result = ready(resolve_deferred(value, &capture, 8, now));
        assert!(result.manual); assert_eq!(result.word, "terminal");
    }
    #[test] fn api6_regression_mouse_up_event_does_not_cancel_selection_using_old_button_sample() {
        let now = Instant::now(); let mut capture = Capture::default(); let value = drag(&mut capture, now, true).unwrap();
        // The LL hook wakes capture before the asynchronous button-state update.
        if sampled_press_invalidates(true, true, false) { capture.clear(); }
        assert!(capture.recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(15)), "a recorded up must not be mistaken for a new down");
        let value = deferred(finish_probe(value, Selection::Unsupported, &mut capture, true, now));
        assert_eq!(ready(resolve_deferred(value, &capture, 8, now)).word, "terminal");
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
            assert!(!result.manual); assert_eq!(result.word, "terminal"); assert_eq!(result.focus, 21);
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
            assert_eq!(ready(resolve_deferred(value, &capture, 8, now + Duration::from_millis(30))).word, "terminal");
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
        assert_eq!(ready(resolve_deferred(value, &capture, 8, now)).word, "terminal", "a captured Pebrel word must not cause a second point request");
        let value = deferred(finish_probe(operation(), Selection::NoSelection, &mut capture, true, now));
        assert_eq!(ready(resolve_deferred(value, &capture, 8, now)).word, "terminal");
    }
    #[test] fn api6_manual_off_remains_independent_and_does_not_retry_pointer_or_sampled_clipboard() {
        let now = Instant::now(); let mut capture = Capture::default(); assert!(drag(&mut capture, now, false).is_none());
        assert!(capture.recent.accept(source(), 8, true, "runtime", now + Duration::from_millis(15)));
        let mut manual = operation(); manual.manual = true; manual.pointer_retry = false;
        assert!(operation_valid(&manual, source(), 1, false, false));
        let value = deferred(finish_probe(manual.clone(), Selection::Unsupported, &mut capture, true, now));
        let result = ready(resolve_deferred(value, &capture, 8, now)); assert!(result.manual); assert_eq!(result.word, "runtime");
        assert!(matches!(finish_probe(manual.clone(), Selection::Unsupported, &mut capture, false, now), Resolution::Failed(_, _)), "without actual input observation, OFF/manual never trusts sampled history");
        capture.clear(); let value = deferred(finish_probe(manual, Selection::Unsupported, &mut capture, true, now));
        assert!(matches!(resolve_deferred(value, &capture, 8, now), Resolution::Failed(_, _)), "manual lookup has no point fallback");
    }
    #[test] fn api6_direct_words_win_and_late_copy_does_not_generate_a_second_result() {
        let now = Instant::now(); let mut capture = Capture::default(); let value = drag(&mut capture, now, true).unwrap();
        let result = ready(finish_probe(value, Selection::Word("runtime".into()), &mut capture, true, now));
        assert_eq!(result.word, "runtime"); assert!(result.compatibility.is_none());
        assert!(capture.recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(15)));
        // The copy remains a passive record; it no longer creates a second
        // automatic candidate independently of the completed probe request.
        assert_eq!(result.word, "runtime");
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
