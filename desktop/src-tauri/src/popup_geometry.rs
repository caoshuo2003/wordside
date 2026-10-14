//! Popup size preferences and suppression of programmatic resize notifications.
//! Positions stay with the visible pinned window; only logical dimensions are saved.
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::VecDeque;

pub const MIN_WIDTH: u32 = 320;
pub const MIN_HEIGHT: u32 = 360;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PopupSize { pub width: u32, pub height: u32 }
impl Default for PopupSize { fn default() -> Self { Self { width:360, height:600 } } }
impl PopupSize {
    pub fn from_physical(width:u32,height:u32,scale:f64)->Option<Self> {
        if width==0 || height==0 || !scale.is_finite() || scale<=0.0 { return None; }
        Some(Self { width:((width as f64/scale).round() as u32).clamp(MIN_WIDTH,8192), height:((height as f64/scale).round() as u32).clamp(MIN_HEIGHT,8192) })
    }
    pub fn fit(self,scale:f64,work_width:i32,work_height:i32)->(u32,u32) {
        let scale=if scale.is_finite() && scale>0.0 {scale}else{1.0};
        (((self.width as f64*scale).round() as u32).min(work_width.saturating_sub(16).max(1) as u32),
         ((self.height as f64*scale).round() as u32).min(work_height.saturating_sub(16).max(1) as u32))
    }
}
pub fn deserialize_size<'de,D:Deserializer<'de>>(deserializer:D)->Result<PopupSize,D::Error> {
    let value=serde_json::Value::deserialize(deserializer)?;
    let default=PopupSize::default();
    let dimension=|name:&str,fallback:u32,minimum:u32| value[name].as_u64().filter(|n|*n>0&&*n<=8192).map(|n|(n as u32).max(minimum)).unwrap_or(fallback);
    Ok(PopupSize {width:dimension("width",default.width,MIN_WIDTH),height:dimension("height",default.height,MIN_HEIGHT)})
}
#[derive(Default)]
pub struct GeometryState { tracking:bool, dpi_resize_pending:bool, last_observed:Option<PopupSize>, expected:VecDeque<PopupSize>, generation:u64, saving:bool, pub dirty:bool }
impl GeometryState {
    fn expect(&mut self,size:PopupSize) {
        if !self.expected.contains(&size) {self.expected.push_back(size);}
        if self.expected.len()>8 {self.expected.pop_front();}
    }
    pub fn programmatic(&mut self,size:PopupSize) {
        // Called before moving/resizing. Hidden startup notifications must not
        // replace a preference loaded from disk before the first lookup.
        self.tracking=true;self.expect(size);
    }
    pub fn dpi_changed(&mut self) {
        // The next native resize is the OS DPI adjustment. On Windows 11 its
        // actual dimensions can differ from ScaleFactorChanged.new_inner_size,
        // so do not predict them or discard the final programmatic target.
        if self.tracking {self.dpi_resize_pending=true;}
    }
    pub fn observed(&mut self,width:u32,height:u32,scale:f64)->Option<PopupSize> {
        if !self.tracking {return None;}
        let next=PopupSize::from_physical(width,height,scale)?;
        let dpi_resize=std::mem::take(&mut self.dpi_resize_pending);
        if self.last_observed==Some(next) {return None;}
        self.last_observed=Some(next);
        if let Some(index)=self.expected.iter().position(|size|*size==next) {self.expected.remove(index);return None;}
        if dpi_resize {return None;}
        // An unpredicted event is a user resize. Discard obsolete/coalesced
        // layout expectations so they cannot swallow a later manual resize.
        self.expected.clear();Some(next)
    }
    pub fn queue_save(&mut self)->Option<u64> {
        self.generation=self.generation.wrapping_add(1);self.dirty=true;
        if self.saving {None}else{self.saving=true;Some(self.generation)}
    }
    pub fn ready(&mut self,token:u64)->Result<(),u64> {
        if token!=self.generation {Err(self.generation)}else{self.saving=false;Ok(())}
    }
    pub fn saved(&mut self,token:u64) {if token==self.generation{self.dirty=false;}}
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn logical_size_survives_dpi_changes() {
        assert_eq!(PopupSize::from_physical(780,960,1.5),Some(PopupSize {width:520,height:640}));
        assert_eq!(PopupSize::from_physical(1040,1280,2.0),Some(PopupSize {width:520,height:640}));
        for (w,h,s) in [(0,600,1.0),(360,0,1.0),(360,600,0.0),(360,600,f64::NAN)] {assert!(PopupSize::from_physical(w,h,s).is_none());}
    }
    #[test] fn fit_small_monitor_without_replacing_preference() {
        let size=PopupSize {width:1000,height:1000};assert_eq!(size.fit(1.5,800,700),(784,684));assert_eq!(size.fit(1.0,2000,1200),(1000,1000));
        assert_eq!(PopupSize::default().fit(1.0,1920,1080),(360,600));
    }
    #[test] fn layout_events_do_not_replace_user_size() {
        let mut state=GeometryState::default();state.programmatic(PopupSize::default());
        assert_eq!(state.observed(540,900,1.5),None);assert!(!state.dirty);
        assert_eq!(state.observed(780,960,1.5),Some(PopupSize{width:520,height:640}));
        state.programmatic(PopupSize{width:520,height:640});state.programmatic(PopupSize::default());
        assert_eq!(state.observed(360,600,1.0),None);assert_eq!(state.observed(600,700,1.0),Some(PopupSize{width:600,height:700}));assert!(state.expected.is_empty());
    }
    #[test] fn startup_notifications_do_not_overwrite_saved_preference() {
        let mut state=GeometryState::default();
        state.dpi_changed();
        assert_eq!(state.observed(360,600,1.0),None);
        assert!(!state.dirty);
        state.programmatic(PopupSize{width:520,height:640});
        assert_eq!(state.observed(780,960,1.5),None);
        assert_eq!(state.observed(900,1050,1.5),Some(PopupSize{width:600,height:700}));
    }
    #[test] fn intermediate_dpi_resize_does_not_replace_unclamped_preference() {
        let mut state=GeometryState::default();
        let preferred=PopupSize{width:1000,height:1000};
        state.programmatic(PopupSize{width:784,height:684});
        assert_eq!(state.observed(784,684,1.0),None);
        // Move to a larger 150% screen: WM_DPICHANGED preserves the old fitted
        // logical dimensions before the final requested 1000x1000 arrives.
        state.programmatic(preferred);
        state.dpi_changed();
        assert_eq!(state.observed(1176,1026,1.5),None);
        assert_eq!(state.observed(1500,1500,1.5),None);
        assert_eq!(state.observed(1560,1530,1.5),Some(PopupSize{width:1040,height:1020}));
        assert!(!state.dirty);
    }
    #[test] fn windows_11_actual_dpi_size_can_differ_from_the_suggested_inner_size() {
        let mut state=GeometryState::default();let preferred=PopupSize{width:1000,height:1000};
        state.programmatic(preferred);state.dpi_changed();
        // The OS rectangle is not necessarily Tao's predicted 1176x1026.
        assert_eq!(state.observed(1194,1044,1.5),None);
        assert_eq!(state.observed(1500,1500,1.5),None);
        assert_eq!(state.observed(1530,1545,1.5),Some(PopupSize{width:1020,height:1030}));
        // Coalescing to the final size must also end the DPI suppression.
        state.programmatic(preferred);state.dpi_changed();
        assert_eq!(state.observed(1500,1500,1.5),None);
        assert_eq!(state.observed(1560,1560,1.5),Some(PopupSize{width:1040,height:1040}));
    }
    #[test] fn repeated_programmatic_notifications_do_not_save_the_fitted_size() {
        let mut state=GeometryState::default();let fitted=PopupSize{width:784,height:684};
        state.programmatic(fitted);
        assert_eq!(state.observed(784,684,1.0),None);
        assert_eq!(state.observed(784,684,1.0),None);
        assert_eq!(state.observed(820,710,1.0),Some(PopupSize{width:820,height:710}));
        assert_eq!(state.observed(820,710,1.0),None);
    }
    #[test] fn rapid_resizes_save_only_the_latest_and_keep_later_changes_dirty() {
        let mut state=GeometryState::default();let first=state.queue_save().unwrap();assert!(state.queue_save().is_none());
        let latest=state.ready(first).unwrap_err();assert!(state.ready(latest).is_ok());
        let next=state.queue_save().unwrap();state.saved(latest);assert!(state.dirty);
        assert!(state.ready(next).is_ok());state.saved(next);assert!(!state.dirty);
    }
}
