//! The notification-area icon and its menu. The glyph is drawn by `icon` at the size the
//! tray asks for, white on a dark taskbar and near-black on a light one.
use crate::menu::{cursor, popup, Item};
use windows::{
    core::w,
    Win32::{
        Foundation::HWND,
        UI::{
            Shell::{
                Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD,
                NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, NOTIFY_ICON_MESSAGE,
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

/// What the icon's tooltip reports.
#[derive(Debug, Clone, Copy)]
pub struct Status<'a> {
    pub enabled: bool,
    /// The saved shortcut, when another program holds it and it could not be registered.
    pub blocked_hotkey: Option<&'a str>,
}

/// Arrives in `lparam` of `WM_TRAY` when the user clicks a notification from `warn`.
pub const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;

fn tooltip(status: Status) -> String {
    let auto = if status.enabled {
        "WordSide · 自动划词已开启"
    } else {
        "WordSide · 自动划词已关闭"
    };
    match status.blocked_hotkey {
        Some(hotkey) => format!("{auto}\n快捷键 {hotkey} 被其他程序占用，请在设置里换一个"),
        None => auto.to_owned(),
    }
}

fn copy(slot: &mut [u16], text: &str) {
    // Leave room for the terminating zero.
    let room = slot.len() - 1;
    for (place, unit) in slot.iter_mut().zip(text.encode_utf16().take(room)) {
        *place = unit;
    }
}

fn data(hwnd: HWND, status: Status) -> NOTIFYICONDATAW {
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
    copy(&mut data.szTip, &tooltip(status));
    data
}

fn notify(message: NOTIFY_ICON_MESSAGE, hwnd: HWND, status: Status) -> bool {
    let data = data(hwnd, status);
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
pub fn add(hwnd: HWND, status: Status) -> bool {
    notify(NIM_ADD, hwnd, status)
}

/// Also redraws the glyph, so a taskbar that turned light or dark gets the right color.
pub fn update(hwnd: HWND, status: Status) {
    notify(NIM_MODIFY, hwnd, status);
}

pub fn remove(hwnd: HWND) {
    notify(
        NIM_DELETE,
        hwnd,
        Status {
            enabled: false,
            blocked_hotkey: None,
        },
    );
}

/// A Windows notification from the tray icon. Clicking it sends `NIN_BALLOONUSERCLICK`.
pub fn warn(hwnd: HWND, status: Status, title: &str, text: &str) {
    let mut data = data(hwnd, status);
    data.uFlags |= NIF_INFO;
    data.dwInfoFlags = NIIF_WARNING;
    copy(&mut data.szInfoTitle, title);
    copy(&mut data.szInfo, text);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        if !data.hIcon.is_invalid() {
            let _ = DestroyIcon(data.hIcon);
        }
    }
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
        let status = |enabled| Status {
            enabled,
            blocked_hotkey: None,
        };
        assert!(tooltip(status(true)).contains("已开启"));
        assert!(tooltip(status(false)).contains("已关闭"));
    }

    #[test]
    fn a_blocked_shortcut_is_named_in_the_tooltip_and_fits() {
        let text = tooltip(Status {
            enabled: false,
            blocked_hotkey: Some("Ctrl+Alt+Shift+F24"),
        });
        assert!(text.contains("Ctrl+Alt+Shift+F24") && text.contains("被其他程序占用"));
        assert!(text.encode_utf16().count() < 128);
        let mut slot = [1u16; 8];
        copy(&mut slot, "0123456789");
        assert_eq!(
            slot[7], 1,
            "the last unit stays free for the terminating zero"
        );
    }
}
