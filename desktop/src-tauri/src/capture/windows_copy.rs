//! Generic, source-bound copy verification. Separate helpers hold the full backup
//! and read copied text. A hung delayed-rendering backup is killed before any input;
//! a hung text reader is killed while the backup helper remains available to restore.
//! This module is never invoked by the pure test suite against the user's clipboard.
use super::{copy_policy::{CopiedText, Receipt, INPUT_MARKER}, policy::selected_word,
    selection::Selection, windows_text::{focus_matches, TextRequest}};
use std::{cell::Cell,ffi::c_void, ptr, thread, time::{Duration, Instant}};
type Handle = isize;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_FORMATS: usize = 128;
#[link(name = "user32")]
extern "system" {
    fn OpenClipboard(window: Handle) -> i32;
    fn CloseClipboard() -> i32;
    fn EnumClipboardFormats(format: u32) -> u32;
    fn GetClipboardData(format: u32) -> Handle;
    fn SetClipboardData(format: u32, data: Handle) -> Handle;
    fn EmptyClipboard() -> i32;
    fn GetClipboardSequenceNumber() -> u32;
    fn GetClipboardOwner() -> Handle;
    fn GetClipboardFormatNameW(format: u32, name: *mut u16, max: i32) -> i32;
    fn GetForegroundWindow() -> Handle;
    fn GetAncestor(window: Handle, flags: u32) -> Handle;
    fn GetWindowThreadProcessId(window: Handle, pid: *mut u32) -> u32;
    fn GetAsyncKeyState(key: i32) -> i16;
    fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
    fn CreateWindowExW(ex: u32, class: *const u16, name: *const u16, style: u32,
        x: i32, y: i32, width: i32, height: i32, parent: Handle, menu: Handle,
        instance: Handle, param: *const c_void) -> Handle;
    fn DestroyWindow(window: Handle) -> i32;
}
#[link(name = "kernel32")]
extern "system" {
    fn GlobalSize(memory: Handle) -> usize;
    fn GlobalLock(memory: Handle) -> *mut c_void;
    fn GlobalUnlock(memory: Handle) -> i32;
    fn GlobalFree(memory: Handle) -> Handle;
    fn SetLastError(code: u32);
    fn GetLastError() -> u32;
}
#[link(name = "ole32")]
extern "system" { fn OleDuplicateData(data: Handle, format: u16, flags: u32) -> Handle; }
#[link(name = "gdi32")]
extern "system" {
    fn DeleteObject(object: Handle) -> i32;
    fn DeleteMetaFile(object: Handle) -> i32;
    fn DeleteEnhMetaFile(object: Handle) -> i32;
    fn GetObjectW(object: Handle, size: i32, buffer: *mut c_void) -> i32;
    fn GetBitmapBits(bitmap: Handle, size: i32, bits: *mut c_void) -> i32;
    fn GetPaletteEntries(palette: Handle, start: u32, count: u32, entries: *mut c_void) -> u32;
}
#[derive(Clone, Copy)]
#[repr(C)]
struct KeyInput { key: u16, scan: u16, flags: u32, time: u32, extra: usize }
#[derive(Clone, Copy)]
#[repr(C)]
struct MouseInput { x: i32, y: i32, data: u32, flags: u32, time: u32, extra: usize }
#[repr(C)]
union InputData { keyboard: KeyInput, mouse: MouseInput }
#[repr(C)]
struct Input { kind: u32, data: InputData }
#[repr(C)]
struct Bitmap { kind: i32, width: i32, height: i32, stride: i32, planes: u16, bits: u16, data: *mut c_void }
#[repr(C)]
struct MetaFilePicture { mode: i32, x: i32, y: i32, metafile: Handle }
fn key(key: u16, up: bool, extended: bool) -> Input {
    Input { kind: 1, data: InputData { keyboard: KeyInput { key, scan: 0,
        flags: u32::from(up) * 2 | u32::from(extended), time: 0, extra: INPUT_MARKER } } }
}
fn root(window: Handle) -> Handle { let value = unsafe { GetAncestor(window, 2) }; if value == 0 { window } else { value } }
fn pid(window: Handle) -> u32 { let mut pid = 0; unsafe { GetWindowThreadProcessId(window, &mut pid); } pid }
#[derive(Clone,Copy,Debug,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
#[serde(rename_all="snake_case")]
pub(super) enum SourceRejection { InvalidSource, ForegroundChanged, ProcessChanged, FocusChanged, InputOrRequestChanged }
fn source_rejection(request: &TextRequest) -> Option<SourceRejection> {
    if request.pid == 0 { Some(SourceRejection::InvalidSource) }
    else if root(unsafe { GetForegroundWindow() }) != root(request.window) { Some(SourceRejection::ForegroundChanged) }
    else if pid(request.window) != request.pid { Some(SourceRejection::ProcessChanged) }
    else if !focus_matches(request.window, request.focus) { Some(SourceRejection::FocusChanged) }
    else { None }
}
fn source_current(request: &TextRequest) -> bool { source_rejection(request).is_none() }
fn source_owns(request: &TextRequest) -> bool {
    let owner = unsafe { GetClipboardOwner() };
    owner != 0 && (pid(owner) == request.pid || root(owner) == root(request.window))
}
struct ClipboardLock;
impl ClipboardLock {
    fn open(window: Handle, budget: Duration) -> Option<Self> {
        let end = Instant::now() + budget;
        loop {
            if unsafe { OpenClipboard(window) } != 0 { return Some(Self); }
            if Instant::now() >= end { return None; }
            thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for ClipboardLock { fn drop(&mut self) { unsafe { CloseClipboard(); } } }
struct Owner(Handle);
impl Owner {
    fn new() -> Option<Self> {
        let class: Vec<_> = "STATIC\0".encode_utf16().collect();
        // Message-only, never visible or activated. A non-null owner is required
        // by EmptyClipboard/SetClipboardData; OpenClipboard(NULL) cannot restore.
        let window = unsafe { CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0,
            0, 0, 0, 0, -3, 0, 0, ptr::null()) };
        (window != 0).then_some(Self(window))
    }
}
impl Drop for Owner { fn drop(&mut self) { unsafe { DestroyWindow(self.0); } } }
struct Format { id: u32, data: Handle, bytes: usize }
impl Drop for Format {
    fn drop(&mut self) { unsafe {
        if self.data == 0 { return; }
        match self.id {
            2 | 9 | 0x82 => { DeleteObject(self.data); }
            14 | 0x8e => { DeleteEnhMetaFile(self.data); }
            3 | 0x83 => {
                let picture = GlobalLock(self.data) as *const MetaFilePicture;
                if !picture.is_null() { DeleteMetaFile((*picture).metafile); GlobalUnlock(self.data); }
                GlobalFree(self.data);
            }
            _ => { GlobalFree(self.data); }
        }
    } }
}
// Internal diagnostics contain API stages and format identifiers only. They are
// returned over the private helper pipe, never selection text or clipboard data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BackupStage { CreateOwner, OpenClipboard, Enumerate, FormatLimit, ReadFormat, UnsupportedFormat, InspectFormat, SizeLimit, DuplicateFormat, OleInitialize, OleOpen, OleEnumerate, OleRead, SourceChanged, PayloadMismatch }
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct BackupFailure {
    pub stage: BackupStage,
    #[serde(skip_serializing_if = "Option::is_none")] pub format_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub win32_error: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub hresult: Option<i32>,
}
fn failure(stage: BackupStage, format_id: Option<u32>, native: bool) -> BackupFailure {
    let error = if native { unsafe { GetLastError() } } else { 0 };
    BackupFailure { stage, format_id, win32_error: (error != 0).then_some(error), hresult: None }
}
fn format_name(id: u32) -> String {
    let mut name = [0u16; 256];
    let len = unsafe { GetClipboardFormatNameW(id, name.as_mut_ptr(), name.len() as i32) };
    String::from_utf16_lossy(&name[..len.max(0) as usize]).to_ascii_lowercase()
}
fn ole_transport(id: u32) -> bool {
    id >= 0xc000 && matches!(format_name(id).as_str(), "dataobject" | "ole private data")
}
fn enumerate_formats() -> Result<Vec<u32>, BackupFailure> {
    let mut ids = Vec::new(); let mut id = 0;
    loop {
        unsafe { SetLastError(0); id = EnumClipboardFormats(id); }
        if id == 0 {
            if unsafe { GetLastError() } != 0 { return Err(failure(BackupStage::Enumerate, None, true)); }
            return Ok(ids);
        }
        if ids.len() >= MAX_FORMATS { return Err(failure(BackupStage::FormatLimit, None, false)); }
        ids.push(id);
    }
}
#[path = "windows_ole.rs"]
mod ole;
fn snapshot_format(id: u32, source: Handle, bytes: &mut usize) -> Result<Format, BackupFailure> { unsafe {
    let previous_bytes=*bytes;
    let unsupported = || failure(BackupStage::UnsupportedFormat, Some(id), false);
    let inspect = || failure(BackupStage::InspectFormat, Some(id), true);
    let limit = || failure(BackupStage::SizeLimit, Some(id), false);
    // Owner-display/private GDI formats require the original application's
    // lifecycle. OLE virtual objects cannot be preserved by copying raw handles.
    if id == 0x80 || (0x200..=0x3ff).contains(&id) { return Err(unsupported()); }
    if source == 0 { return Err(failure(BackupStage::ReadFormat, Some(id), true)); }
    if id >= 0xc000 {
        let name = format_name(id);
        if matches!(name.as_str(), "dataobject" | "ole private data" | "filecontents") { return Err(unsupported()); }
    }
    if matches!(id, 2 | 0x82) {
        let mut bitmap: Bitmap = std::mem::zeroed();
        SetLastError(0);
        if GetObjectW(source, std::mem::size_of::<Bitmap>() as i32, &mut bitmap as *mut _ as _) == 0 { return Err(inspect()); }
        let size = usize::try_from(bitmap.stride).ok().and_then(|stride| bitmap.height.checked_abs()
            .and_then(|height| usize::try_from(height).ok()).and_then(|height| stride.checked_mul(height))).ok_or_else(limit)?;
        *bytes = bytes.checked_add(size).ok_or_else(limit)?;
        if *bytes > MAX_BYTES { return Err(limit()); }
    } else if id == 9 {
        // CF_PALETTE is also synthesized for ordinary DIB clipboard images.
        SetLastError(0);
        let entries = GetPaletteEntries(source, 0, 0, ptr::null_mut());
        if entries == 0 { return Err(inspect()); }
        let size = usize::try_from(entries).ok().and_then(|count| count.checked_mul(4)).and_then(|size| size.checked_add(4)).ok_or_else(limit)?;
        *bytes = bytes.checked_add(size).ok_or_else(limit)?;
        if *bytes > MAX_BYTES { return Err(limit()); }
    } else if matches!(id, 3 | 14 | 0x83 | 0x8e) {
        // These uncommon GDI formats cannot currently be budgeted safely.
        return Err(unsupported());
    } else {
        SetLastError(0);
        let size = GlobalSize(source);
        if size == 0 { return Err(inspect()); }
        *bytes = bytes.checked_add(size).ok_or_else(limit)?;
        if *bytes > MAX_BYTES { return Err(limit()); }
    }
    SetLastError(0);
    let data = match id {
        0x82 => OleDuplicateData(source, 2, 2),
        _ => OleDuplicateData(source, id as u16, 2),
    };
    if data == 0 { return Err(failure(BackupStage::DuplicateFormat, Some(id), true)); }
    Ok(Format { id, data, bytes:*bytes-previous_bytes })
} }
// An OLE provider and an already-rendered Win32 format can disagree without a
// sequence change. Never choose one silently: both views must have equal data.
fn verify_payload(saved: &Format, source: Handle) -> Result<(), BackupFailure> { unsafe {
    let mismatch = || failure(BackupStage::PayloadMismatch, Some(saved.id), false);
    if source == 0 { return Err(failure(BackupStage::ReadFormat, Some(saved.id), true)); }
    let same = if saved.id == 9 {
        let count = GetPaletteEntries(saved.data, 0, 0, ptr::null_mut());
        if count == 0 || GetPaletteEntries(source, 0, 0, ptr::null_mut()) != count { return Err(mismatch()); }
        let size = (count as usize).checked_mul(4).filter(|size| *size <= MAX_BYTES).ok_or_else(mismatch)?;
        let mut left = vec![0u8; size]; let mut right = vec![0u8; size];
        GetPaletteEntries(saved.data, 0, count, left.as_mut_ptr() as _) == count
            && GetPaletteEntries(source, 0, count, right.as_mut_ptr() as _) == count && left == right
    } else if matches!(saved.id, 2 | 0x82) {
        let mut left: Bitmap = std::mem::zeroed(); let mut right: Bitmap = std::mem::zeroed();
        let size = std::mem::size_of::<Bitmap>() as i32;
        if GetObjectW(saved.data, size, &mut left as *mut _ as _) == 0 || GetObjectW(source, size, &mut right as *mut _ as _) == 0 { return Err(mismatch()); }
        if (left.width,left.height,left.stride,left.planes,left.bits) != (right.width,right.height,right.stride,right.planes,right.bits) { return Err(mismatch()); }
        let size = usize::try_from(left.stride).ok().and_then(|stride| left.height.checked_abs()
            .and_then(|height| usize::try_from(height).ok()).and_then(|height| stride.checked_mul(height)))
            .filter(|size| *size > 0 && *size <= MAX_BYTES).ok_or_else(mismatch)?;
        let mut left = vec![0u8; size]; let mut right = vec![0u8; size];
        GetBitmapBits(saved.data, size as i32, left.as_mut_ptr() as _) == size as i32
            && GetBitmapBits(source, size as i32, right.as_mut_ptr() as _) == size as i32 && left == right
    } else {
        let left_size=GlobalSize(saved.data); let right_size=GlobalSize(source);
        if left_size==0 || right_size==0 || left_size>MAX_BYTES || right_size>MAX_BYTES { return Err(mismatch()); }
        let left=GlobalLock(saved.data) as *const u8;
        if left.is_null() { return Err(mismatch()); }
        let right=GlobalLock(source) as *const u8;
        if right.is_null() { GlobalUnlock(saved.data); return Err(mismatch()); }
        let same=payload_equal(saved.id,std::slice::from_raw_parts(left,left_size),std::slice::from_raw_parts(right,right_size));
        GlobalUnlock(source); GlobalUnlock(saved.data); same
    };
    if same { Ok(()) } else { Err(mismatch()) }
} }
fn payload_equal(id:u32, left:&[u8], right:&[u8])->bool {
    fn text(data:&[u8], wide:bool)->Option<&[u8]> {
        let end=if wide { data.chunks_exact(2).position(|pair|pair==[0,0]).map(|i|i*2+2) }
            else { data.iter().position(|byte|*byte==0).map(|i|i+1) }?;
        Some(&data[..end])
    }
    match id {
        1|7|13=>match (text(left,id==13),text(right,id==13)) { (Some(left),Some(right))=>left==right, _=>false },
        _=>left==right,
    }
}
fn native_payload(saved:Format, source:Handle, bytes:&mut usize)->Result<Format,BackupFailure> {
    // A provider may expose a complete static representation only through OLE.
    // Keep that owned copy when the native API has no handle and no explicit
    // access error. The enclosing snapshot still checks sequence and owner.
    // No format, including enterprise metadata, is omitted or replaced by NULL.
    if source==0 && unsafe{GetLastError()}==0 {return Ok(saved);}
    verify_payload(&saved,source)?;
    let id=saved.id; *bytes=bytes.saturating_sub(saved.bytes); drop(saved);
    // Keep the original native allocation, including application-owned padding.
    snapshot_format(id,source,bytes)
}
pub(super) struct Backup { pub before: u32, pub ole_formats: usize, formats: Vec<Format>, owner: Owner }
fn coherent_snapshot<T>(result:Result<T,BackupFailure>, unchanged:bool)->Result<T,BackupFailure> {
    if unchanged {result} else {Err(failure(BackupStage::SourceChanged,None,false))}
}
fn needs_ole_retry(error:&BackupFailure)->bool {
    error.stage==BackupStage::ReadFormat && error.format_id.is_some_and(|id|id>=0xc000)
        && error.win32_error.is_none() && error.hresult.is_none()
}
impl Backup {
    pub fn take() -> Result<Self, BackupFailure> {
        super::timing::stable_backup(||{
            match Self::take_once(false) {
                Err(error) if needs_ole_retry(&error)=>Self::take_once(true),
                result=>result,
            }
        }, |failure|matches!(failure.stage,BackupStage::SourceChanged|BackupStage::OpenClipboard))
    }
    fn take_once(force_ole:bool) -> Result<Self, BackupFailure> {
        unsafe { SetLastError(0); }
        let owner = Owner::new().ok_or_else(|| failure(BackupStage::CreateOwner, None, true))?;
        let mut lock = ClipboardLock::open(owner.0, Duration::from_millis(80))
            .ok_or_else(|| failure(BackupStage::OpenClipboard, None, true))?;
        let before=unsafe{GetClipboardSequenceNumber()};
        let original_owner=unsafe{GetClipboardOwner()};
        let result=(||{
            let mut ids = enumerate_formats()?;
            let mut bytes = 0;
            let mut ole_formats = 0;
            let mut formats = if force_ole || ids.iter().copied().any(ole_transport) {
                drop(lock); // OLE must open the clipboard itself, never under our lock.
                let saved = ole::snapshot(before, original_owner, &mut bytes)?;
                lock = ClipboardLock::open(owner.0, Duration::from_millis(80))
                    .ok_or_else(|| failure(BackupStage::OpenClipboard, None, true))?;
                if !ole::unchanged(before, original_owner) { return Err(failure(BackupStage::SourceChanged, None, false)); }
                ids = enumerate_formats()?;
                ole_formats = saved.len();
                saved
            } else { Vec::new() };
            let mut ordered = Vec::new();
            for id in ids {
                // OLE transport is replaced only after every declared static
                // payload is materialized. All actual formats remain present.
                if ole_transport(id) { continue; }
                unsafe { SetLastError(0); }
                let source = unsafe { GetClipboardData(id) };
                let value = if let Some(index) = formats.iter().position(|format| format.id == id) {
                    native_payload(formats.remove(index),source,&mut bytes)?
                } else {
                    if ordered.len() + formats.len() >= MAX_FORMATS { return Err(failure(BackupStage::FormatLimit, None, false)); }
                    snapshot_format(id, source, &mut bytes)?
                };
                ordered.push(value);
            }
            // Keep original Win32 format priority; append only OLE-only payloads.
            ordered.append(&mut formats); let formats = ordered;
            drop(lock);
            Ok(Self { before, ole_formats, formats, owner })
        })();
        // Native delayed rendering may advance the sequence without OLE markers.
        // A changed snapshot is retried in full, including failures on stale IDs.
        coherent_snapshot(result,ole::unchanged(before,original_owner))
    }
    pub fn restore(&mut self, expected: Option<u32>, request: &TextRequest) -> Result<RestoreState, ()> {
        let Some(sequence) = expected else { return Ok(RestoreState::Skipped); };
        let mut receipt = Receipt::new(self.before);
        receipt.observe(sequence, true, true);
        // New data from any application, including another copy in the same
        // process, always wins over our backup. Recheck under the clipboard lock.
        if !receipt.restore_at(unsafe { GetClipboardSequenceNumber() }, source_owns(request)) { return Ok(RestoreState::Skipped); }
        let _lock = ClipboardLock::open(self.owner.0, Duration::from_millis(500)).ok_or(())?;
        if !receipt.restore_at(unsafe { GetClipboardSequenceNumber() }, source_owns(request)) { return Ok(RestoreState::Skipped); }
        if unsafe { EmptyClipboard() } == 0 { return Err(()); }
        let end = Instant::now() + Duration::from_millis(100);
        for format in &mut self.formats {
            while unsafe { SetClipboardData(format.id, format.data) } == 0 {
                if Instant::now() >= end { return Err(()); }
                thread::sleep(Duration::from_millis(2));
            }
            format.data = 0; // Windows owns this handle now.
        }
        Ok(RestoreState::Restored)
    }
}
#[derive(Clone,Copy,Debug,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
#[serde(rename_all="snake_case")]
pub(super) enum RestoreState { Restored, Skipped }
fn released() -> bool {
    // Never release or change a user's held key. In particular Alt+D must be
    // completely released before a copy chord can be sent.
    (1..256).all(|key| unsafe { GetAsyncKeyState(key) >= 0 })
}
// RegisterHotKey consumes the letter, so releasing Alt before that letter
// can look like a bare Alt press to the source application. Use a modifier-only
// pair: an unassigned non-modifier (such as E8) still clears terminal selections.
// Only the hook thread sends/repairs this marked left Ctrl. Never release a
// physical Ctrl or send Escape to dismiss a source application's menu.
static MENU_KEY_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn recover_menu_key(pending:&std::sync::atomic::AtomicBool,physical_left_up:bool,send_up:impl FnOnce()->u32)->bool {
    use std::sync::atomic::Ordering;
    if !pending.load(Ordering::SeqCst) {return true;}
    if !physical_left_up || send_up()!=1 {return false;}
    pending.store(false,Ordering::SeqCst);true
}
fn complete_menu_pair(sent:u32,pending:&std::sync::atomic::AtomicBool,physical_left_up:bool,send_up:impl FnOnce()->u32)->bool {
    if sent==1 {
        pending.store(true,std::sync::atomic::Ordering::SeqCst);
        recover_menu_key(pending,physical_left_up,send_up);
    }
    sent==2
}
fn send_menu_up()->u32 {
    let up=key(0xA2,true,false);
    unsafe {SendInput(1,&up,std::mem::size_of::<Input>() as i32)}
}
fn physical_menu_key_released()->bool {
    matches!(super::input::physical_controls_released(),Some((true,_)))
}
pub(super) fn recover_menu_mask()->bool {
    recover_menu_key(&MENU_KEY_PENDING,physical_menu_key_released(),send_menu_up)
}
pub(super) fn mask_alt_menu(request: &TextRequest)->bool {
    if !recover_menu_mask() || !source_current(request) || unsafe { GetAsyncKeyState(0x12) } >= 0 {return false;}
    // The physical hook excludes our tagged events. Check both Ctrl sides and
    // the OS sample before introducing any new modifier pair.
    if super::input::physical_controls_released()!=Some((true,true))
        || [0x11,0xA2,0xA3].into_iter().any(|code|unsafe{GetAsyncKeyState(code)}<0) {return false;}
    let inputs=[key(0xA2,false,false),key(0xA2,true,false)];
    let sent=unsafe {SendInput(inputs.len() as u32,inputs.as_ptr(),std::mem::size_of::<Input>() as i32)};
    // A partial send retains its own up for retry, but must wait if the user
    // physically pressed left Ctrl in the meantime. Synthetic state alone
    // cannot make that decision: our own partial down also sets the OS bit.
    complete_menu_pair(sent,&MENU_KEY_PENDING,physical_menu_key_released(),send_menu_up)
}
fn send_copy() -> bool {
    // Ctrl+Insert is the standard Windows copy chord and never sends Ctrl+C
    // (SIGINT) into a terminal that has no selection. No process-name allowlist.
    let inputs = [key(0x11, false, false), key(0x2d, false, true),
        key(0x2d, true, true), key(0x11, true, false)];
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<Input>() as i32) };
    if sent != inputs.len() as u32 && sent > 0 {
        // A partial insertion must not leave our synthetic Ctrl/Insert held.
        let release = [key(0x2d, true, true), key(0x11, true, false)];
        unsafe { SendInput(release.len() as u32, release.as_ptr(), std::mem::size_of::<Input>() as i32); }
    }
    sent == inputs.len() as u32
}
#[derive(Clone,Copy,Default,serde::Serialize)]
pub(super) struct CopyDiagnostics {
    input_sent: Option<bool>,
    sequence_changed: bool,
    busy_snapshots: usize,
    source_owner: Option<bool>,
    text_read: bool,
    read_outcome: Option<&'static str>,
    restore_completed: Option<bool>,
    request_valid: bool,
    #[serde(skip_serializing_if="Option::is_none")]
    invalid_reason: Option<SourceRejection>,
}
impl CopyDiagnostics {
    // Even a partial SendInput can have started a copy. Only None proves no
    // copy was attempted, including when the final restore was safely skipped.
    pub(super) fn input_attempted(&self) -> bool { self.input_sent.is_some() }
}
pub(super) struct VerifiedCopy {pub selection:Option<Selection>,pub diagnostics:CopyDiagnostics}
struct NativeBackend<'a, V, L, R, C> {
    request: &'a TextRequest, valid: V, release: L, read: R, restore: C, started: Instant, before:u32, diagnostics:Cell<CopyDiagnostics>,
}
impl<V: Fn()->bool, L: Fn()->Option<u64>, R: FnMut(u32)->Option<CopiedText>, C: FnMut(Option<u32>)->Option<RestoreState>>
    super::copy_policy::Backend for NativeBackend<'_,V,L,R,C> {
    fn valid(&self)->bool{
        let reason=if !(self.valid)() {Some(SourceRejection::InputOrRequestChanged)} else {source_rejection(self.request)};
        let valid=reason.is_none();
        let mut diagnostics=self.diagnostics.get();diagnostics.request_valid&=valid;
        if diagnostics.invalid_reason.is_none() {diagnostics.invalid_reason=reason;}
        self.diagnostics.set(diagnostics);valid
    }
    fn release_stamp(&self)->Option<u64>{released().then(||(self.release)()).flatten()}
    fn elapsed(&self)->Duration{self.started.elapsed()}
    fn pause(&mut self,duration:Duration){thread::sleep(duration);}
    fn snapshot(&mut self)->Option<super::copy_policy::ClipboardState>{
        // Never attribute a sequence exposed by a writer before it closes the
        // clipboard. Do not read/materialize data or hold this lock across IPC,
        // SendInput, waits or restoration.
        let mut diagnostics=self.diagnostics.get();
        let Some(_lock)=ClipboardLock::open(0,Duration::ZERO) else {
            diagnostics.busy_snapshots=diagnostics.busy_snapshots.saturating_add(1);
            self.diagnostics.set(diagnostics);return None;
        };
        let sequence=unsafe{GetClipboardSequenceNumber()};let owns=source_owns(self.request);
        diagnostics.sequence_changed|=sequence!=self.before;diagnostics.source_owner=Some(owns);
        self.diagnostics.set(diagnostics);
        Some(super::copy_policy::ClipboardState{sequence,source_owns:owns})
    }
    fn send_copy(&mut self)->bool{
        let sent=send_copy();let mut diagnostics=self.diagnostics.get();diagnostics.input_sent=Some(sent);self.diagnostics.set(diagnostics);sent
    }
    fn read(&mut self,sequence:u32)->Option<CopiedText>{
        let result=(self.read)(sequence);let mut diagnostics=self.diagnostics.get();diagnostics.text_read=true;
        diagnostics.read_outcome=Some(super::diagnostics::code(result.as_ref().map(|reply|&reply.selection)));self.diagnostics.set(diagnostics);result
    }
    fn restore(&mut self,sequence:Option<u32>)->bool{
        let state=(self.restore)(sequence);let mut diagnostics=self.diagnostics.get();
        diagnostics.restore_completed=state.map(|state|state==RestoreState::Restored);self.diagnostics.set(diagnostics);state.is_some()
    }
}
pub fn verify(request: &TextRequest, before: u32, valid: impl Fn()->bool, release: impl Fn()->Option<u64>,
    read: impl FnMut(u32)->Option<CopiedText>, restore: impl FnMut(Option<u32>)->Option<RestoreState>) -> VerifiedCopy {
    let mut backend=NativeBackend{request,valid,release,read,restore,started:Instant::now(),before,
        diagnostics:Cell::new(CopyDiagnostics{request_valid:true,..Default::default()})};
    let selection=super::copy_policy::transact(&mut backend,before);
    VerifiedCopy{selection,diagnostics:backend.diagnostics.into_inner()}
}

// Fixed metadata only. No text, clipboard payload, process or window identifiers.
#[derive(Clone,Copy,Debug,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
#[serde(rename_all="snake_case")]
pub(super) enum ReadRejection { SourceBeforeRead, ClipboardBusy, SequenceChanged, OwnerBeforeRead, OwnerAfterRead, SourceAfterRead }
#[derive(Clone,Copy,Debug,Default,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
pub(super) struct ReadDiagnostics {
    #[serde(skip_serializing_if="Option::is_none")]
    pub rejected_at: Option<ReadRejection>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub source_rejection: Option<SourceRejection>,
    pub materialization_started: bool,
    #[serde(skip_serializing_if="Option::is_none")]
    pub sequence_matches_before_read: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub source_owner_before_read: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub source_owner_after_read: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub sequence_changed_during_read: Option<bool>,
}
impl ReadDiagnostics {
    fn source_valid(&mut self, reason: Option<SourceRejection>, stage: ReadRejection) -> bool {
        if reason.is_none() {return true;}
        self.rejected_at=Some(stage);self.source_rejection=reason;false
    }
    fn begin(&mut self, actual: u32, expected: u32, owner: bool) -> bool {
        self.sequence_matches_before_read=Some(actual==expected);self.source_owner_before_read=Some(owner);
        if actual!=expected {self.rejected_at=Some(ReadRejection::SequenceChanged);return false;}
        if !owner {self.rejected_at=Some(ReadRejection::OwnerBeforeRead);return false;}
        self.materialization_started=true;true
    }
}
pub fn read_copied(request: &TextRequest, expected: u32, detail: &mut ReadDiagnostics) -> CopiedText { read_text(request,expected,false,detail) }
pub fn read_observed(request: &TextRequest, expected: u32, detail: &mut ReadDiagnostics) -> CopiedText { read_text(request,expected,true,detail) }
fn read_text(request: &TextRequest, expected: u32, allow_unowned: bool, detail: &mut ReadDiagnostics) -> CopiedText {
    *detail=ReadDiagnostics::default();
    // Only a caller's fresh, gesture-bound passive observation permits a null
    // owner. Synthetic copy verification always requires the source's owner.
    let owns=||source_owns(request) || allow_unowned && unsafe {GetClipboardOwner()}==0;
    let failed = |selection| CopiedText { selection, sequence: None };
    if !detail.source_valid(source_rejection(request),ReadRejection::SourceBeforeRead) {return failed(Selection::SourceMismatch);}
    let Some(_lock) = ClipboardLock::open(0, Duration::from_millis(80)) else {
        detail.rejected_at=Some(ReadRejection::ClipboardBusy);return failed(Selection::CopyUnavailable);
    };
    let locked_sequence=unsafe {GetClipboardSequenceNumber()};
    // A new copy after the parent's locked snapshot is still rejected, even
    // when it came from this same application. Never adopt an arbitrary update.
    if !detail.begin(locked_sequence,expected,owns()) {return failed(Selection::SourceMismatch);}
    let mut result = (|| {
        let data = unsafe { GetClipboardData(13) };
        if data == 0 { return Selection::CopyUnavailable; }
        let bytes = unsafe { GlobalSize(data) };
        if bytes == 0 || bytes > 8192 || bytes % 2 != 0 { return Selection::InvalidWord; }
        let pointer = unsafe { GlobalLock(data) } as *const u16;
        if pointer.is_null() { return Selection::CopyUnavailable; }
        let text = unsafe { std::slice::from_raw_parts(pointer, bytes / 2) };
        let result = text.iter().position(|value| *value == 0)
            .and_then(|end| String::from_utf16(&text[..end]).ok())
            .and_then(|text| selected_word(&text)).map(Selection::Word).unwrap_or(Selection::InvalidWord);
        unsafe { GlobalUnlock(data); }
        result
    })();
    // Materialization may advance the sequence while this same source owns the
    // locked clipboard. Return that final sequence even for invalid text so the
    // independent backup helper can still restore the original formats.
    let owner=owns();detail.source_owner_after_read=Some(owner);
    let sequence=owner.then(||unsafe {GetClipboardSequenceNumber()});
    detail.sequence_changed_during_read=sequence.map(|value|value!=locked_sequence);
    if !owner {detail.rejected_at=Some(ReadRejection::OwnerAfterRead);result=Selection::SourceMismatch;}
    else if !detail.source_valid(source_rejection(request),ReadRejection::SourceAfterRead) {result=Selection::SourceMismatch;}
    CopiedText { selection: result, sequence }
}

#[cfg(test)]
mod tests {
    #[test] fn menu_mask_partial_send_is_remembered_until_our_keyup_succeeds() {
        use std::sync::atomic::{AtomicBool,Ordering};
        for sent in [0,2] {
            let pending=AtomicBool::new(false);
            assert_eq!(complete_menu_pair(sent,&pending,true,||panic!("no partial key to release")),sent==2);
            assert!(!pending.load(Ordering::SeqCst));
        }
        let pending=AtomicBool::new(false);
        assert!(!complete_menu_pair(1,&pending,true,||0));assert!(pending.load(Ordering::SeqCst));
        assert!(!recover_menu_key(&pending,true,||0));assert!(pending.load(Ordering::SeqCst));
        assert!(recover_menu_key(&pending,true,||1));assert!(!pending.load(Ordering::SeqCst));
        assert!(recover_menu_key(&pending,true,||panic!("never release a key we did not press")));
        assert!(!complete_menu_pair(1,&pending,true,||1));assert!(!pending.load(Ordering::SeqCst));
    }


    #[test] fn partial_menu_modifier_waits_for_the_users_physical_key_release() {
        use std::sync::atomic::{AtomicBool,Ordering};
        let pending=AtomicBool::new(false);
        assert!(!complete_menu_pair(1,&pending,false,||panic!("do not release the user's left Ctrl")));
        assert!(pending.load(Ordering::SeqCst));
        assert!(!recover_menu_key(&pending,false,||panic!("refresh must also respect a real held Ctrl")));
        assert!(pending.load(Ordering::SeqCst));
        assert!(!recover_menu_key(&pending,true,||0),"denied injection must remain retryable");
        assert!(pending.load(Ordering::SeqCst));
        assert!(recover_menu_key(&pending,true,||1));
        assert!(!pending.load(Ordering::SeqCst));
        assert!(recover_menu_key(&pending,false,||panic!("no synthetic key is pending")));
    }

    use super::*;
    #[test] fn copied_reader_rejects_a_new_copy_even_when_the_same_source_still_owns_it() {
        for owner in [false,true] {
            let mut detail=ReadDiagnostics::default();
            assert!(!detail.begin(12,11,owner));assert!(!detail.materialization_started);
            assert_eq!(detail.rejected_at,Some(ReadRejection::SequenceChanged));
        }
        let mut detail=ReadDiagnostics::default();
        assert!(!detail.begin(11,11,false));assert_eq!(detail.rejected_at,Some(ReadRejection::OwnerBeforeRead));
        assert!(!detail.materialization_started);
        let mut detail=ReadDiagnostics::default();
        assert!(detail.begin(11,11,true));assert!(detail.materialization_started);assert_eq!(detail.rejected_at,None);
    }
    #[test] fn copied_reader_diagnostics_distinguish_source_rejection_before_and_after_materialization() {
        for stage in [ReadRejection::SourceBeforeRead,ReadRejection::SourceAfterRead] {
            let mut detail=ReadDiagnostics::default();
            if stage==ReadRejection::SourceAfterRead {assert!(detail.begin(11,11,true));}
            assert!(!detail.source_valid(Some(SourceRejection::FocusChanged),stage));
            assert_eq!(detail.rejected_at,Some(stage));assert_eq!(detail.source_rejection,Some(SourceRejection::FocusChanged));
            assert_eq!(detail.materialization_started,stage==ReadRejection::SourceAfterRead);
        }
    }
    #[test] fn notepad_regression_partial_copy_or_skipped_restore_cannot_be_retried() {
        for input_sent in [None,Some(false),Some(true)] {
            for restore_completed in [None,Some(false),Some(true)] {
                let report=CopyDiagnostics{input_sent,restore_completed,..Default::default()};
                assert_eq!(report.input_attempted(),input_sent.is_some());
            }
        }
    }
    #[test] fn restoration_diagnostics_distinguish_completed_skipped_and_failed() {
        use super::super::copy_policy::Backend;
        let request=TextRequest{pid:0,window:0,focus:0,origin:super::super::windows_text::TextOrigin::Window};
        for (state,accepted,completed) in [(Some(RestoreState::Restored),true,Some(true)),
            (Some(RestoreState::Skipped),true,Some(false)),(None,false,None)] {
            let mut backend=NativeBackend{request:&request,valid:||true,release:||None,
                read:|_|None,restore:|_|state,started:Instant::now(),before:1,
                diagnostics:Cell::new(CopyDiagnostics::default())};
            assert_eq!(backend.restore(Some(2)),accepted);
            assert_eq!(backend.diagnostics.get().restore_completed,completed);
        }
    }
    #[link(name = "kernel32")]
    extern "system" { fn GlobalAlloc(flags: u32, bytes: usize) -> Handle; }
    fn buffer(id: u32, contents: &[u8]) -> Format { unsafe {
        let data = GlobalAlloc(2, contents.len()); assert_ne!(data, 0);
        let target = GlobalLock(data) as *mut u8; assert!(!target.is_null());
        ptr::copy_nonoverlapping(contents.as_ptr(), target, contents.len()); GlobalUnlock(data);
        Format { id, data, bytes:contents.len() }
    } }
    #[test] fn duplicates_text_image_and_file_list_without_touching_system_clipboard() { unsafe {
        // Real Windows movable memory/duplication, no Open/Empty/SetClipboard or input.
        let unicode: Vec<u8> = "原有内容 🧪\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut dib = vec![0u8; 44]; dib[..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes()); dib[8..12].copy_from_slice(&1i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes()); dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[40..].copy_from_slice(&[32, 64, 128, 255]);
        let files: Vec<u8> = "D:\\one.pdf\0D:\\two.txt\0\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut dropfiles = vec![0u8; 20]; dropfiles[..4].copy_from_slice(&20u32.to_le_bytes());
        dropfiles[16..20].copy_from_slice(&1u32.to_le_bytes()); dropfiles.extend(files);
        for (id, bytes) in [(13, unicode), (8, dib), (15, dropfiles)] {
            let original = buffer(id, &bytes); let copy = snapshot_format(id, original.data, &mut 0).unwrap();
            assert_ne!(original.data, copy.data); drop(original);
            let data = GlobalLock(copy.data) as *const u8; assert!(!data.is_null());
            assert_eq!(std::slice::from_raw_parts(data, bytes.len()), bytes.as_slice());
            GlobalUnlock(copy.data);
        }
    } }
    #[test] fn capture_race_delayed_rendering_discards_partial_native_backup_before_copy() {
        let stale=failure(BackupStage::ReadFormat,Some(49400),false);
        assert_eq!(coherent_snapshot::<()>(Err(stale),false).unwrap_err().stage,BackupStage::SourceChanged);
        assert_eq!(coherent_snapshot(Ok(42),false).unwrap_err().stage,BackupStage::SourceChanged);
        assert_eq!(coherent_snapshot::<()>(Err(stale),true),Err(stale),"an unchanged unreadable format is not treated as empty");
        assert!(needs_ole_retry(&stale));
        assert!(!needs_ole_retry(&BackupFailure{win32_error:Some(5),..stale}),"explicit access denial is final");
        assert!(!needs_ole_retry(&failure(BackupStage::UnsupportedFormat,Some(49400),false)));
    }
    #[test] fn capture_race_complete_ole_metadata_is_kept_when_native_handle_is_unavailable() { unsafe {
        // Private memory only; no Get/SetClipboardData. The metadata format is
        // retained with real bytes, not dropped or re-created as delayed NULL.
        #[link(name="user32")] extern "system" {fn RegisterClipboardFormatW(name:*const u16)->u32;}
        // Format IDs are assigned per Windows session. A hard-coded ID can
        // become DataObject/OLE transport after another application registers
        // formats. Reserve only this fixture's opaque metadata name.
        let name:Vec<u16>="WordSide synthetic metadata fixture\0".encode_utf16().collect();
        let id=RegisterClipboardFormatW(name.as_ptr());assert!(id>=0xc000);
        let original=buffer(id,b"synthetic-identity\0"); let mut budget=0;
        let saved=snapshot_format(original.id,original.data,&mut budget).unwrap();
        SetLastError(0); let kept=native_payload(saved,0,&mut budget).unwrap();
        assert_eq!(kept.id,id); assert_ne!(kept.data,0);
        let bytes=GlobalLock(kept.data) as *const u8;
        assert_eq!(std::slice::from_raw_parts(bytes,19),b"synthetic-identity\0"); GlobalUnlock(kept.data);
        let saved=snapshot_format(original.id,original.data,&mut 0).unwrap();
        SetLastError(5); assert!(native_payload(saved,0,&mut 0).is_err());
    } }
    #[test] fn replacing_an_ole_payload_reuses_its_budget_and_keeps_native_bytes() { unsafe {
        let original=buffer(13,&[b'a',0,0,0,0x11,0x22,0x33,0x44]);
        let saved=snapshot_format(13,original.data,&mut 0).unwrap();
        let mut budget=MAX_BYTES;
        let native=native_payload(saved,original.data,&mut budget).unwrap();
        assert_eq!(budget,MAX_BYTES);
        let bytes=GlobalLock(native.data) as *const u8; assert!(!bytes.is_null());
        assert_eq!(std::slice::from_raw_parts(bytes,8),[b'a',0,0,0,0x11,0x22,0x33,0x44]); GlobalUnlock(native.data);
    } }
    #[test] fn differing_text_and_unknown_format_padding_still_refuse_a_merge() {
        for(id,left,right) in [(13,vec![b'a',0,0,0],vec![b'b',0,0,0]),(1,b"first\0".to_vec(),b"other\0".to_vec())] {
            let saved=buffer(id,&left); let native=buffer(id,&right); assert!(verify_payload(&saved,native.data).is_err());
        }
        assert!(!payload_equal(0xc321,b"same\0padding",b"same\0"));
        assert!(!payload_equal(13,&[b'a',0],&[b'a',0]));
    }
    #[test] fn equal_standard_text_with_different_allocation_padding_is_preserved() {
        for (id, left, right) in [
            (13, vec![b'a',0,0,0], vec![b'a',0,0,0,0x55,0x66,0x77,0x88]),
            (1, b"same\0".to_vec(), b"same\0unused padding".to_vec()),
            (7, b"same\0padding".to_vec(), b"same\0".to_vec()),
        ] {
            let ole = buffer(id, &left); let native = buffer(id, &right);
            assert_eq!(verify_payload(&ole, native.data), Ok(()), "format {id}");
        }
    }
    #[test] fn browser_text_formats_clone_as_plain_global_memory() { unsafe {
        #[link(name="user32")] extern "system" { fn RegisterClipboardFormatW(name: *const u16) -> u32; }
        // Registering a format name does not open, read or change clipboard data.
        let custom = |name: &str| { let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let id = RegisterClipboardFormatW(name.as_ptr()); assert!(id >= 0xc000); id };
        let html = custom("HTML Format");
        let token = custom("Chromium internal source RFH token");
        let url = custom("Chromium internal source URL");
        let unicode: Vec<u8> = "synthetic 中文\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let cases = [(html, b"Version:0.9\r\nStartHTML:00000000\r\nsynthetic\0".to_vec()),
            (13, unicode), (token, vec![20,0,0,0,1,0,0,0,2,0,0,0,0,0,0,0,3,0,0,0,0,0,0,0]),
            (url, vec![0]), (16, 0x409u32.to_le_bytes().to_vec()),
            (1, b"synthetic\0".to_vec()), (7, b"synthetic\0".to_vec())];
        let mut budget = 0;
        for (id, bytes) in cases {
            let original = buffer(id, &bytes);
            let copy = snapshot_format(id, original.data, &mut budget).unwrap(); drop(original);
            let pointer = GlobalLock(copy.data) as *const u8; assert!(!pointer.is_null());
            assert_eq!(std::slice::from_raw_parts(pointer, bytes.len()), bytes.as_slice()); GlobalUnlock(copy.data);
        }
        assert!(budget > 0 && budget < MAX_BYTES);
    } }
    #[test] fn unsupported_or_oversize_formats_refuse_backup_before_any_copy() {
        let ordinary = buffer(13, &[0, 0]);
        for id in [0x80, 0x200, 0x2ff, 0x300, 0x3ff] {
            assert!(snapshot_format(id, ordinary.data, &mut 0).is_err());
        }
        assert!(snapshot_format(13, ordinary.data, &mut { MAX_BYTES }).is_err());
        assert!(snapshot_format(13, 0, &mut 0).is_err());
    }
    #[test] fn palette_synthesized_from_images_is_cloned_with_entries_and_a_budget() { unsafe {
        #[link(name="gdi32")] extern "system" { fn CreatePalette(palette: *const c_void) -> Handle; }
        let bytes = [0u8,3,2,0, 10,20,30,0, 50,60,70,0];
        let original = Format { id:9, data:CreatePalette(bytes.as_ptr() as _), bytes:bytes.len() }; assert_ne!(original.data,0);
        let mut budget=0;let copy=snapshot_format(9,original.data,&mut budget).unwrap();
        assert_eq!(budget,12);drop(original);
        let mut entries=[0u8;8];assert_eq!(GetPaletteEntries(copy.data,0,2,entries.as_mut_ptr() as _),2);
        assert_eq!(entries,[10,20,30,0,50,60,70,0]);
        assert!(snapshot_format(9,copy.data,&mut {MAX_BYTES}).is_err());
    } }
    #[test] fn ole_and_rendered_payloads_must_agree_before_they_are_merged() { unsafe {
        let old = buffer(13, &[65,0,0,0]); let equal = snapshot_format(13,old.data,&mut 0).unwrap();
        verify_payload(&equal,old.data).unwrap();
        let changed = buffer(13, &[66,0,0,0]);
        assert_eq!(verify_payload(&equal,changed.data).unwrap_err().stage,BackupStage::PayloadMismatch);
        #[link(name="gdi32")] extern "system" { fn CreatePalette(palette:*const c_void)->Handle; fn CreateBitmap(width:i32,height:i32,planes:u32,bits:u32,data:*const c_void)->Handle; }
        let palette = [0u8,3,2,0, 10,20,30,0, 50,60,70,0]; let other = [0u8,3,2,0, 11,20,30,0, 50,60,70,0];
        let original = Format{id:9,data:CreatePalette(palette.as_ptr() as _),bytes:palette.len()};
        let equal = snapshot_format(9,original.data,&mut 0).unwrap();verify_payload(&equal,original.data).unwrap();
        let changed = Format{id:9,data:CreatePalette(other.as_ptr() as _),bytes:other.len()};
        assert_eq!(verify_payload(&equal,changed.data).unwrap_err().stage,BackupStage::PayloadMismatch);
        let original = Format{id:2,data:CreateBitmap(1,1,1,32,[10u8,20,30,0].as_ptr() as _),bytes:4};
        let equal = snapshot_format(2,original.data,&mut 0).unwrap();verify_payload(&equal,original.data).unwrap();
        let changed = Format{id:2,data:CreateBitmap(1,1,1,32,[11u8,20,30,0].as_ptr() as _),bytes:4};
        assert_eq!(verify_payload(&equal,changed.data).unwrap_err().stage,BackupStage::PayloadMismatch);
    } }
    #[test] fn input_abi_and_safe_copy_chord() {
        assert_eq!(std::mem::size_of::<Input>(), if cfg!(target_pointer_width="64") { 40 } else { 28 });
        let insert = key(0x2d, false, true);
        unsafe { assert_eq!(insert.data.keyboard.flags, 1); assert_eq!(insert.data.keyboard.extra, INPUT_MARKER); }
    }
}

#[cfg(test)]
#[path = "windows_copy_tests.rs"]
mod isolated_tests;
