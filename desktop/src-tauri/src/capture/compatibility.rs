//! One recent, gesture-bound selection. Observation never starts a lookup.
use super::{policy::selected_word, selection::Selection};
use std::time::{Duration, Instant};

pub const COPY_WINDOW: Duration = Duration::from_millis(350);
const RETENTION: Duration = Duration::from_secs(30);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source { pub window: isize, pub pid: u32, pub focus: isize }
impl Source {
    fn matches(self, current: Self) -> bool {
        self.window == current.window && self.pid == current.pid
            && (self.focus == 0 || self.focus == current.focus)
    }
}
#[derive(Clone, Debug)]
pub struct Snapshot { pub source: Source, pub sequence: u32, pub word: String, pub revision: u64, expires: Instant }
impl Snapshot {
    pub fn valid_at(&self, sequence: u32, now: Instant) -> bool { self.sequence == sequence && now <= self.expires }
}
pub fn allows_fallback(failure: &Selection) -> bool { matches!(failure, Selection::NoSelection | Selection::Unreported | Selection::Unsupported | Selection::CopyUnavailable) }
struct Pending { source: Source, before: u32, deadline: Instant }
#[derive(Default)]
pub struct RecentSelection { pending: Option<Pending>, snapshot: Option<Snapshot>, revision: u64 }
impl RecentSelection {
    pub fn clear(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.pending = None;
        self.snapshot = None;
    }
    pub fn begin(&mut self, source: Source, before: u32, at: Instant) {
        self.clear();
        self.pending = Some(Pending { source, before, deadline: at + COPY_WINDOW });
    }
    pub fn observe(&mut self, source: Source, sequence: u32, now: Instant) {
        if self.snapshot.as_ref().is_some_and(|v| !v.source.matches(source) || v.sequence != sequence || now > v.expires) {
            self.clear();
        }
        if self.pending.as_ref().is_some_and(|v| !v.source.matches(source) || now > v.deadline) {
            self.pending = None;
        }
    }
    pub fn wants_clipboard(&self, source: Source, sequence: u32, now: Instant) -> bool {
        self.pending.as_ref().is_some_and(|v| v.source.matches(source) && sequence != v.before && now <= v.deadline)
    }
    pub fn clipboard_deadline(&self, source: Source, sequence: u32, now: Instant) -> Option<Instant> {
        self.pending.as_ref().filter(|v|v.source.matches(source) && sequence!=v.before && now<=v.deadline).map(|v|v.deadline)
    }
    // The caller must recheck sequence/foreground after the read as well as owner.
    pub fn accept(&mut self, source: Source, sequence: u32, owner_matches: bool, text: &str, now: Instant) -> bool {
        if !self.wants_clipboard(source, sequence, now) { return false; }
        if !owner_matches { self.clear(); return false; }
        self.pending = None;
        let Some(word) = selected_word(text) else { self.clear(); return false; };
        self.snapshot = Some(Snapshot { source, sequence, word, revision: self.revision, expires: now + RETENTION });
        true
    }
    pub fn snapshot(&self, source: Source, sequence: u32, now: Instant) -> Option<Snapshot> {
        self.snapshot.as_ref().filter(|v| v.source.matches(source) && v.sequence == sequence && now <= v.expires).cloned()
    }
    pub fn fallback(&self, failure: &Selection, source: Source, sequence: u32, now: Instant) -> Option<Snapshot> {
        // Never override a protected field, changed source, invalid selection or access denial.
        if !allows_fallback(failure) { return None; }
        self.snapshot(source, sequence, now)
    }
    pub fn current(&self, revision: u64, source: Source, sequence: u32, now: Instant) -> bool {
        self.snapshot.as_ref().is_some_and(|v| v.revision == revision && v.source.matches(source) && v.sequence == sequence && now <= v.expires)
    }
    pub fn pending(&self) -> bool { self.pending.is_some() }
}

#[derive(Clone, Copy)]
pub enum Input {
    Down { window: isize, x: i32, y: i32, sequence: u32, shift: bool, at: Instant },
    Up { window: isize, x: i32, y: i32, at: Instant },
    Copy { x: i32, y: i32, sequence: u32, at: Instant },
    Changed,
}
#[derive(Clone, Copy)]
pub struct Intent { pub source: Source, pub x: i32, pub y: i32, pub pointer: bool }
#[derive(Default)]
pub struct Effect { pub invalidate: bool, pub lookup: Option<Intent> }
struct Drag { source: Source, x: i32, y: i32, sequence: u32, double_click: bool, shift: bool }
#[derive(Default)]
pub struct Capture { pub recent: RecentSelection, drag: Option<Drag>, last_click: Option<(isize, i32, i32, Instant)>, selection:Option<(Intent,Instant)> }
impl Capture {
    pub fn clear(&mut self) { self.recent.clear(); self.drag = None; self.last_click = None; self.selection=None; }
    // Coordinates only, never retained dictionary text. Ordinary input, another
    // press, a different control or expiry invalidates this pointer evidence.
    pub fn selection_point(&self,source:Source,now:Instant)->Option<(i32,i32)> {
        self.selection.as_ref().filter(|(intent,at)|intent.source==source && now.checked_duration_since(*at).is_some_and(|age|age<=RETENTION))
            .map(|(intent,_)|(intent.x,intent.y))
    }
    pub fn manual(&mut self) { self.drag = None; self.last_click = None; }
    pub fn input(&mut self, input: Input, source: Source, automatic: bool, double_time: Duration) -> Effect {
        match input {
            Input::Down { window, x, y, sequence, shift, at } => {
                self.recent.clear(); self.drag = None; self.selection=None;
                if window == source.window && source.pid != 0 {
                    let double_click = self.last_click.is_some_and(|(old, px, py, time)| old == window
                        && at.saturating_duration_since(time) <= double_time
                        && (i64::from(x) - i64::from(px)).abs() <= 6 && (i64::from(y) - i64::from(py)).abs() <= 6);
                    self.last_click = Some((window, x, y, at));
                    self.drag = Some(Drag { source, x, y, sequence, double_click, shift });
                } else { self.last_click = None; }
                Effect { invalidate: true, lookup: None }
            }
            Input::Up { window, x, y, at } => {
                let Some(drag) = self.drag.take() else { return Effect::default(); };
                if window != source.window || drag.source.window != source.window || drag.source.pid != source.pid
                    || !super::policy::mouse_selection((drag.x, drag.y), (x, y), drag.double_click, drag.shift) {
                    return Effect::default();
                }
                self.recent.begin(source, drag.sequence, at);
                let intent=Intent{source,x,y,pointer:true};self.selection=Some((intent,at));
                Effect { invalidate: false, lookup: automatic.then_some(intent) }
            }
            Input::Copy { x, y, sequence, at } => {
                self.clear();
                if source.pid == 0 { return Effect { invalidate: true, lookup: None }; }
                self.recent.begin(source, sequence, at);
                Effect { invalidate: true, lookup: automatic.then_some(Intent { source, x, y, pointer: false }) }
            }
            Input::Changed => { self.clear(); Effect { invalidate: true, lookup: None } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Source { Source { window: 20, pid: 200, focus: 21 } }
    fn selected() -> (RecentSelection, Instant) {
        let now = Instant::now(); let mut recent = RecentSelection::default();
        recent.begin(source(), 7, now);
        assert!(recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(5)));
        (recent, now)
    }
    fn drag(capture: &mut Capture, automatic: bool, now: Instant) -> Effect {
        assert!(capture.input(Input::Down { window: 20, x: 10, y: 10, sequence: 7, shift: false, at: now }, source(), automatic, Duration::from_millis(500)).lookup.is_none());
        capture.input(Input::Up { window: 20, x: 40, y: 10, at: now + Duration::from_millis(10) }, source(), automatic, Duration::from_millis(500))
    }
    #[test] fn manual_position_requires_the_same_fresh_gesture_even_when_automatic_is_off() {
        let at=Instant::now();let mut capture=Capture::default();drag(&mut capture,false,at);
        capture.manual();let now=at+Duration::from_millis(100);
        assert_eq!(capture.selection_point(source(),now),Some((40,10)));
        for changed in [Source{focus:22,..source()},Source{pid:300,..source()},Source{window:30,..source()}] {assert!(capture.selection_point(changed,now).is_none());}
        assert!(capture.selection_point(source(),at).is_none(),"a future event cannot authorize reading");
        assert!(capture.selection_point(source(),at+Duration::from_secs(31)).is_none());
        capture.input(Input::Changed,source(),false,Duration::from_millis(500));assert!(capture.selection_point(source(),now).is_none());
        drag(&mut capture,false,at);capture.input(Input::Down{window:20,x:60,y:10,sequence:7,shift:false,at:now},source(),false,Duration::from_millis(500));
        assert!(capture.selection_point(source(),now).is_none(),"even a same-source click clears prior selection evidence");
    }
    #[test] fn off_records_a_quick_selection_without_starting_lookup_and_manual_can_use_it() {
        let now = Instant::now(); let mut capture = Capture::default();
        assert!(drag(&mut capture, false, now).lookup.is_none()); assert!(capture.recent.pending());
        assert!(capture.recent.accept(source(), 8, true, "terminal", now + Duration::from_millis(15)));
        assert_eq!(capture.recent.fallback(&Selection::Unsupported, source(), 8, now + Duration::from_millis(20)).unwrap().word, "terminal");
    }
    #[test] fn on_dispatches_the_same_selection_but_off_copy_only_records_it() {
        let now = Instant::now(); let mut capture = Capture::default();
        let intent = drag(&mut capture, true, now).lookup.unwrap(); assert!(intent.pointer); assert_eq!(intent.x, 40);
        let effect = capture.input(Input::Copy { x: 40, y: 10, sequence: 8, at: now }, source(), false, Duration::from_millis(500));
        assert!(effect.invalidate && effect.lookup.is_none()); assert!(capture.recent.pending());
        let effect = capture.input(Input::Copy { x: 40, y: 10, sequence: 8, at: now }, source(), true, Duration::from_millis(500));
        assert!(!effect.lookup.unwrap().pointer);
    }
    #[test] fn ordinary_click_or_keyboard_edit_clears_the_last_word_and_does_not_query() {
        let now = Instant::now(); let mut capture = Capture::default(); drag(&mut capture, false, now);
        assert!(capture.recent.accept(source(), 8, true, "terminal", now));
        let later = now + Duration::from_secs(1);
        capture.input(Input::Down { window: 20, x: 50, y: 20, sequence: 8, shift: false, at: later }, source(), false, Duration::from_millis(500));
        assert!(capture.input(Input::Up { window: 20, x: 50, y: 20, at: later }, source(), false, Duration::from_millis(500)).lookup.is_none());
        assert!(!capture.recent.pending()); assert!(capture.recent.snapshot(source(), 8, later).is_none());
        drag(&mut capture, false, later); capture.recent.accept(source(), 8, true, "run", later);
        capture.input(Input::Changed, source(), false, Duration::from_millis(500));
        assert!(capture.recent.snapshot(source(), 8, later).is_none());
    }
    #[test] fn double_click_shift_selection_and_foreign_mouse_release_are_distinguished() {
        let now = Instant::now(); let mut capture = Capture::default(); let interval = Duration::from_millis(500);
        for at in [now, now + Duration::from_millis(70)] {
            capture.input(Input::Down { window: 20, x: 10, y: 10, sequence: 7, shift: false, at }, source(), false, interval);
            capture.input(Input::Up { window: 20, x: 10, y: 10, at }, source(), false, interval);
        }
        assert!(capture.recent.pending()); capture.clear();
        capture.input(Input::Down { window: 20, x: 10, y: 10, sequence: 7, shift: true, at: now }, source(), true, interval);
        assert!(capture.input(Input::Up { window: 20, x: 10, y: 10, at: now }, source(), true, interval).lookup.is_some());
        capture.clear();
        capture.input(Input::Down { window: 20, x: 10, y: 10, sequence: 7, shift: false, at: now }, source(), true, interval);
        assert!(capture.input(Input::Up { window: 30, x: 40, y: 10, at: now }, source(), true, interval).lookup.is_none());
        assert!(!capture.recent.pending());
    }
    #[test] fn observed_word_is_available_to_manual_lookup_without_automatic_mode() {
        let (recent, now) = selected();
        let word = recent.fallback(&Selection::Unsupported, source(), 8, now).unwrap();
        assert_eq!(word.word, "terminal");
        assert!(recent.current(word.revision, source(), 8, now));
        assert_eq!(recent.fallback(&Selection::NoSelection, source(), 8, now).unwrap().word, "terminal");
    }
    #[test] fn old_clipboard_without_new_gesture_and_sequence_is_never_accepted() {
        let now = Instant::now(); let mut recent = RecentSelection::default();
        assert!(!recent.accept(source(), 8, true, "terminal", now));
        assert!(recent.fallback(&Selection::Unsupported, source(), 8, now).is_none());
        recent.begin(source(), 8, now);
        assert!(!recent.wants_clipboard(source(), 8, now));
        assert!(!recent.accept(source(), 8, true, "terminal", now));
    }
    #[test] fn quick_manual_activation_can_finish_only_the_existing_copy_window() {
        let now = Instant::now(); let mut recent = RecentSelection::default();
        recent.begin(source(), 7, now);
        assert!(recent.snapshot(source(), 7, now).is_none());
        assert!(recent.accept(source(), 8, true, "run", now + Duration::from_millis(349)));
        assert_eq!(recent.fallback(&Selection::Unsupported, source(), 8, now + Duration::from_millis(349)).unwrap().word, "run");
        recent.begin(source(), 8, now);
        assert!(!recent.accept(source(), 9, true, "runtime", now + COPY_WINDOW + Duration::from_millis(1)));
    }
    #[test] fn click_new_gesture_or_keyboard_change_invalidates_the_previous_word() {
        let (mut recent, now) = selected(); let old = recent.snapshot(source(), 8, now).unwrap();
        recent.clear();
        assert!(!recent.current(old.revision, source(), 8, now));
        assert!(recent.fallback(&Selection::NoSelection, source(), 8, now).is_none());
        recent.begin(source(), 8, now);
        assert!(recent.snapshot(source(), 8, now).is_none());
        assert!(recent.accept(source(), 9, true, "run", now));
        assert!(!recent.current(old.revision, source(), 9, now));
        assert_eq!(recent.snapshot(source(), 9, now).unwrap().word, "run");
    }
    #[test] fn source_process_focus_and_sequence_changes_reject_and_clear_a_snapshot() {
        for changed in [Source { window: 30, ..source() }, Source { pid: 300, ..source() }, Source { focus: 22, ..source() }] {
            let (mut recent, now) = selected();
            assert!(recent.snapshot(changed, 8, now).is_none()); recent.observe(changed, 8, now);
            assert!(recent.snapshot(source(), 8, now).is_none(), "returning to source must not revive an old selection");
        }
        let (mut recent, now) = selected(); recent.observe(source(), 9, now);
        assert!(recent.snapshot(source(), 8, now).is_none());
    }
    #[test] fn owner_mismatch_or_invalid_copy_cannot_resurrect_the_previous_word() {
        for (owner, text) in [(false, "run"), (true, "foo\nbar"), (true, ""), (true, "C:\\Users")] {
            let (mut recent, now) = selected(); recent.begin(source(), 8, now);
            assert!(!recent.accept(source(), 9, owner, text, now));
            assert!(recent.snapshot(source(), 9, now).is_none()); assert!(!recent.pending());
        }
    }
    #[test] fn collection_window_and_retention_are_bounded() {
        let now = Instant::now(); let mut recent = RecentSelection::default(); recent.begin(source(), 7, now);
        recent.observe(source(), 8, now + COPY_WINDOW + Duration::from_millis(1)); assert!(!recent.pending());
        let (mut recent, now) = selected();
        recent.observe(source(), 8, now + RETENTION + Duration::from_millis(6)); assert!(recent.snapshot(source(), 8, now).is_none());
    }
    #[test] fn explicit_protection_and_other_read_failures_never_use_compatibility_text() {
        let (recent, now) = selected();
        for failure in [Selection::Protected, Selection::SourceMismatch, Selection::InvalidWord, Selection::AccessDenied, Selection::Timeout, Selection::ReadError("provider".into())] {
            assert!(recent.fallback(&failure, source(), 8, now).is_none(), "{failure:?}");
        }
    }
}
