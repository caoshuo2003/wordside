//! Materialize OLE payloads before copying. No old IDataObject survives this call.
//! Restore uses Win32 static formats under the existing sequence/owner lock;
//! Windows supplies a fresh default OLE wrapper rather than stale provider handles.
//! Streams, storage and metafiles are taken from OLE's own Win32 rendering of them.
use super::*;
use ::windows::{
    core::Error,
    Win32::System::{
        Com::{CoTaskMemFree, IDataObject, FORMATETC, STGMEDIUM},
        Ole::{OleGetClipboard, OleInitialize, OleUninitialize, ReleaseStgMedium},
    },
};
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            OleUninitialize();
        }
    }
}
struct Medium(STGMEDIUM);
impl Drop for Medium {
    fn drop(&mut self) {
        unsafe {
            ReleaseStgMedium(&mut self.0);
        }
    }
}
fn com_error(stage: BackupStage, id: Option<u32>, error: Error) -> BackupFailure {
    BackupFailure {
        stage,
        format_id: id,
        win32_error: None,
        hresult: Some(error.code().0),
    }
}
pub(super) fn unchanged(sequence: u32, owner: Handle) -> bool {
    unsafe { GetClipboardSequenceNumber() == sequence && GetClipboardOwner() == owner }
}
fn medium_for(id: u32) -> u32 {
    if matches!(id, 2 | 9 | 0x82) {
        16
    } else {
        1
    }
}
// Streams (4), storage (8), metafile pictures (32) and enhanced metafiles (64) have
// no handle to ask OLE for. OLE itself renders every static format onto the Win32
// clipboard, storage as a compound file in global memory: the same copy that stays
// when the source application exits. Those are backed up from there, and on restore
// the default OLE wrapper offers the compound file as storage again. FileContents is
// one stream per file under a single name and its Win32 rendering holds only the
// first, so it never qualifies.
const RENDERED_MEDIA: u32 = 4 | 8 | 32 | 64;
fn rendered_natively(format: &FORMATETC) -> bool {
    let id = u32::from(format.cfFormat);
    (matches!(id, 3 | 14)
        || format.tymed & medium_for(id) == 0 && format.tymed & RENDERED_MEDIA != 0)
        && !file_contents_view(id)
}
// Every declared representation must have a lossless static Win32 equivalent.
// Indexed/virtual files and target-specific objects fail before input.
fn static_format(format: &FORMATETC) -> bool {
    format.ptd.is_null()
        && format.dwAspect == 1
        && format.lindex == -1
        && format.cfFormat != 0
        && (format.tymed & medium_for(u32::from(format.cfFormat)) != 0 || rendered_natively(format))
}
// Some sources declare a format with nothing behind it (.NET WinForms with a metafile
// it cannot draw; Windows' EnterpriseDataProtectionId marker): OLE reports no such
// data and the Win32 clipboard returns none, so no application can paste it. Only
// these answers make a declared format count as empty.
fn undeliverable(error: &Error) -> bool {
    matches!(
        error.code().0 as u32,
        // DV_E_FORMATETC, DV_E_TYMED, DV_E_CLIPFORMAT, and CLIPBRD_E_BAD_DATA: the
        // Win32 rendering came back empty.
        0x8004_0064 | 0x8004_0069 | 0x8004_006a | 0x8004_01d3
    )
}
pub(super) struct Snapshot {
    /// Payloads read through OLE.
    pub saved: Vec<Format>,
    /// Formats OLE has rendered onto the Win32 clipboard, where they must be read.
    pub rendered: Vec<u32>,
    /// Declared formats that hold nothing (see `undeliverable`). The Win32 pass needs
    /// only what OLE did deliver, so this is for tests and diagnosis.
    #[allow(dead_code)]
    pub empty: Vec<u32>,
}
pub(super) fn snapshot(
    sequence: u32,
    owner: Handle,
    paths: bool,
    bytes: &mut usize,
) -> Result<Snapshot, BackupFailure> {
    unsafe {
        OleInitialize(None).map_err(|error| com_error(BackupStage::OleInitialize, None, error))?;
        let _apartment = Apartment;
        // COM pumps synchronous cross-apartment calls while materializing. All COM
        // references/mediums are released before OleUninitialize and blocking stdin.
        let object =
            OleGetClipboard().map_err(|error| com_error(BackupStage::OleOpen, None, error))?;
        materialize(&object, paths, bytes, || unchanged(sequence, owner))
    }
}
fn materialize(
    object: &IDataObject,
    paths: bool,
    bytes: &mut usize,
    current: impl Fn() -> bool,
) -> Result<Snapshot, BackupFailure> {
    unsafe {
        if !current() {
            return Err(failure(BackupStage::SourceChanged, None, false));
        }
        let enumerator = object
            .EnumFormatEtc(1)
            .map_err(|error| com_error(BackupStage::OleEnumerate, None, error))?;
        let mut saved: Vec<Format> = Vec::new();
        let mut rendered: Vec<u32> = Vec::new();
        let mut empty: Vec<u32> = Vec::new();
        loop {
            let mut format = FORMATETC::default();
            let mut fetched = 0;
            let status = enumerator.Next(std::slice::from_mut(&mut format), Some(&mut fetched));
            let plain = static_format(&format);
            if !format.ptd.is_null() {
                CoTaskMemFree(Some(format.ptd as _));
                format.ptd = ptr::null_mut();
            }
            if status.is_err() {
                return Err(com_error(
                    BackupStage::OleEnumerate,
                    None,
                    Error::from_hresult(status),
                ));
            }
            if fetched == 0 {
                if status.0 != 1 {
                    return Err(failure(BackupStage::OleEnumerate, None, false));
                }
                break;
            }
            let id = u32::from(format.cfFormat);
            // Copied files on disk keep their paths; see file_contents_view.
            if paths && file_contents_view(id) {
                continue;
            }
            // Never merge distinct FORMATETC payloads with one format identifier.
            if fetched != 1
                || !plain
                || ole_transport(id)
                || saved.iter().any(|item| item.id == id)
                || rendered.contains(&id)
                || empty.contains(&id)
            {
                return Err(failure(BackupStage::UnsupportedFormat, Some(id), false));
            }
            if saved.len() + rendered.len() >= MAX_FORMATS {
                return Err(failure(BackupStage::FormatLimit, None, false));
            }
            if !current() {
                return Err(failure(BackupStage::SourceChanged, None, false));
            }
            if rendered_natively(&format) {
                // Metafiles are small and the only kind seen declared empty; ask once.
                if matches!(id, 3 | 14) {
                    format.tymed = if id == 3 { 32 } else { 64 };
                    match object.GetData(&format) {
                        Ok(medium) => drop(Medium(medium)),
                        Err(error) if undeliverable(&error) => {
                            empty.push(id);
                            continue;
                        }
                        Err(error) => return Err(com_error(BackupStage::OleRead, Some(id), error)),
                    }
                }
                rendered.push(id);
                continue;
            }
            format.tymed = medium_for(id);
            let medium = match object.GetData(&format) {
                Ok(medium) => Medium(medium),
                // Declared with nothing behind it, like the EnterpriseDataProtectionId
                // marker modern Windows apps add: no application can paste it either.
                Err(error) if undeliverable(&error) => {
                    empty.push(id);
                    continue;
                }
                Err(error) => return Err(com_error(BackupStage::OleRead, Some(id), error)),
            };
            if medium.0.tymed != format.tymed {
                return Err(failure(BackupStage::UnsupportedFormat, Some(id), false));
            }
            let handle = if format.tymed == 16 {
                medium.0.u.hBitmap.0 as isize
            } else {
                medium.0.u.hGlobal.0 as isize
            };
            saved.push(snapshot_format(id, handle, bytes)?);
            if !current() {
                return Err(failure(BackupStage::SourceChanged, None, false));
            }
        }
        if saved.is_empty() && rendered.is_empty() && empty.is_empty() {
            return Err(failure(BackupStage::OleEnumerate, None, false));
        }
        if !current() {
            return Err(failure(BackupStage::SourceChanged, None, false));
        }
        Ok(Snapshot {
            saved,
            rendered,
            empty,
        })
    }
}

#[cfg(test)]
#[path = "windows_ole_tests.rs"]
mod tests;
