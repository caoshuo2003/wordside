//! Physical input and stable release fences, independent of the desktop APIs.
use std::{sync::{Arc, atomic::{AtomicU64, Ordering}}, time::{Duration, Instant}};
pub const RELEASE_SETTLE: Duration = Duration::from_millis(40);
#[derive(Clone, Default)]
pub struct Activity(Arc<AtomicU64>);
impl Activity {
    pub fn changed(&self, held: bool) {
        let value = self.0.load(Ordering::SeqCst);
        self.0.store((value.wrapping_add(2) & !1) | u64::from(held), Ordering::SeqCst);
    }
    pub fn stamp(&self) -> Option<u64> {
        let value = self.0.load(Ordering::SeqCst);
        (value & 1 == 0).then_some(value)
    }
}
#[derive(Default)]
pub struct Released { since: Option<(u64, Instant)> }
impl Released {
    pub fn ready(&mut self, stamp: Option<u64>, now: Instant) -> bool {
        let Some(stamp) = stamp else { self.since = None; return false; };
        if let Some((old, since)) = self.since {
            if old == stamp { return now.saturating_duration_since(since) >= RELEASE_SETTLE; }
        }
        self.since = Some((stamp, now)); false
    }
}
/// A provider may materialize delayed data and advance the clipboard sequence.
/// Discard that whole attempt and take a fresh coherent snapshot, with a bound.
pub fn stable_backup<T, E>(mut take: impl FnMut() -> Result<T,E>, retryable: impl Fn(&E) -> bool) -> Result<T,E> {
    let started = Instant::now();
    for attempt in 0..3 {
        match take() {
            Err(error) if attempt < 2 && retryable(&error) && started.elapsed() < Duration::from_millis(450) => {},
            result => return result,
        }
    }
    unreachable!()
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn every_key_edge_restarts_the_release_fence_even_between_polls() {
        let activity=Activity::default(); let mut released=Released::default(); let now=Instant::now();
        assert!(!released.ready(activity.stamp(), now));
        assert!(released.ready(activity.stamp(), now+RELEASE_SETTLE));
        activity.changed(true); assert!(!released.ready(activity.stamp(), now+RELEASE_SETTLE));
        activity.changed(false); assert!(!released.ready(activity.stamp(), now+RELEASE_SETTLE));
        assert!(released.ready(activity.stamp(), now+RELEASE_SETTLE*2));
        activity.changed(true); activity.changed(false);
        assert!(!released.ready(activity.stamp(), now+RELEASE_SETTLE*3));
    }
    #[test] fn delayed_rendering_is_retried_with_a_whole_new_snapshot() {
        let mut calls=0;
        let result=stable_backup(|| { calls+=1; if calls<3 {Err("changed")} else {Ok(vec![calls])} }, |error|*error=="changed");
        assert_eq!(result, Ok(vec![3])); assert_eq!(calls,3);
    }
    #[test] fn continuously_changing_or_unsupported_clipboard_is_bounded() {
        for (error, expected) in [("changed",3),("unsupported",1)] {
            let mut calls=0;
            let result:Result<(),_>=stable_backup(|| {calls+=1; Err(error)}, |error|*error=="changed");
            assert_eq!(result,Err(error)); assert_eq!(calls,expected);
        }
    }
}
