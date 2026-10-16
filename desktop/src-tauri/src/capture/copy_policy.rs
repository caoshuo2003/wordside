//! Clipboard ownership rules shared by the native copy transaction and pure tests.
use super::selection::Selection;

pub const INPUT_MARKER: usize = 0x5753_4443;
pub fn own_input(flags: u32, extra: usize) -> bool {
    flags & 0x10 != 0 && extra == INPUT_MARKER
}
pub fn should_verify(selection: &Selection) -> bool {
    matches!(selection, Selection::Word(_) | Selection::Unreported | Selection::Unsupported | Selection::InvalidWord)
}

#[derive(Default, Debug)]
pub struct Receipt {
    before: u32,
    copied: Option<u32>,
    foreign: bool,
}
impl Receipt {
    pub fn new(before: u32) -> Self { Self { before, ..Default::default() } }
    pub fn observe(&mut self, sequence: u32, source_owns: bool, input_current: bool) -> Option<u32> {
        if self.foreign { return None; }
        if let Some(copied) = self.copied {
            if sequence != copied { self.foreign = true; }
        } else if sequence != self.before {
            if source_owns && input_current { self.copied = Some(sequence); }
            else { self.foreign = true; }
        }
        (!self.foreign).then_some(self.copied).flatten()
    }
    pub fn restore_at(&self, sequence: u32, source_owns: bool) -> bool {
        !self.foreign && self.copied == Some(sequence) && source_owns
    }
}

/// The backup is already isolated and complete before this transaction starts.
/// Keeping the orchestration here lets failure/race tests run without desktop input.
pub struct CopiedText {
    pub selection: Selection,
    // Proven while the reader holds the clipboard lock; delayed rendering may
    // advance this sequence without another application replacing the data.
    pub sequence: Option<u32>,
}
pub trait Backend {
    fn valid(&self) -> bool;
    fn release_stamp(&self) -> Option<u64>;
    fn elapsed(&self) -> std::time::Duration;
    fn pause(&mut self, duration: std::time::Duration);
    fn sequence(&mut self) -> u32;
    fn source_owns(&self) -> bool;
    fn send_copy(&mut self) -> bool;
    fn read(&mut self, sequence: u32) -> Option<CopiedText>;
    fn restore(&mut self, sequence: Option<u32>) -> bool;
}
pub fn transact(backend: &mut impl Backend, before: u32) -> Option<Selection> {
    use std::time::Duration;
    let mut receipt = Receipt::new(before);
    let mut copied = None;
    let outcome = (|| {
        let mut release=super::timing::Released::default();
        let started=std::time::Instant::now();
        while !release.ready(backend.release_stamp(),started+backend.elapsed()) {
            if !backend.valid() { return None; }
            if backend.elapsed() >= Duration::from_millis(650) { return Some(Selection::CopyUnavailable); }
            backend.pause(Duration::from_millis(10));
        }
        if !backend.valid() { return None; }
        if backend.sequence() != before { return Some(Selection::ClipboardUnavailable); }
        let sent = backend.send_copy();
        let until = backend.elapsed() + Duration::from_millis(1000);
        loop {
            let current = backend.valid();
            let sequence = backend.sequence();
            if let Some(sequence) = receipt.observe(sequence, backend.source_owns(), current) {
                copied = Some(sequence);
                if !current { return None; }
                return if sent {
                    Some(backend.read(sequence).map(|read| {
                        if let Some(materialized) = read.sequence { copied = Some(materialized); }
                        read.selection
                    }).unwrap_or(Selection::CopyUnavailable))
                } else { Some(Selection::CopyUnavailable) };
            }
            if !current { return None; }
            if backend.elapsed() >= until { return Some(Selection::CopyUnavailable); }
            backend.pause(Duration::from_millis(5));
        }
    })();
    if !backend.restore(copied) { return Some(Selection::ClipboardRestoreFailed); }
    if !backend.valid() { return None; }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn shifted_or_unsupported_providers_are_verified_but_explicit_no_selection_is_not_copied() {
        for value in [Selection::Word("wrong neighbor".into()), Selection::Unsupported, Selection::InvalidWord] {
            assert!(should_verify(&value));
        }
        for value in [Selection::NoSelection, Selection::Protected, Selection::SourceMismatch,
            Selection::Timeout, Selection::AccessDenied, Selection::ReadError("provider".into()), Selection::CopyUnavailable] {
            assert!(!should_verify(&value), "{value:?}");
        }
    }
    #[test] fn old_clipboard_and_unknown_owner_never_authorize_a_lookup_or_restore() {
        let mut receipt = Receipt::new(10);
        assert_eq!(receipt.observe(10, true, true), None);
        assert!(!receipt.restore_at(10, true));
        assert_eq!(receipt.observe(11, false, true), None);
        assert_eq!(receipt.observe(12, true, true), None);
        assert!(!receipt.restore_at(12, true));
    }
    #[test] fn user_copy_from_same_or_different_source_is_not_overwritten() {
        for owner in [true, false] {
            let mut receipt = Receipt::new(10);
            assert_eq!(receipt.observe(11, true, true), Some(11));
            assert!(!receipt.restore_at(12, owner));
            assert_eq!(receipt.observe(12, owner, true), None);
            assert!(!receipt.restore_at(11, true), "a foreign update permanently ends the transaction");
        }
        let mut receipt = Receipt::new(10);
        assert_eq!(receipt.observe(11, true, false), None, "input changed before this copy could be attributed");
        assert!(!receipt.restore_at(11, true));
    }
    #[test] fn cancellation_after_an_attributed_copy_still_allows_cleanup() {
        let mut receipt = Receipt::new(u32::MAX);
        assert_eq!(receipt.observe(0, true, true), Some(0));
        assert_eq!(receipt.observe(0, true, false), Some(0));
        assert!(receipt.restore_at(0, true));
        assert!(!receipt.restore_at(0, false));
    }
    #[test] fn only_our_tagged_synthetic_keyboard_input_is_ignored() {
        assert!(own_input(0x10, INPUT_MARKER));
        assert!(!own_input(0, INPUT_MARKER));
        assert!(!own_input(0x10, 123));
        assert!(!own_input(0x10, 0));
    }
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    use std::time::Duration;
    struct Fake {
        time: Duration, release: Duration, sequence: u32, current: bool, owner: bool,
        sent: bool, copies: usize, reads: usize, cleanups: Vec<Option<u32>>,
        release_trace: Vec<(Duration,Option<u64>)>, read_missing: bool, response_after: Option<Duration>, on_read: Option<(bool,u32)>, materialized: Option<u32>, restore_ok: bool,
    }
    impl Default for Fake {
        fn default() -> Self { Self { time:Duration::ZERO,release:Duration::ZERO,sequence:10,current:true,
            owner:true,sent:true,copies:0,reads:0,cleanups:vec![],response_after:Some(Duration::from_millis(10)),
            release_trace:vec![],read_missing:false,on_read:None,materialized:None,restore_ok:true } }
    }
    impl Backend for Fake {
        fn valid(&self)->bool{self.current}
        fn release_stamp(&self)->Option<u64>{
            if !self.release_trace.is_empty() {return self.release_trace.iter().rev().find(|(time,_)|self.time>=*time).and_then(|(_,stamp)|*stamp);}
            (self.time>=self.release).then_some(1)
        }
        fn elapsed(&self)->Duration{self.time}
        fn pause(&mut self,duration:Duration){self.time+=duration;}
        fn sequence(&mut self)->u32{
            if self.copies>0&&self.response_after.is_some_and(|at|self.time>=at)&&self.sequence==10{self.sequence=11;}
            self.sequence
        }
        fn source_owns(&self)->bool{self.owner}
        fn send_copy(&mut self)->bool{self.copies+=1;self.sent}
        fn read(&mut self,sequence:u32)->Option<CopiedText>{
            self.reads+=1;
            if self.read_missing {return None;}
            let proven=self.materialized.unwrap_or(sequence);
            if self.materialized.is_some(){self.sequence=proven;}
            // This update happens after the reader released its clipboard lock.
            if let Some((current,sequence))=self.on_read{self.current=current;self.sequence=sequence;}
            Some(CopiedText{selection:Selection::Word("runtime".into()),sequence:Some(proven)})
        }
        fn restore(&mut self,sequence:Option<u32>)->bool{
            self.cleanups.push(sequence);
            if sequence==Some(self.sequence)&&self.owner&&self.restore_ok{self.sequence=10;}
            self.restore_ok
        }
    }
    #[test]fn corrected_copy_is_returned_once_after_original_data_is_restored(){
        let mut backend=Fake::default();
        assert_eq!(transact(&mut backend,10),Some(Selection::Word("runtime".into())));
        assert_eq!((backend.copies,backend.reads,backend.sequence),(1,1,10));
        assert_eq!(backend.cleanups,vec![Some(11)]);
    }
    #[test]fn held_shortcut_waits_and_cannot_be_mixed_with_the_copy_chord(){
        let mut backend=Fake{release:Duration::from_millis(100),..Fake::default()};
        assert!(transact(&mut backend,10).is_some());assert!(backend.time>=backend.release);assert_eq!(backend.copies,1);
        let mut backend=Fake{release:Duration::from_secs(3),..Fake::default()};
        assert_eq!(transact(&mut backend,10),Some(Selection::CopyUnavailable));
        assert_eq!(backend.copies,0);assert_eq!(backend.cleanups,vec![None]);
    }
    #[test]fn a_new_key_edge_during_release_settling_delays_the_single_copy() {
        let mut backend=Fake{release_trace:vec![(Duration::ZERO,None),(Duration::from_millis(100),Some(2)),(Duration::from_millis(110),None),(Duration::from_millis(120),Some(4))],..Fake::default()};
        assert_eq!(transact(&mut backend,10),Some(Selection::Word("runtime".into())));
        assert!(backend.time>=Duration::from_millis(160)); assert_eq!(backend.copies,1);
    }
    #[test]fn a_reader_without_a_reply_still_restores_and_finishes_the_first_request() {
        let mut backend=Fake{read_missing:true,..Fake::default()};
        assert_eq!(transact(&mut backend,10),Some(Selection::CopyUnavailable));
        assert_eq!(backend.copies,1); assert_eq!(backend.cleanups,vec![Some(11)]); assert_eq!(backend.sequence,10);
    }
    #[test]fn cancelled_or_changed_source_before_dispatch_never_synthesizes_input(){
        let mut backend=Fake{current:false,..Fake::default()};
        assert_eq!(transact(&mut backend,10),None);assert_eq!(backend.copies,0);assert_eq!(backend.cleanups,vec![None]);
        let mut backend=Fake{sequence:12,..Fake::default()};
        assert_eq!(transact(&mut backend,10),Some(Selection::ClipboardUnavailable));assert_eq!(backend.copies,0);assert_eq!(backend.sequence,12);
    }
    #[test]fn hung_or_unsupported_copy_never_reads_old_clipboard(){
        let mut backend=Fake{response_after:None,..Fake::default()};
        assert_eq!(transact(&mut backend,10),Some(Selection::CopyUnavailable));
        assert_eq!(backend.reads,0);assert_eq!(backend.sequence,10);assert_eq!(backend.cleanups,vec![None]);
    }
    #[test]fn late_source_response_within_budget_is_cleaned_up(){
        let mut backend=Fake{response_after:Some(Duration::from_millis(900)),..Fake::default()};
        assert_eq!(transact(&mut backend,10),Some(Selection::Word("runtime".into())));
        assert_eq!(backend.sequence,10);assert_eq!(backend.reads,1);
    }
    #[test]fn cancellation_during_read_restores_ours_and_does_not_publish_the_result(){
        let mut backend=Fake{on_read:Some((false,11)),..Fake::default()};
        assert_eq!(transact(&mut backend,10),None);assert_eq!(backend.sequence,10);
    }
    #[test]fn a_new_copy_even_in_the_same_source_wins_over_the_backup(){
        let mut backend=Fake{on_read:Some((true,12)),..Fake::default()};
        let _=transact(&mut backend,10);assert_eq!(backend.sequence,12);assert_eq!(backend.cleanups,vec![Some(11)]);
    }
    #[test]fn delayed_rendering_advances_the_attributed_sequence_and_still_restores(){
        let mut backend=Fake{materialized:Some(12),..Fake::default()};
        assert_eq!(transact(&mut backend,10),Some(Selection::Word("runtime".into())));
        assert_eq!(backend.cleanups,vec![Some(12)]);assert_eq!(backend.sequence,10);
    }
    #[test]fn cancellation_after_materialization_restores_final_sequence_without_returning_the_word(){
        let mut backend=Fake{materialized:Some(12),on_read:Some((false,12)),..Fake::default()};
        assert_eq!(transact(&mut backend,10),None);
        assert_eq!(backend.cleanups,vec![Some(12)]);assert_eq!(backend.sequence,10);
    }
    #[test]fn user_copy_after_delayed_rendering_is_never_overwritten(){
        let mut backend=Fake{materialized:Some(12),on_read:Some((true,13)),..Fake::default()};
        let _=transact(&mut backend,10);
        assert_eq!(backend.cleanups,vec![Some(12)]);assert_eq!(backend.sequence,13);
    }
    #[test]fn partial_injection_still_cleans_up_without_using_the_text(){
        let mut backend=Fake{sent:false,..Fake::default()};
        assert_eq!(transact(&mut backend,10),Some(Selection::CopyUnavailable));
        assert_eq!(backend.reads,0);assert_eq!(backend.sequence,10);
    }
    #[test]fn failed_restore_is_visible_and_never_returns_a_word(){
        let mut backend=Fake{restore_ok:false,..Fake::default()};
        assert_eq!(transact(&mut backend,10),Some(Selection::ClipboardRestoreFailed));
    }
}
