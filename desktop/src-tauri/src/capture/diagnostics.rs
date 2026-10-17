//! One bounded failure snapshot. Never serialize a Selection, request or text.
use super::{selection::Selection,windows_copy::{BackupFailure,CopyDiagnostics}};
use std::{path::{Path,PathBuf},time::{Instant,SystemTime,UNIX_EPOCH}};
#[derive(serde::Serialize)]
#[serde(rename_all="snake_case")]
pub enum Phase { Provider, Backup, Copy }
#[derive(serde::Serialize)]
pub struct FailureReport {
    version: &'static str,
    timestamp_ms: u128,
    elapsed_ms: u128,
    pub phase: Phase,
    pub provider: &'static str,
    #[serde(skip_serializing_if="Option::is_none")]
    pub provider_detail: Option<super::windows_text::ProviderDiagnostics>,
    outcome: &'static str,
    #[serde(skip_serializing_if="Option::is_none")]
    pub request_current: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub input_current: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub backup: Option<BackupFailure>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub copy: Option<CopyDiagnostics>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub copy_read: Option<super::windows_copy::ReadDiagnostics>,
}
pub fn code(selection: Option<&Selection>) -> &'static str {
    selection.map(|value|value.failure().map(|(code,_)|code).unwrap_or("word")).unwrap_or("cancelled")
}
impl FailureReport {
    pub fn new() -> Self {
        Self { version:env!("CARGO_PKG_VERSION"),timestamp_ms:SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),elapsed_ms:0,
            phase:Phase::Provider,provider:"pending",provider_detail:None,outcome:"pending",request_current:None,input_current:None,backup:None,copy:None,copy_read:None }
    }
    pub fn finish(mut self, recorder: &FailureRecorder, outcome: Option<&Selection>, started: Instant) {
        if matches!(outcome,Some(Selection::Word(_))) {return;}
        self.outcome=code(outcome);self.elapsed_ms=started.elapsed().as_millis();
        recorder.save(&self);
    }
}
pub struct FailureRecorder(Option<PathBuf>);
impl FailureRecorder {
    pub fn new(path: Option<PathBuf>) -> Self {Self(path)}
    fn save(&self, report: &FailureReport) {
        let Some(path)=self.0.as_deref()else{return;};
        let Ok(bytes)=serde_json::to_vec_pretty(report)else{return;};
        // Fixed metadata fields only; retain one snapshot, never an append log.
        if bytes.len()<=4096 && path.parent().is_some_and(Path::is_dir) {let _=std::fs::write(path,bytes);}
    }
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn failure_snapshot_excludes_selected_words_raw_errors_and_successful_history(){
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("capture-status.json");
        let recorder=FailureRecorder::new(Some(path.clone()));let mut report=FailureReport::new();
        report.provider=code(Some(&Selection::Word("PRIVATE_SELECTED_WORD".into())));
        report.finish(&recorder,Some(&Selection::ReadError("PRIVATE_OS_DETAIL".into())),Instant::now());
        let saved=std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("read_error"));assert!(saved.contains("word"));
        assert!(!saved.contains("PRIVATE_"));assert!(saved.len()<4096);
        FailureReport::new().finish(&recorder,Some(&Selection::Word("SECOND_PRIVATE_WORD".into())),Instant::now());
        assert_eq!(std::fs::read_to_string(&path).unwrap(),saved,"successful words never create a query history");
        FailureReport::new().finish(&recorder,Some(&Selection::Timeout),Instant::now());
        let saved=std::fs::read_to_string(path).unwrap();assert!(saved.contains("timeout"));assert!(!saved.contains("read_error"),"only one failure is retained");
    }
}
