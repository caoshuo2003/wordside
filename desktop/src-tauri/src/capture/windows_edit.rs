//! Read-only selection from standard Win32 EDIT and Unicode RichEdit controls.
//! All messages are system messages, marshalled by Windows across processes.
use super::{selection::Selection, policy::selected_word};
use std::time::Instant;

type Handle = isize;
const EM_GETSEL: u32 = 0x00b0;
const EM_GETPASSWORDCHAR: u32 = 0x00d2;
const WM_GETTEXT: u32 = 0x000d;
const EM_LINEINDEX: u32 = 0x00bb;
const EM_GETLINE: u32 = 0x00c4;
// Scalar-only RichEdit message: unlike EM_GETTEXTRANGE it carries no pointer.
const EM_EXLINEFROMCHAR: u32 = 0x0436;
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
fn rich_class(name: &str) -> bool {
    ["richedit20w","richedit50w","richedit60w","richeditd2d","richeditd2dpt"].contains(&name)
}
unsafe fn rich_slice(window: Handle, start: u32, end: u32) -> Result<Vec<u16>, Selection> {
    let mut cursor=start; let mut text=Vec::with_capacity((end-start) as usize);
    while cursor<end {
        let line=message(window,EM_EXLINEFROMCHAR,0,cursor as isize)?;
        let index=message(window,EM_LINEINDEX,line,0)?;
        if index>cursor as usize {return Err(Selection::SourceMismatch);}
        let next=message(window,EM_LINEINDEX,line+1,0)?;
        let until=if next==usize::MAX {end as usize} else {next.min(end as usize)};
        if until<=cursor as usize || until<index {return Err(Selection::SourceMismatch);}
        let needed=until-index;
        // EM_GETLINE's first WORD is the maximum UTF-16 count. No unchecked
        // narrowing, whole-document read, remote memory or pointer WM_USER call.
        if needed>u16::MAX as usize {return Err(Selection::Unsupported);}
        let mut line_text=vec![0u16;needed.max(1)]; line_text[0]=needed as u16;
        let count=message(window,EM_GETLINE,line,line_text.as_mut_ptr() as isize)?;
        if count>line_text.len() {return Err(Selection::SourceMismatch);}
        let offset=cursor as usize-index;
        if count<needed {
            // An incomplete line cannot reconstruct the exact selected text.
            // Never join different words across a missing gap into a false word.
            return Err(Selection::InvalidWord);
        }
        text.extend_from_slice(&line_text[offset..needed]); cursor=until as u32;
    }
    Ok(text)
}
/// None means this HWND is not a supported native text control.
/// A recognised control with an insertion point returns NoSelection; a parent
/// or old clipboard must not supply a retained selection for it.
pub fn selected_edit(window: Handle, source: Handle) -> Option<Selection> {
    unsafe {
        let mut name = [0u16; 64];
        let count = GetClassNameW(window, name.as_mut_ptr(), name.len() as i32);
        if count <= 0 {return None;}
        let name=String::from_utf16_lossy(&name[..count as usize]).to_ascii_lowercase();
        let rich=rich_class(&name);
        if name!="edit" && !rich {return None;}
        let source_root = GetAncestor(source, 2);
        let source_root = if source_root == 0 { source } else { source_root };
        if GetAncestor(window, 2) != source_root { return Some(Selection::SourceMismatch); }
        let read = || -> Result<Selection, Selection> {
            password_guard(window)?;
            let (start, end) = range(window)?;
            if start == end { return Ok(Selection::NoSelection); }
            if start > end || end - start > 100 {return Ok(Selection::InvalidWord);}
            password_guard(window)?;
            let text=if rich {
                // RichEdit uses CR positions internally; WM_GETTEXT may expand
                // CRLF. Line indices and line text share the native coordinate
                // system, so earlier paragraphs cannot shift the selected word.
                rich_slice(window,start,end)?
            } else {
                if end>MAX_PREFIX_UTF16 {return Ok(Selection::InvalidWord);}
                let mut text = vec![0u16; end as usize + 1];
                let count = message(window, WM_GETTEXT, text.len(), text.as_mut_ptr() as isize)?;
                if count < end as usize || count >= text.len() { return Err(Selection::SourceMismatch); }
                text[start as usize..end as usize].to_vec()
            };
            if range(window)? != (start, end) || GetAncestor(window, 2) != source_root {
                return Err(Selection::SourceMismatch);
            }
            password_guard(window)?;
            let selected = String::from_utf16(&text).map_err(|_| Selection::InvalidWord)?;
            Ok(selected_word(&selected).map(Selection::Word).unwrap_or(Selection::InvalidWord))
        };
        Some(read().unwrap_or_else(|failure| failure))
    }
}

#[cfg(test)]
#[path="windows_edit_tests.rs"]
mod tests;
