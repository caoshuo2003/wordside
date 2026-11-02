//! The notification-area icon and its menu. The glyph is drawn by `icon` at the size the
//! tray asks for, white on a dark taskbar and near-black on a light one.
use crate::menu::{cursor, popup, Item};
use windows::{
    core::w,
    Win32::{
        Foundation::HWND,
        UI::{
            Shell::{
                Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
                NOTIFYICONDATAW, NOTIFY_ICON_MESSAGE,
            },
            WindowsAndMessaging::*,
        },
    },
};

/// The tray reports clicks with this message; `lparam` carries the mouse message.
pub const WM_TRAY: u32 = WM_APP + 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    ToggleAuto,
    Lookup,
    Settings,
    Quit,
}

fn tooltip(enabled: bool) -> &'static str {
    if enabled {
        "WordSide · 自动划词已开启"
    } else {
        "WordSide · 自动划词已关闭"
    }
}

fn data(hwnd: HWND, enabled: bool) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        ..Default::default()
    };
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    if let Ok(icon) = crate::icon::tray(size, crate::app::light_taskbar()) {
        data.hIcon = icon;
    }
    for (slot, unit) in data.szTip.iter_mut().zip(tooltip(enabled).encode_utf16()) {
        *slot = unit;
    }
    data
}

fn notify(message: NOTIFY_ICON_MESSAGE, hwnd: HWND, enabled: bool) -> bool {
    let data = data(hwnd, enabled);
    let done = unsafe { Shell_NotifyIconW(message, &data) }.as_bool();
    if !data.hIcon.is_invalid() {
        // The shell keeps its own copy of the icon.
        unsafe {
            let _ = DestroyIcon(data.hIcon);
        }
    }
    done
}

/// Also called when Explorer restarts, which drops every icon.
pub fn add(hwnd: HWND, enabled: bool) -> bool {
    notify(NIM_ADD, hwnd, enabled)
}

/// Also redraws the glyph, so a taskbar that turned light or dark gets the right color.
pub fn update(hwnd: HWND, enabled: bool) {
    notify(NIM_MODIFY, hwnd, enabled);
}

pub fn remove(hwnd: HWND) {
    notify(NIM_DELETE, hwnd, false);
}

/// The message Explorer broadcasts when the taskbar comes back.
pub fn taskbar_created() -> u32 {
    unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }
}

/// Shows the menu at the mouse and returns the chosen command.
pub fn menu(hwnd: HWND, enabled: bool, hotkey: &str) -> Option<Command> {
    let lookup = if hotkey.is_empty() {
        "查词".to_owned()
    } else {
        format!("查词\t{hotkey}")
    };
    let items = [
        Item::Command {
            label: "自动划词",
            checked: enabled,
        },
        Item::Separator,
        Item::command(&lookup),
        Item::command("设置"),
        Item::Separator,
        Item::command("退出"),
    ];
    let commands = [
        Some(Command::ToggleAuto),
        None,
        Some(Command::Lookup),
        Some(Command::Settings),
        None,
        Some(Command::Quit),
    ];
    popup(hwnd, &items, cursor()).and_then(|index| commands[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tooltip_tells_whether_auto_lookup_is_on() {
        assert!(tooltip(true).contains("已开启"));
        assert!(tooltip(false).contains("已关闭"));
        assert!(tooltip(true).encode_utf16().count() < 128);
    }
}
