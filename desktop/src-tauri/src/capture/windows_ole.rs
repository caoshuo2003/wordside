//! Materialize OLE payloads before copying. No old IDataObject survives this call.
//! Restore uses Win32 static formats under the existing sequence/owner lock;
//! Windows supplies a fresh default OLE wrapper rather than stale provider handles.
use super::*;
use ::windows::{core::Error, Win32::System::{Com::{IDataObject, FORMATETC, STGMEDIUM, CoTaskMemFree},
    Ole::{OleInitialize, OleUninitialize, OleGetClipboard, ReleaseStgMedium}}};
struct Apartment;
impl Drop for Apartment { fn drop(&mut self) { unsafe { OleUninitialize(); } } }
struct Medium(STGMEDIUM);
impl Drop for Medium { fn drop(&mut self) { unsafe { ReleaseStgMedium(&mut self.0); } } }
fn com_error(stage: BackupStage, id: Option<u32>, error: Error) -> BackupFailure {
    BackupFailure { stage, format_id: id, win32_error: None, hresult: Some(error.code().0) }
}
pub(super) fn unchanged(sequence: u32, owner: Handle) -> bool {
    unsafe { GetClipboardSequenceNumber() == sequence && GetClipboardOwner() == owner }
}
fn medium_for(id: u32) -> u32 { if matches!(id, 2 | 9 | 0x82) { 16 } else { 1 } }
// Every declared representation must have a lossless static Win32 equivalent.
// Indexed/virtual files, streams, storage and target-specific objects fail before input.
fn static_format(format: &FORMATETC) -> bool {
    format.ptd.is_null() && format.dwAspect == 1 && format.lindex == -1
        && format.cfFormat != 0 && format.tymed & medium_for(u32::from(format.cfFormat)) != 0
}
pub(super) fn snapshot(sequence: u32, owner: Handle, bytes: &mut usize) -> Result<Vec<Format>, BackupFailure> { unsafe {
    OleInitialize(None).map_err(|error| com_error(BackupStage::OleInitialize, None, error))?;
    let _apartment = Apartment;
    // COM pumps synchronous cross-apartment calls while materializing. All COM
    // references/mediums are released before OleUninitialize and blocking stdin.
    let object = OleGetClipboard().map_err(|error| com_error(BackupStage::OleOpen, None, error))?;
    materialize(&object, bytes, || unchanged(sequence, owner))
} }
fn materialize(object: &IDataObject, bytes: &mut usize, current: impl Fn() -> bool) -> Result<Vec<Format>, BackupFailure> { unsafe {
    if !current() { return Err(failure(BackupStage::SourceChanged, None, false)); }
    let enumerator = object.EnumFormatEtc(1).map_err(|error| com_error(BackupStage::OleEnumerate, None, error))?;
    let mut saved: Vec<Format> = Vec::new();
    loop {
        let mut format = FORMATETC::default(); let mut fetched = 0;
        let status = enumerator.Next(std::slice::from_mut(&mut format), Some(&mut fetched));
        let plain = static_format(&format);
        if !format.ptd.is_null() { CoTaskMemFree(Some(format.ptd as _)); format.ptd = ptr::null_mut(); }
        if status.is_err() { return Err(com_error(BackupStage::OleEnumerate, None, Error::from_hresult(status))); }
        if fetched == 0 {
            if status.0 != 1 { return Err(failure(BackupStage::OleEnumerate, None, false)); }
            break;
        }
        let id = u32::from(format.cfFormat);
        // Never merge distinct FORMATETC payloads with one format identifier.
        if fetched != 1 || !plain || ole_transport(id) || saved.iter().any(|item| item.id == id) {
            return Err(failure(BackupStage::UnsupportedFormat, Some(id), false));
        }
        if saved.len() >= MAX_FORMATS { return Err(failure(BackupStage::FormatLimit, None, false)); }
        if !current() { return Err(failure(BackupStage::SourceChanged, None, false)); }
        format.tymed = medium_for(id);
        let medium = Medium(object.GetData(&format).map_err(|error| com_error(BackupStage::OleRead, Some(id), error))?);
        if medium.0.tymed != format.tymed { return Err(failure(BackupStage::UnsupportedFormat, Some(id), false)); }
        let handle = if format.tymed == 16 { medium.0.u.hBitmap.0 as isize } else { medium.0.u.hGlobal.0 as isize };
        saved.push(snapshot_format(id, handle, bytes)?);
        if !current() { return Err(failure(BackupStage::SourceChanged, None, false)); }
    }
    if saved.is_empty() { return Err(failure(BackupStage::OleEnumerate, None, false)); }
    if !current() { return Err(failure(BackupStage::SourceChanged, None, false)); }
    Ok(saved)
} }

#[cfg(test)]
#[path = "windows_ole_tests.rs"]
mod tests;
