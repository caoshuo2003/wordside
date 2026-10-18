//! Bounded input observation. Ordinary input is never intercepted. A registered
//! Alt shortcut uses a tagged, non-character event to prevent menu activation.
//! Hooks use their own message thread so clipboard/provider work cannot block them.
use super::timing::Activity;
use std::{cell::RefCell, sync::{Arc, atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering}, mpsc::{sync_channel, Receiver, SyncSender}}, thread::JoinHandle, time::{Duration, Instant}};
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Point { pub x: i32, pub y: i32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Down, Up, Other, Key { code: u32, modifiers: u32 } }
#[derive(Clone, Copy)]
pub struct Event { pub kind: Kind, pub window: isize, pub point: Point, pub sequence: u32, pub shift: bool, pub at: Instant, pub generation: u64, pub shortcut: Option<(u64,super::compatibility::Source)> }
#[repr(C)]
struct MousePacket { point: Point, data: u32, flags: u32, time: u32, extra: usize }
#[repr(C)]
struct KeyPacket { code: u32, scan: u32, flags: u32, time: u32, extra: usize }
#[derive(Default)]
#[repr(C)]
struct Message { window: isize, message: u32, wparam: usize, lparam: isize, time: u32, x: i32, y: i32, private: u32 }
#[derive(Default)] #[repr(C)] struct FocusRect {left:i32,top:i32,right:i32,bottom:i32}
#[derive(Default)] #[repr(C)] struct GuiInfo {size:u32,flags:u32,active:isize,focus:isize,capture:isize,menu:isize,moving:isize,caret:isize,rect:FocusRect}
#[link(name="user32")]
extern "system" {
    fn SetWindowsHookExW(kind: i32, callback: unsafe extern "system" fn(i32, usize, isize) -> isize, module: isize, thread: u32) -> isize;
    fn UnhookWindowsHookEx(hook: isize) -> i32;
    fn CallNextHookEx(hook: isize, code: i32, wparam: usize, lparam: isize) -> isize;
    fn GetMessageW(message: *mut Message, window: isize, min: u32, max: u32) -> i32;
    fn PeekMessageW(message: *mut Message, window: isize, min: u32, max: u32, remove: u32) -> i32;
    fn PostThreadMessageW(thread: u32, message: u32, wparam: usize, lparam: isize) -> i32;
    fn GetForegroundWindow() -> isize;
    fn GetWindowThreadProcessId(window:isize,pid:*mut u32)->u32;
    fn GetGUIThreadInfo(thread:u32,info:*mut GuiInfo)->i32;
    fn WindowFromPoint(point: Point) -> isize;
    fn GetAncestor(window: isize, flags: u32) -> isize;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn GetClipboardSequenceNumber() -> u32;
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetLastInputInfo(info: *mut LastInput) -> i32;
}
#[link(name="kernel32")]
extern "system" { fn GetCurrentThreadId() -> u32; fn GetModuleHandleW(name: *const u16) -> isize; fn GetTickCount() -> u32; }
#[repr(C)] struct LastInput { size: u32, time: u32 }
const WAKE: u32 = 0x8000 + 0x43;
const REFRESH: u32 = 0x8000 + 0x44;
const CAPACITY: usize = 64;
#[derive(Default)]
struct Modifiers([bool; 8]);
impl Modifiers {
    fn index(key: u32) -> Option<usize> { Some(match key { 0xA0 | 0x10 => 0, 0xA1 => 1, 0xA2 | 0x11 => 2, 0xA3 => 3, 0xA4 | 0x12 => 4, 0xA5 => 5, 0x5B => 6, 0x5C => 7, _ => return None }) }
    fn update(&mut self, key: u32, down: bool) -> bool {
        if let Some(index) = Self::index(key) { self.0[index] = down; true } else { false }
    }
    fn mask(&self) -> u32 {
        u32::from(self.0[0] || self.0[1]) * 4 | u32::from(self.0[2] || self.0[3]) * 2
            | u32::from(self.0[4] || self.0[5]) | u32::from(self.0[6] || self.0[7]) * 8
    }
}
#[derive(Clone)]
struct HeldKeys { keys: [bool; 256], count: u32 }
impl Default for HeldKeys { fn default() -> Self { Self { keys: [false;256], count: 0 } } }
impl HeldKeys {
    fn update(&mut self, code: u32, down: bool) -> bool {
        let code=match code { 0x10=>0xA0, 0x11=>0xA2, 0x12=>0xA4, key=>key };
        let Some(old)=self.keys.get_mut(code as usize) else { return false; };
        if *old==down { return false; }
        *old=down; if down { self.count+=1; } else { self.count-=1; } true
    }
}
struct Context { shortcut_press: Option<u32>, repair_since: Option<Instant>, held: HeldKeys, activity: Activity, sender: SyncSender<Event>, capture_thread: u32, lost: Arc<AtomicBool>, modifiers: Modifiers, generation: Arc<AtomicU64>, shortcut: Arc<AtomicU64> }
thread_local! { static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) }; }
// Windows silently removes a low-level hook whose callback once exceeds
// LowLevelHooksTimeout (paging after a long idle, a busy boot) and never tells
// the owner. Session input newer than every callback means the hooks are gone.
thread_local! { static HEARD: RefCell<Option<Arc<AtomicU32>>> = const { RefCell::new(None) }; }
const STALL_MS: i32 = 1000;
fn hooks_stalled(heard: u32, input: u32) -> bool { input.wrapping_sub(heard) as i32 > STALL_MS }
fn last_input() -> Option<u32> {
    let mut info = LastInput { size: std::mem::size_of::<LastInput>() as u32, time: 0 };
    (unsafe { GetLastInputInfo(&mut info) } != 0).then_some(info.time)
}
fn heard() {
    HEARD.with(|cell| { if let Ok(heard) = cell.try_borrow() { if let Some(heard) = heard.as_ref() { heard.store(unsafe { GetTickCount() }, Ordering::Relaxed); } } });
}
// Only the hook's physical history can distinguish a real Ctrl press from
// our tagged menu mask. Unknown history never authorizes releasing a key.
pub(super) fn physical_controls_released() -> Option<(bool,bool)> {
    CONTEXT.with(|cell| cell.try_borrow().ok().and_then(|context|context.as_ref()
        .map(|context|(!context.held.keys[0xA2],!context.held.keys[0xA3]))))
}
fn root(window: isize) -> isize { let root = unsafe { GetAncestor(window, 2) }; if root == 0 { window } else { root } }
fn binding_matches(binding:u64,code:u32,modifiers:u32)->bool {
    binding != 0 && (binding & 0xffff) == (u64::from(code) | (u64::from(modifiers) << 8))
}
fn advance_generation(context:&Context,shortcut:bool)->u64 {
    if shortcut {context.generation.load(Ordering::SeqCst)}
    else {context.generation.fetch_add(1,Ordering::SeqCst).wrapping_add(1)}
}
#[cfg(test)] fn key_generation(context:&Context,code:u32,modifiers:u32)->u64 {
    advance_generation(context,binding_matches(context.shortcut.load(Ordering::SeqCst),code,modifiers))
}
fn needs_alt_mask(modifiers:u32)->bool {modifiers & 1 != 0 && modifiers & 2 == 0}
unsafe fn shortcut_source(window:isize)->super::compatibility::Source {
    let mut pid=0;let thread=GetWindowThreadProcessId(window,&mut pid);
    let mut gui=GuiInfo{size:std::mem::size_of::<GuiInfo>() as u32,..Default::default()};
    let focus=if thread!=0&&GetGUIThreadInfo(thread,&mut gui)!=0 {gui.focus}else{0};
    super::compatibility::Source{window,pid,focus}
}
fn track_edge(context: &mut Context, code: u32, down: bool) -> bool {
    let changed=context.held.update(code,down);
    if changed { context.repair_since=None; context.activity.changed(context.held.count!=0); }
    changed
}
fn key_transition(context:&mut Context,code:u32,down:bool,alt_flag:bool)->Option<(u32,u64,Option<u64>)> {
    let changed=track_edge(context,code,down);
    let modifier=context.modifiers.update(code,down);
    if !down && context.shortcut_press==Some(code) {context.shortcut_press=None;}
    if modifier || !down {return None;}
    // Retain the physical hotkey press until its keyup, even if Alt/Ctrl is
    // released first. Ordinary editing repeats still invalidate old selections.
    if !changed && context.shortcut_press==Some(code) {return None;}
    let modifiers=context.modifiers.mask()|u32::from(alt_flag);
    let binding=context.shortcut.load(Ordering::SeqCst);
    let shortcut=binding_matches(binding,code,modifiers);
    if shortcut {context.shortcut_press=Some(code);}
    Some((modifiers,advance_generation(context,shortcut),shortcut.then_some(binding)))
}
// A missed keyup (or a key already held during hook startup) must not leave
// every later hotkey waiting forever. Only repair to UP, on the hook thread,
// after a stable physical all-up sample. Never synthesize or release input.
fn repair_released(context: &mut Context, all_up: bool, now: Instant) -> bool {
    if context.held.count==0 || !all_up { context.repair_since=None; return false; }
    let since=context.repair_since.get_or_insert(now);
    if now.saturating_duration_since(*since)<Duration::from_millis(80) { return false; }
    context.held=HeldKeys::default(); context.modifiers=Modifiers::default();
    context.shortcut_press=None; context.repair_since=None; context.activity.changed(false); true
}
fn send(context: &Context, event: Event) {
    if context.sender.try_send(event).is_err() { context.lost.store(true, Ordering::SeqCst); }
    unsafe { PostThreadMessageW(context.capture_thread, WAKE, 0, 0); }
}
unsafe extern "system" fn mouse_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == 0 && lparam != 0 {
        heard();
        let packet = &*(lparam as *const MousePacket);
        let message=wparam as u32;
        let kind=match message { 0x201=>Some(Kind::Down), 0x202=>Some(Kind::Up), 0x204|0x207|0x20B|0x20A|0x20E=>Some(Kind::Other), _=>None };
        let button=match message { 0x201=>Some((1,true)), 0x202=>Some((1,false)), 0x204=>Some((2,true)), 0x205=>Some((2,false)), 0x207=>Some((4,true)), 0x208=>Some((4,false)), 0x20B|0x20C=>Some((if packet.data>>16==1 {5}else{6},message==0x20B)), _=>None };
        if kind.is_some() || button.is_some() {
            CONTEXT.with(|cell| { if let Ok(mut context)=cell.try_borrow_mut() { if let Some(context)=context.as_mut() {
                if let Some((key,down))=button { if track_edge(context,key,down) { PostThreadMessageW(context.capture_thread,WAKE,0,0); } }
                if let Some(kind)=kind {
                    let window=if kind==Kind::Down {WindowFromPoint(packet.point)} else {GetForegroundWindow()};
                    let generation=context.generation.fetch_add(1,Ordering::SeqCst).wrapping_add(1);
                    send(context,Event{kind,window:root(window),point:packet.point,sequence:GetClipboardSequenceNumber(),shift:context.modifiers.mask()&4!=0,at:Instant::now(),generation,shortcut:None});
                }
            } } });
        }
    }
    CallNextHookEx(0, code, wparam, lparam)
}
unsafe extern "system" fn key_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == 0 && lparam != 0 {
        heard();
        let down = matches!(wparam as u32, 0x100 | 0x104);
        let up = matches!(wparam as u32, 0x101 | 0x105);
        if down || up {
            let packet = &*(lparam as *const KeyPacket);
            // Do not recursively trigger a lookup or change physical modifier state
            // for the copy transaction. Other injected input is still observed.
            if super::copy_policy::own_input(packet.flags, packet.extra) {
                return CallNextHookEx(0, code, wparam, lparam);
            }
            let mut menu_mask=None;
            CONTEXT.with(|cell| { if let Ok(mut context) = cell.try_borrow_mut() { if let Some(context) = context.as_mut() {
                let observed=key_transition(context,packet.code,down,packet.flags&0x20!=0);
                PostThreadMessageW(context.capture_thread,WAKE,0,0);
                if let Some((modifiers,generation,binding))=observed {
                    let mut point=Point::default(); GetCursorPos(&mut point);
                    let window=root(GetForegroundWindow());
                    let shortcut=binding.map(|binding|(binding,shortcut_source(window)));
                    if needs_alt_mask(modifiers) {menu_mask=shortcut.map(|(_,source)|source).filter(|source|super::policy::permits_source(source.pid,std::process::id()));}
                    send(context,Event{kind:Kind::Key{code:packet.code,modifiers},window,point,sequence:GetClipboardSequenceNumber(),shift:modifiers&4!=0,at:Instant::now(),generation,shortcut});
                }
            } } });
            // Do this on the input edge, while Alt is still held. Waiting for
            // the lookup worker is too late: the source menu may already own
            // keyboard focus. Our tagged pair is excluded above and cannot
            // invalidate the original selection or recurse into this branch.
            if let Some(source)=menu_mask {
                let _=super::windows_copy::mask_alt_menu(&super::windows_text::TextRequest{pid:source.pid,window:source.window,focus:source.focus,origin:super::windows_text::TextOrigin::Focused});
            }
        }
    }
    CallNextHookEx(0, code, wparam, lparam)
}
struct Hook(isize);
impl Drop for Hook { fn drop(&mut self) { unsafe { UnhookWindowsHookEx(self.0); } } }
pub struct Observer { events: Receiver<Event>, lost: Arc<AtomicBool>, stop: Arc<AtomicBool>, thread: u32, join: Option<JoinHandle<()>>, generation: Arc<AtomicU64>, shortcut: Arc<AtomicU64>, activity: Activity, heard: Arc<AtomicU32> }
impl Observer {
    pub fn new(initial_shortcut: u64) -> Result<Self, String> {
        let capture_thread = unsafe { GetCurrentThreadId() };
        let (sender, events) = sync_channel(CAPACITY); let (ready, result) = sync_channel(1);
        let lost = Arc::new(AtomicBool::new(false)); let stop = Arc::new(AtomicBool::new(false)); let thread_id = Arc::new(AtomicU32::new(0));
        let generation = Arc::new(AtomicU64::new(0)); let shortcut = Arc::new(AtomicU64::new(initial_shortcut));
        let activity=Activity::default(); let hook_activity=activity.clone();
        let hook_generation = generation.clone(); let hook_shortcut = shortcut.clone();
        let hook_lost = lost.clone(); let hook_stop = stop.clone(); let hook_id = thread_id.clone();
        let heard = Arc::new(AtomicU32::new(unsafe { GetTickCount() })); let hook_heard = heard.clone();
        let join = std::thread::spawn(move || unsafe {
            let mut message = Message::default(); PeekMessageW(&mut message, 0, 0, 0, 0);
            let thread = GetCurrentThreadId(); hook_id.store(thread, Ordering::SeqCst);
            HEARD.with(|cell| *cell.borrow_mut() = Some(hook_heard.clone()));
            CONTEXT.with(|cell| *cell.borrow_mut() = Some(Context { shortcut_press: None, repair_since: None, held: HeldKeys::default(), activity: hook_activity, sender, capture_thread, lost: hook_lost, modifiers: Modifiers::default(), generation: hook_generation, shortcut: hook_shortcut }));
            let module = GetModuleHandleW(std::ptr::null());
            let mouse = SetWindowsHookExW(14, mouse_hook, module, 0);
            let mouse = (mouse != 0).then_some(Hook(mouse));
            let keyboard = if mouse.is_some() { SetWindowsHookExW(13, key_hook, module, 0) } else { 0 };
            let keyboard = (keyboard != 0).then_some(Hook(keyboard));
            if mouse.is_none() || keyboard.is_none() {
                let _ = ready.send(Err("兼容取词监听未启动，请重启 WordSide 后重新选词。直接选区取词仍可使用。".to_string()));
            } else {
                // Seed only after BOTH hooks are live. A release between a seed
                // and installation would otherwise leave the gate held forever.
                CONTEXT.with(|cell| { if let Some(context)=cell.borrow_mut().as_mut() {
                    let mut held=HeldKeys::default(); let mut modifiers=Modifiers::default();
                    for key in 1..256 { if !matches!(key,0x10|0x11|0x12) {
                        let down=GetAsyncKeyState(key as i32)<0; held.update(key,down); modifiers.update(key,down);
                    } }
                    context.held=held; context.modifiers=modifiers; context.activity.changed(context.held.count!=0);
                } });
                hook_heard.store(GetTickCount(), Ordering::SeqCst);
                let _ = ready.send(Ok(thread));
                while !hook_stop.load(Ordering::SeqCst) && GetMessageW(&mut message, 0, 0, 0) > 0 {
                    if message.message==REFRESH {
                        super::windows_copy::recover_menu_mask();
                        CONTEXT.with(|cell| { if let Some(context)=cell.borrow_mut().as_mut() {
                            if context.held.count!=0 {
                                let all_up=(1..256).all(|key|GetAsyncKeyState(key)>=0);
                                if repair_released(context,all_up,Instant::now()) {PostThreadMessageW(context.capture_thread,WAKE,0,0);}
                            }
                        } });
                    }
                }
            }
            // Make one last bounded attempt to release our own incomplete menu
            // pair before removing the observer that normally retries it.
            super::windows_copy::recover_menu_mask();
            drop(keyboard); drop(mouse); CONTEXT.with(|cell| *cell.borrow_mut() = None);
        });
        match result.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(thread)) => Ok(Self { events, lost, stop, thread, join: Some(join), generation, shortcut, activity, heard }),
            response => {
                stop.store(true, Ordering::SeqCst);
                unsafe { PostThreadMessageW(thread_id.load(Ordering::SeqCst), 0x12, 0, 0); }
                // Do not block capture startup if system hook registration itself is stuck.
                drop(join);
                Err(response.ok().and_then(Result::err).unwrap_or_else(|| "兼容取词监听启动超时，请重启 WordSide。".into()))
            }
        }
    }
    pub fn refresh(&self) { unsafe { PostThreadMessageW(self.thread,REFRESH,0,0); } }
    /// The hooks missed newer session input, or their thread has exited.
    pub fn stalled(&self) -> bool {
        self.join.as_ref().is_some_and(JoinHandle::is_finished)
            || last_input().is_some_and(|input| hooks_stalled(self.heard.load(Ordering::SeqCst), input))
    }
    pub fn activity(&self) -> Activity { self.activity.clone() }
    pub fn generation(&self) -> u64 { self.generation.load(Ordering::SeqCst) }
    pub fn guard(&self) -> (Arc<AtomicU64>, u64) { (self.generation.clone(), self.generation()) }
    pub fn set_shortcut(&self, value: u64) { self.shortcut.store(value, Ordering::SeqCst); }
    pub fn drain(&self) -> (bool, Vec<Event>) {
        let events = self.events.try_iter().collect();
        (self.lost.swap(false, Ordering::SeqCst), events)
    }
}
impl Drop for Observer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        unsafe { PostThreadMessageW(self.thread, 0x12, 0, 0); }
        if let Some(join) = self.join.take() { let _ = join.join(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn hooks_are_stalled_only_when_session_input_is_newer_than_every_callback() {
        assert!(!hooks_stalled(5_000,5_000));
        assert!(!hooks_stalled(5_000,4_000),"no input since the last callback proves nothing");
        assert!(!hooks_stalled(5_000,5_900),"ordinary callback latency");
        assert!(hooks_stalled(5_000,6_500));
        assert!(hooks_stalled(u32::MAX-200,1_000),"the tick count wraps after 49.7 days");
        assert!(!hooks_stalled(1_000,u32::MAX-200));
    }
    #[test] fn menu_mask_physical_control_history_is_side_specific_and_unknown_is_not_up() {
        assert_eq!(physical_controls_released(),None);
        let (sender,_)=sync_channel(1);
        let context=Context{shortcut_press:None,repair_since:None,held:HeldKeys::default(),activity:Activity::default(),
            sender,capture_thread:0,lost:Arc::new(AtomicBool::new(false)),modifiers:Modifiers::default(),
            generation:Arc::new(AtomicU64::new(0)),shortcut:Arc::new(AtomicU64::new(0))};
        CONTEXT.with(|cell|*cell.borrow_mut()=Some(context));
        assert_eq!(physical_controls_released(),Some((true,true)));
        for (code,down,expected) in [(0xA3,true,(true,false)),(0x11,true,(false,false)),
            (0xA3,false,(false,true)),(0xA2,false,(true,true))] {
            CONTEXT.with(|cell|{track_edge(cell.borrow_mut().as_mut().unwrap(),code,down);});
            assert_eq!(physical_controls_released(),Some(expected));
        }
        CONTEXT.with(|cell|*cell.borrow_mut()=None);
        assert_eq!(physical_controls_released(),None);
    }
    #[test] fn only_our_registered_alt_chord_needs_menu_masking_and_tagged_input_is_ignored() {
        assert!(needs_alt_mask(1));assert!(needs_alt_mask(1|4));
        for modifiers in [0,2,4,8,1|2,1|2|4] {assert!(!needs_alt_mask(modifiers));}
        assert!(super::super::copy_policy::own_input(0x10,super::super::copy_policy::INPUT_MARKER));
        assert!(!super::super::copy_policy::own_input(0x10,0));
    }
    #[test] fn both_mouse_edges_survive_one_delayed_capture_poll() {
        let (sender, events) = sync_channel(CAPACITY); let lost = Arc::new(AtomicBool::new(false));
        let context = Context { shortcut_press: None, repair_since: None, held: HeldKeys::default(), activity: Activity::default(), sender, capture_thread: 0, lost: lost.clone(), modifiers: Modifiers::default(), generation: Arc::new(AtomicU64::new(0)), shortcut: Arc::new(AtomicU64::new(0)) };
        let now = Instant::now();
        let down = Event { kind: Kind::Down, window: 20, point: Point { x: 10, y: 20 }, sequence: 7, shift: false, at: now, generation: 1, shortcut:None };
        let up = Event { kind: Kind::Up, point: Point { x: 50, y: 20 }, at: now + Duration::from_millis(15), ..down };
        send(&context, down); send(&context, up);
        let observed: Vec<_> = events.try_iter().collect();
        assert_eq!(observed.len(), 2); assert_eq!(observed[0].kind, Kind::Down); assert_eq!(observed[1].kind, Kind::Up);
        assert_eq!(observed[0].sequence, 7); assert_eq!(observed[1].point.x, 50); assert!(!lost.load(Ordering::SeqCst));
    }
    #[test] fn overflow_is_bounded_and_explicitly_marks_lost_selection_evidence() {
        let (sender, events) = sync_channel(CAPACITY); let lost = Arc::new(AtomicBool::new(false));
        let context = Context { shortcut_press: None, repair_since: None, held: HeldKeys::default(), activity: Activity::default(), sender, capture_thread: 0, lost: lost.clone(), modifiers: Modifiers::default(), generation: Arc::new(AtomicU64::new(0)), shortcut: Arc::new(AtomicU64::new(0)) };
        let event = Event { kind: Kind::Down, window: 20, point: Point::default(), sequence: 7, shift: false, at: Instant::now(), generation: 1, shortcut:None };
        for _ in 0..CAPACITY + 10 { send(&context, event); }
        assert_eq!(events.try_iter().count(), CAPACITY); assert!(lost.swap(false, Ordering::SeqCst));
    }
    #[test] fn own_shortcut_preserves_the_snapshot_but_other_keys_invalidate_a_queued_display() {
        let (sender, _) = sync_channel(CAPACITY);
        let generation = Arc::new(AtomicU64::new(10));
        let shortcut = Arc::new(AtomicU64::new(0x44 | (1u64 << 8)));
        let context = Context { shortcut_press: None, repair_since: None, held: HeldKeys::default(), activity: Activity::default(), sender, capture_thread: 0, lost: Arc::new(AtomicBool::new(false)), modifiers: Modifiers::default(), generation: generation.clone(), shortcut: shortcut.clone() };
        assert_eq!(key_generation(&context, 0x44, 1), 10, "Alt+D itself must not discard the selected word");
        let display_generation = generation.load(Ordering::SeqCst);
        assert_eq!(key_generation(&context, 0x25, 0), 11, "arrow key changes selection");
        assert_ne!(generation.load(Ordering::SeqCst), display_generation);
        shortcut.store(0x41 | (3u64 << 8), Ordering::SeqCst);
        assert_eq!(key_generation(&context, 0x41, 3), 11);
        assert_eq!(key_generation(&context, 0x44, 1), 12, "the old shortcut becomes normal input after reconfiguration");
    }
    #[test] fn held_hotkey_letter_survives_modifier_release_and_typematic_repeat() {
        let(sender,_)=sync_channel(CAPACITY);
        let mut context=Context{shortcut_press:None,repair_since:None,held:HeldKeys::default(),activity:Activity::default(),sender,capture_thread:0,lost:Arc::new(AtomicBool::new(false)),modifiers:Modifiers::default(),generation:Arc::new(AtomicU64::new(10)),shortcut:Arc::new(AtomicU64::new(0x44|(1u64<<8)))};
        assert!(key_transition(&mut context,0xA4,true,false).is_none()); assert!(context.activity.stamp().is_none());
        assert_eq!(key_transition(&mut context,0x44,true,true),Some((1,10,Some(0x144))));
        assert!(key_transition(&mut context,0xA4,false,false).is_none()); assert!(context.activity.stamp().is_none(),"D is still physically down");
        assert!(key_transition(&mut context,0x44,true,false).is_none(),"repeat after releasing Alt must not invalidate the one request");
        assert_eq!(context.generation.load(Ordering::SeqCst),10);
        assert!(key_transition(&mut context,0x44,false,false).is_none()); assert!(context.activity.stamp().is_some());
        assert_eq!(key_transition(&mut context,0x25,true,false),Some((0,11,None)),"a real new arrow press still invalidates selection");
        assert_eq!(key_transition(&mut context,0x25,true,false),Some((0,12,None)),"ordinary editing repeats still invalidate a newly queued lookup");
    }
    #[test] fn missed_keyup_recovers_without_a_second_hotkey_or_releasing_real_input() {
        let(sender,_)=sync_channel(CAPACITY);
        let mut context=Context{shortcut_press:None,repair_since:None,held:HeldKeys::default(),activity:Activity::default(),sender,capture_thread:0,lost:Arc::new(AtomicBool::new(false)),modifiers:Modifiers::default(),generation:Arc::new(AtomicU64::new(10)),shortcut:Arc::new(AtomicU64::new(0x44|(1u64<<8)))};
        key_transition(&mut context,0xA4,true,false); key_transition(&mut context,0x44,true,true);
        let now=Instant::now();
        assert!(!repair_released(&mut context,false,now),"held physical input cannot be repaired away");
        assert!(!repair_released(&mut context,true,now));
        assert!(!repair_released(&mut context,true,now+Duration::from_millis(79)));
        assert!(repair_released(&mut context,true,now+Duration::from_millis(80)));
        assert!(context.activity.stamp().is_some()); assert_eq!(context.modifiers.mask(),0);
        assert_eq!(context.generation.load(Ordering::SeqCst),10,"repair is a missed release, not new selection input");
        assert!(key_transition(&mut context,0xA4,false,false).is_none());
        assert!(key_transition(&mut context,0x44,false,false).is_none());
        assert_eq!(context.held.count,0,"late keyup must not underflow");
    }
    #[test] fn a_complete_mouse_click_between_samples_restarts_missed_keyup_recovery() {
        let(sender,_)=sync_channel(CAPACITY);
        let mut context=Context{shortcut_press:None,repair_since:None,held:HeldKeys::default(),activity:Activity::default(),sender,capture_thread:0,lost:Arc::new(AtomicBool::new(false)),modifiers:Modifiers::default(),generation:Arc::new(AtomicU64::new(0)),shortcut:Arc::new(AtomicU64::new(0))};
        key_transition(&mut context,0xA4,true,false); let now=Instant::now();
        assert!(!repair_released(&mut context,true,now));
        assert!(track_edge(&mut context,1,true));assert!(track_edge(&mut context,1,false));
        assert!(!repair_released(&mut context,true,now+Duration::from_millis(79)));
        assert!(!repair_released(&mut context,true,now+Duration::from_millis(158)));
        assert!(repair_released(&mut context,true,now+Duration::from_millis(159)));
    }
    #[test] fn intermittent_physical_down_samples_restart_release_recovery() {
        let(sender,_)=sync_channel(CAPACITY);
        let mut context=Context{shortcut_press:None,repair_since:None,held:HeldKeys::default(),activity:Activity::default(),sender,capture_thread:0,lost:Arc::new(AtomicBool::new(false)),modifiers:Modifiers::default(),generation:Arc::new(AtomicU64::new(0)),shortcut:Arc::new(AtomicU64::new(0))};
        key_transition(&mut context,0xA4,true,false); let now=Instant::now();
        assert!(!repair_released(&mut context,true,now));
        assert!(!repair_released(&mut context,false,now+Duration::from_millis(70)));
        assert!(!repair_released(&mut context,true,now+Duration::from_millis(80)));
        assert!(!repair_released(&mut context,true,now+Duration::from_millis(159)));
        assert!(repair_released(&mut context,true,now+Duration::from_millis(160)));
    }
    #[test] fn modifiers_are_observed_before_dispatch_and_releasing_one_side_keeps_the_other() {
        let mut state = Modifiers::default(); assert!(state.update(0xA4, true)); assert_eq!(state.mask(), 1);
        assert!(!state.update(0x44, true)); assert_eq!(state.mask(), 1, "Alt+D must be recognizable before async state updates");
        state.update(0xA2, true); state.update(0xA3, true); state.update(0xA2, false); assert_eq!(state.mask(), 3);
        state.update(0xA3, false); state.update(0xA4, false); assert_eq!(state.mask(), 0);
        state.update(0xA1, true); state.update(0x5C, true); assert_eq!(state.mask(), 12);
    }
}
