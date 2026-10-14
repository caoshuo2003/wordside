//! Read-only fallback for ordinary Win32 EDIT controls (including rename fields).
//! All messages are system messages, marshalled by Windows across processes.
use super::{selection::Selection, policy::selected_word};
use std::time::Instant;

type Handle = isize;
const EM_GETSEL: u32 = 0x00b0;
const EM_GETPASSWORDCHAR: u32 = 0x00d2;
const WM_GETTEXT: u32 = 0x000d;
const MESSAGE_TIMEOUT_MS: u32 = 150;
const MAX_PREFIX_UTF16: u32 = 256 * 1024;

#[link(name = "user32")]
extern "system" {
    fn GetClassNameW(window: Handle, text: *mut u16, count: i32) -> i32;
    fn GetAncestor(window: Handle, flags: u32) -> Handle;
    fn GetWindowLongW(window: Handle, index: i32) -> i32;
    fn SendMessageTimeoutW(window: Handle, message: u32, wparam: usize,
        lparam: isize, flags: u32, timeout: u32, result: *mut usize) -> isize;
}
#[link(name = "kernel32")]
extern "system" {
    fn SetLastError(code: u32);
    fn GetLastError() -> u32;
}
fn native_error(operation: &str, code: u32) -> Selection {
    match code {
        5 => Selection::AccessDenied,
        1400 => Selection::SourceMismatch,
        1460 => Selection::Timeout,
        _ => Selection::ReadError(format!("{operation}: Win32 {code}")),
    }
}
unsafe fn message(window: Handle, id: u32, wparam: usize, lparam: isize)
    -> Result<usize, Selection>
{
    let mut result = 0;
    SetLastError(0);
    let started = Instant::now();
    // BLOCK | ABORTIFHUNG | ERRORONEXIT. No NOTIMEOUTIFNOTHUNG flag.
    if SendMessageTimeoutW(window, id, wparam, lparam, 0x23,
        MESSAGE_TIMEOUT_MS, &mut result) == 0
    {
        let code = GetLastError();
        // Windows may leave LastError unset; only classify it as timeout when
        // the actual wait exhausted our bound, otherwise retain a read error.
        return Err(if code == 0 && started.elapsed().as_millis() >= u128::from(MESSAGE_TIMEOUT_MS) {
            Selection::Timeout
        } else { native_error("Edit message", code) });
    }
    Ok(result)
}
unsafe fn password_guard(window: Handle) -> Result<(), Selection> {
    SetLastError(0);
    let style = GetWindowLongW(window, -16) as u32;
    let error = GetLastError();
    if style == 0 && error != 0 { return Err(native_error("Edit style", error)); }
    if style & 0x20 != 0 || message(window, EM_GETPASSWORDCHAR, 0, 0)? != 0 {
        return Err(Selection::Protected);
    }
    Ok(())
}
unsafe fn range(window: Handle) -> Result<(u32, u32), Selection> {
    // Full DWORD offsets, not the packed 16-bit return value of EM_GETSEL.
    let (mut start, mut end) = (0u32, 0u32);
    message(window, EM_GETSEL, (&mut start as *mut u32) as usize,
        (&mut end as *mut u32) as isize)?;
    Ok((start, end))
}
/// None means this HWND is not an ordinary EDIT; preserve the UIA route.
/// A recognised EDIT with an empty selection returns NoSelection, so the
/// caller cannot borrow an unrelated parent control's retained selection.
pub fn selected_edit(window: Handle, source: Handle) -> Option<Selection> {
    unsafe {
        let mut name = [0u16; 64];
        let count = GetClassNameW(window, name.as_mut_ptr(), name.len() as i32);
        if count <= 0 || !String::from_utf16_lossy(&name[..count as usize]).eq_ignore_ascii_case("Edit") {
            return None;
        }
        let source_root = GetAncestor(source, 2);
        let source_root = if source_root == 0 { source } else { source_root };
        if GetAncestor(window, 2) != source_root { return Some(Selection::SourceMismatch); }
        let read = || -> Result<Selection, Selection> {
            password_guard(window)?;
            let (start, end) = range(window)?;
            if start == end { return Ok(Selection::NoSelection); }
            if start > end || end - start > 100 || end > MAX_PREFIX_UTF16 {
                return Ok(Selection::InvalidWord);
            }
            password_guard(window)?;
            // Read only the necessary prefix, never a suffix or the window
            // title. Only the selected UTF-16 slice leaves this function.
            let mut text = vec![0u16; end as usize + 1];
            let count = message(window, WM_GETTEXT, text.len(), text.as_mut_ptr() as isize)?;
            if count < end as usize || count >= text.len() { return Err(Selection::SourceMismatch); }
            if range(window)? != (start, end) || GetAncestor(window, 2) != source_root {
                return Err(Selection::SourceMismatch);
            }
            password_guard(window)?;
            let selected = String::from_utf16(&text[start as usize..end as usize])
                .map_err(|_| Selection::InvalidWord)?;
            Ok(selected_word(&selected).map(Selection::Word).unwrap_or(Selection::InvalidWord))
        };
        Some(read().unwrap_or_else(|failure| failure))
    }
}
