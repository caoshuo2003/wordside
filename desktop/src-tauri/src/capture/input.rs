//! Read-only, bounded input observation. No input is intercepted or synthesized.
//! Hooks use their own message thread so clipboard/provider work cannot block them.
use std::{cell::RefCell, sync::{Arc, atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering}, mpsc::{sync_channel, Receiver, SyncSender}}, thread::JoinHandle, time::{Duration, Instant}};
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Point { pub x: i32, pub y: i32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Down, Up, Other, Key { code: u32, modifiers: u32 } }
#[derive(Clone, Copy)]
pub struct Event { pub kind: Kind, pub window: isize, pub point: Point, pub sequence: u32, pub shift: bool, pub at: Instant, pub generation: u64 }
#[repr(C)]
struct MousePacket { point: Point, data: u32, flags: u32, time: u32, extra: usize }
#[repr(C)]
struct KeyPacket { code: u32, scan: u32, flags: u32, time: u32, extra: usize }
#[derive(Default)]
#[repr(C)]
struct Message { window: isize, message: u32, wparam: usize, lparam: isize, time: u32, x: i32, y: i32, private: u32 }
#[link(name="user32")]
extern "system" {
    fn SetWindowsHookExW(kind: i32, callback: unsafe extern "system" fn(i32, usize, isize) -> isize, module: isize, thread: u32) -> isize;
    fn UnhookWindowsHookEx(hook: isize) -> i32;
    fn CallNextHookEx(hook: isize, code: i32, wparam: usize, lparam: isize) -> isize;
    fn GetMessageW(message: *mut Message, window: isize, min: u32, max: u32) -> i32;
    fn PeekMessageW(message: *mut Message, window: isize, min: u32, max: u32, remove: u32) -> i32;
    fn PostThreadMessageW(thread: u32, message: u32, wparam: usize, lparam: isize) -> i32;
    fn GetForegroundWindow() -> isize;
    fn WindowFromPoint(point: Point) -> isize;
    fn GetAncestor(window: isize, flags: u32) -> isize;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn GetClipboardSequenceNumber() -> u32;
    fn GetAsyncKeyState(key: i32) -> i16;
}
#[link(name="kernel32")]
extern "system" { fn GetCurrentThreadId() -> u32; fn GetModuleHandleW(name: *const u16) -> isize; }
const WAKE: u32 = 0x8000 + 0x43;
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
struct Context { sender: SyncSender<Event>, capture_thread: u32, lost: Arc<AtomicBool>, modifiers: Modifiers, generation: Arc<AtomicU64>, shortcut: Arc<AtomicU64> }
thread_local! { static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) }; }
fn root(window: isize) -> isize { let root = unsafe { GetAncestor(window, 2) }; if root == 0 { window } else { root } }
fn key_generation(context: &Context, code: u32, modifiers: u32) -> u64 {
    let signature = u64::from(code) | (u64::from(modifiers) << 32);
    if context.shortcut.load(Ordering::SeqCst) == signature { context.generation.load(Ordering::SeqCst) }
    else { context.generation.fetch_add(1, Ordering::SeqCst).wrapping_add(1) }
}
fn send(context: &Context, event: Event) {
    if context.sender.try_send(event).is_err() { context.lost.store(true, Ordering::SeqCst); }
    unsafe { PostThreadMessageW(context.capture_thread, WAKE, 0, 0); }
}
unsafe extern "system" fn mouse_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == 0 && lparam != 0 {
        let kind = match wparam as u32 { 0x201 => Some(Kind::Down), 0x202 => Some(Kind::Up), 0x204 | 0x207 | 0x20B | 0x20A | 0x20E => Some(Kind::Other), _ => None };
        if let Some(kind) = kind {
            let packet = &*(lparam as *const MousePacket);
            CONTEXT.with(|cell| { if let Ok(context) = cell.try_borrow() { if let Some(context) = context.as_ref() {
                let window = if kind == Kind::Down { WindowFromPoint(packet.point) } else { GetForegroundWindow() };
                send(context, Event { kind, window: root(window), point: packet.point, sequence: GetClipboardSequenceNumber(), shift: context.modifiers.mask() & 4 != 0, at: Instant::now(), generation: context.generation.fetch_add(1, Ordering::SeqCst).wrapping_add(1) });
            } } });
        }
    }
    CallNextHookEx(0, code, wparam, lparam)
}
unsafe extern "system" fn key_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == 0 && lparam != 0 {
        let down = matches!(wparam as u32, 0x100 | 0x104);
        let up = matches!(wparam as u32, 0x101 | 0x105);
        if down || up {
            let packet = &*(lparam as *const KeyPacket);
            CONTEXT.with(|cell| { if let Ok(mut context) = cell.try_borrow_mut() { if let Some(context) = context.as_mut() {
                // LL callbacks precede async key-state updates; maintain both sides explicitly.
                if !context.modifiers.update(packet.code, down) && down {
                    let modifiers = context.modifiers.mask() | if packet.flags & 0x20 != 0 { 1 } else { 0 };
                    let generation = key_generation(context, packet.code, modifiers);
                    let mut point = Point::default(); GetCursorPos(&mut point);
                    send(context, Event { kind: Kind::Key { code: packet.code, modifiers }, window: root(GetForegroundWindow()), point, sequence: GetClipboardSequenceNumber(), shift: modifiers & 4 != 0, at: Instant::now(), generation });
                }
            } } });
        }
    }
    CallNextHookEx(0, code, wparam, lparam)
}
struct Hook(isize);
impl Drop for Hook { fn drop(&mut self) { unsafe { UnhookWindowsHookEx(self.0); } } }
pub struct Observer { events: Receiver<Event>, lost: Arc<AtomicBool>, stop: Arc<AtomicBool>, thread: u32, join: Option<JoinHandle<()>>, generation: Arc<AtomicU64>, shortcut: Arc<AtomicU64> }
impl Observer {
    pub fn new(initial_shortcut: u64) -> Result<Self, String> {
        let capture_thread = unsafe { GetCurrentThreadId() };
        let (sender, events) = sync_channel(CAPACITY); let (ready, result) = sync_channel(1);
        let lost = Arc::new(AtomicBool::new(false)); let stop = Arc::new(AtomicBool::new(false)); let thread_id = Arc::new(AtomicU32::new(0));
        let generation = Arc::new(AtomicU64::new(0)); let shortcut = Arc::new(AtomicU64::new(initial_shortcut));
        let hook_generation = generation.clone(); let hook_shortcut = shortcut.clone();
        let hook_lost = lost.clone(); let hook_stop = stop.clone(); let hook_id = thread_id.clone();
        let join = std::thread::spawn(move || unsafe {
            let mut message = Message::default(); PeekMessageW(&mut message, 0, 0, 0, 0);
            let thread = GetCurrentThreadId(); hook_id.store(thread, Ordering::SeqCst);
            let mut modifiers = Modifiers::default();
            for key in [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x5B, 0x5C] { modifiers.update(key, GetAsyncKeyState(key as i32) < 0); }
            CONTEXT.with(|cell| *cell.borrow_mut() = Some(Context { sender, capture_thread, lost: hook_lost, modifiers, generation: hook_generation, shortcut: hook_shortcut }));
            let module = GetModuleHandleW(std::ptr::null());
            let mouse = SetWindowsHookExW(14, mouse_hook, module, 0);
            let mouse = (mouse != 0).then_some(Hook(mouse));
            let keyboard = if mouse.is_some() { SetWindowsHookExW(13, key_hook, module, 0) } else { 0 };
            let keyboard = (keyboard != 0).then_some(Hook(keyboard));
            if mouse.is_none() || keyboard.is_none() {
                let _ = ready.send(Err("兼容取词监听未启动，请重启 WordSide 后重新选词。直接选区取词仍可使用。".to_string()));
            } else {
                let _ = ready.send(Ok(thread));
                while !hook_stop.load(Ordering::SeqCst) && GetMessageW(&mut message, 0, 0, 0) > 0 {}
            }
            drop(keyboard); drop(mouse); CONTEXT.with(|cell| *cell.borrow_mut() = None);
        });
        match result.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(thread)) => Ok(Self { events, lost, stop, thread, join: Some(join), generation, shortcut }),
            response => {
                stop.store(true, Ordering::SeqCst);
                unsafe { PostThreadMessageW(thread_id.load(Ordering::SeqCst), 0x12, 0, 0); }
                // Do not block capture startup if system hook registration itself is stuck.
                drop(join);
                Err(response.ok().and_then(Result::err).unwrap_or_else(|| "兼容取词监听启动超时，请重启 WordSide。".into()))
            }
        }
    }
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
    #[test] fn both_mouse_edges_survive_one_delayed_capture_poll() {
        let (sender, events) = sync_channel(CAPACITY); let lost = Arc::new(AtomicBool::new(false));
        let context = Context { sender, capture_thread: 0, lost: lost.clone(), modifiers: Modifiers::default(), generation: Arc::new(AtomicU64::new(0)), shortcut: Arc::new(AtomicU64::new(0)) };
        let now = Instant::now();
        let down = Event { kind: Kind::Down, window: 20, point: Point { x: 10, y: 20 }, sequence: 7, shift: false, at: now, generation: 1 };
        let up = Event { kind: Kind::Up, point: Point { x: 50, y: 20 }, at: now + Duration::from_millis(15), ..down };
        send(&context, down); send(&context, up);
        let observed: Vec<_> = events.try_iter().collect();
        assert_eq!(observed.len(), 2); assert_eq!(observed[0].kind, Kind::Down); assert_eq!(observed[1].kind, Kind::Up);
        assert_eq!(observed[0].sequence, 7); assert_eq!(observed[1].point.x, 50); assert!(!lost.load(Ordering::SeqCst));
    }
    #[test] fn overflow_is_bounded_and_explicitly_marks_lost_selection_evidence() {
        let (sender, events) = sync_channel(CAPACITY); let lost = Arc::new(AtomicBool::new(false));
        let context = Context { sender, capture_thread: 0, lost: lost.clone(), modifiers: Modifiers::default(), generation: Arc::new(AtomicU64::new(0)), shortcut: Arc::new(AtomicU64::new(0)) };
        let event = Event { kind: Kind::Down, window: 20, point: Point::default(), sequence: 7, shift: false, at: Instant::now(), generation: 1 };
        for _ in 0..CAPACITY + 10 { send(&context, event); }
        assert_eq!(events.try_iter().count(), CAPACITY); assert!(lost.swap(false, Ordering::SeqCst));
    }
    #[test] fn own_shortcut_preserves_the_snapshot_but_other_keys_invalidate_a_queued_display() {
        let (sender, _) = sync_channel(CAPACITY);
        let generation = Arc::new(AtomicU64::new(10));
        let shortcut = Arc::new(AtomicU64::new(0x44 | (1u64 << 32)));
        let context = Context { sender, capture_thread: 0, lost: Arc::new(AtomicBool::new(false)), modifiers: Modifiers::default(), generation: generation.clone(), shortcut: shortcut.clone() };
        assert_eq!(key_generation(&context, 0x44, 1), 10, "Alt+D itself must not discard the selected word");
        let display_generation = generation.load(Ordering::SeqCst);
        assert_eq!(key_generation(&context, 0x25, 0), 11, "arrow key changes selection");
        assert_ne!(generation.load(Ordering::SeqCst), display_generation);
        shortcut.store(0x41 | (3u64 << 32), Ordering::SeqCst);
        assert_eq!(key_generation(&context, 0x41, 3), 11);
        assert_eq!(key_generation(&context, 0x44, 1), 12, "the old shortcut becomes normal input after reconfiguration");
    }
    #[test] fn modifiers_are_observed_before_dispatch_and_releasing_one_side_keeps_the_other() {
        let mut state = Modifiers::default(); assert!(state.update(0xA4, true)); assert_eq!(state.mask(), 1);
        assert!(!state.update(0x44, true)); assert_eq!(state.mask(), 1, "Alt+D must be recognizable before async state updates");
        state.update(0xA2, true); state.update(0xA3, true); state.update(0xA2, false); assert_eq!(state.mask(), 3);
        state.update(0xA3, false); state.update(0xA4, false); assert_eq!(state.mask(), 0);
        state.update(0xA1, true); state.update(0x5C, true); assert_eq!(state.mask(), 12);
    }
}
