use std::time::{Duration, Instant};
#[derive(Clone, Copy, PartialEq)]
pub enum Focus { Unknown, Source, Popup, Other }
pub struct Snapshot {
    pub visible: bool, pub pinned: bool, pub press_started: bool,
    pub mouse_down: bool, pub inside_popup: bool, pub focus: Focus,
}
#[derive(Default)]
pub struct Dismissal { away_since: Option<Instant>, hide_pending: bool }
impl Dismissal {
    // Main-thread hiding is asynchronous. Keep that transition latched even if
    // another input batch observes the old native visibility before it runs.
    pub fn showing(&mut self) { self.hide_pending=false; self.away_since=None; }
    pub fn update(&mut self, now: Instant, state: Snapshot) -> bool {
        if !state.visible { self.hide_pending=false; self.away_since=None; return false; }
        if state.pinned || self.hide_pending { self.away_since = None; return false; }
        if state.press_started && !state.inside_popup { self.away_since = None; self.hide_pending=true; return true; }
        // Activation can temporarily yield NULL; it does not prove an outside click.
        if state.focus != Focus::Other || (state.mouse_down && state.inside_popup) {
            self.away_since = None; return false;
        }
        let since = *self.away_since.get_or_insert(now);
        let hide=now.saturating_duration_since(since) >= Duration::from_millis(160);
        if hide { self.hide_pending=true; }
        hide
    }
}
#[cfg(windows)]
pub fn belongs_to_window(window: isize, target: isize) -> bool {
    #[link(name = "user32")] extern "system" {
        fn GetAncestor(window:isize, flags:u32)->isize;
        fn GetWindow(window:isize, command:u32)->isize;
        fn IsChild(parent:isize, child:isize)->i32;
    }
    if window == 0 || target == 0 { return false; }
    unsafe {
        if window == target || IsChild(target, window) != 0 { return true; }
        let root = GetAncestor(window, 2); let mut current = if root != 0 { root } else { window };
        for _ in 0..16 {
            if current == target || IsChild(target, current) != 0 { return true; }
            current = GetWindow(current, 4); // GW_OWNER: covers owned WebView/context windows.
            if current == 0 { break; }
        }
    }
    false
}
#[cfg(test)] mod tests {
    use super::*;
    fn state(focus:Focus)->Snapshot { Snapshot {visible:true,pinned:false,press_started:false,mouse_down:false,inside_popup:true,focus} }
    #[test] fn inside_click_and_activation_gaps_keep_card() {
        let mut policy=Dismissal::default();let now=Instant::now();
        let mut click=state(Focus::Unknown);click.press_started=true;click.mouse_down=true;
        assert!(!policy.update(now,click));
        assert!(!policy.update(now+Duration::from_millis(200),state(Focus::Unknown)));
        assert!(!policy.update(now+Duration::from_millis(400),state(Focus::Popup)));
        assert!(!policy.update(now+Duration::from_millis(600),state(Focus::Source)));
    }
    #[test] fn transient_focus_does_not_hide_but_real_departure_does() {
        let mut policy=Dismissal::default();let now=Instant::now();
        assert!(!policy.update(now,state(Focus::Other)));
        assert!(!policy.update(now+Duration::from_millis(80),state(Focus::Popup)));
        assert!(!policy.update(now+Duration::from_millis(500),state(Focus::Other)));
        assert!(policy.update(now+Duration::from_millis(660),state(Focus::Other)));
    }
    #[test] fn outside_click_is_immediate_and_pin_is_respected() {
        let mut policy=Dismissal::default();let now=Instant::now();let mut click=state(Focus::Other);
        click.press_started=true;click.mouse_down=true;click.inside_popup=false;
        assert!(policy.update(now,click));
        let mut pin=state(Focus::Other);pin.pinned=true;pin.press_started=true;pin.inside_popup=false;
        assert!(!policy.update(now,pin));assert!(!policy.update(now+Duration::from_secs(1),state(Focus::Other)));
        let mut hidden=state(Focus::Other);hidden.visible=false;assert!(!policy.update(now+Duration::from_secs(2),hidden));
    }
    #[test] fn capture_race_a_new_card_rearms_dismissal_even_if_hidden_visibility_was_not_polled() {
        let mut policy=Dismissal::default(); let at=Instant::now();
        let click=||Snapshot{visible:true,pinned:false,press_started:true,mouse_down:true,inside_popup:false,focus:Focus::Source};
        assert!(policy.update(at,click()));
        assert!(!policy.update(at+Duration::from_millis(20),click()));
        policy.showing();
        assert!(policy.update(at+Duration::from_millis(90),click()));
    }
    #[cfg(windows)] #[test] fn hidden_native_windows_cover_child_and_owner_relationships() {
        // No WS_VISIBLE, ShowWindow, SetFocus or input. Each HWND is destroyed on this thread.
        #[link(name="user32")] extern "system" {
            fn CreateWindowExW(ex:u32,class:*const u16,title:*const u16,style:u32,x:i32,y:i32,w:i32,h:i32,parent:isize,menu:isize,instance:isize,param:*const std::ffi::c_void)->isize;
            fn DestroyWindow(window:isize)->i32;
        }
        struct Hidden(isize);impl Drop for Hidden {fn drop(&mut self){unsafe{DestroyWindow(self.0);}}}
        let class:Vec<u16>="STATIC\0".encode_utf16().collect();
        let create=|style,parent| {let handle=unsafe{CreateWindowExW(0,class.as_ptr(),class.as_ptr(),style,0,0,10,10,parent,0,0,std::ptr::null())};assert_ne!(handle,0);Hidden(handle)};
        let popup=create(0x80000000,0);let child=create(0x40000000,popup.0);let owned=create(0x80000000,popup.0);
        let owned_child=create(0x40000000,owned.0);let foreign=create(0x80000000,0);
        for handle in [popup.0,child.0,owned.0,owned_child.0] {assert!(belongs_to_window(handle,popup.0));}
        assert!(!belongs_to_window(foreign.0,popup.0));assert!(!belongs_to_window(0,popup.0));
    }
}
