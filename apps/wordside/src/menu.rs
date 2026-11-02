//! The system's popup menu, used by the tray and by right-clicks on the card. It follows the
//! system theme and works with screen readers, which a drawn menu would not.
use windows::{
    core::{HSTRING, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, POINT, WPARAM},
        UI::WindowsAndMessaging::*,
    },
};

pub enum Item<'a> {
    /// A label; `\t` separates the shortcut shown on the right.
    Command {
        label: &'a str,
        checked: bool,
    },
    Separator,
}

impl<'a> Item<'a> {
    pub fn command(label: &'a str) -> Self {
        Self::Command {
            label,
            checked: false,
        }
    }
}

/// Shows the menu at `at` (screen pixels) and returns the index of the chosen item.
pub fn popup(hwnd: HWND, items: &[Item], at: POINT) -> Option<usize> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for (index, item) in items.iter().enumerate() {
            let _ = match item {
                Item::Command { label, checked } => {
                    let check = if *checked { MF_CHECKED } else { MF_UNCHECKED };
                    AppendMenuW(menu, MF_STRING | check, index + 1, &HSTRING::from(*label))
                }
                Item::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()),
            };
        }
        // Without this the menu stays open after a click elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            at.x,
            at.y,
            None,
            hwnd,
            None,
        )
        .0;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        (chosen > 0).then(|| chosen as usize - 1)
    }
}

pub fn cursor() -> POINT {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    point
}
