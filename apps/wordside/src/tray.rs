//! The notification-area icon and its menu. The icon is 0.2.0's green palette icon: one ICO
//! holding PNG images from 16 to 64 px, of which the size the tray asks for is picked.
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
const ICON: &[u8] = include_bytes!("../assets/tray.ico");

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    ToggleAuto,
    Lookup,
    Settings,
    Quit,
}

/// The ICO's image for `size` px: the smallest one at least that big, else the largest.
fn image(ico: &[u8], size: u32) -> Option<&[u8]> {
    let count = u16::from_le_bytes(ico.get(4..6)?.try_into().ok()?) as usize;
    let mut best: Option<(u32, &[u8])> = None;
    for index in 0..count {
        let entry = ico.get(6 + 16 * index..22 + 16 * index)?;
        let width = match entry[0] {
            0 => 256,
            width => width as u32,
        };
        let len = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
        let data = ico.get(offset..offset.checked_add(len)?)?;
        let better = match best {
            None => true,
            Some((current, _)) if current >= size => width >= size && width < current,
            Some((current, _)) => width > current,
        };
        if better {
            best = Some((width, data));
        }
    }
    best.map(|(_, data)| data)
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
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    if let Some(png) = image(ICON, size as u32) {
        if let Ok(icon) =
            unsafe { CreateIconFromResourceEx(png, true, 0x0003_0000, size, size, LR_DEFAULTCOLOR) }
        {
            data.hIcon = icon;
        }
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
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    fn width(png: &[u8]) -> u32 {
        u32::from_be_bytes(png[16..20].try_into().unwrap())
    }

    // Ported from 0.2.0 `picks_the_smallest_image_at_least_as_big`, for the one icon kept.
    #[test]
    fn picks_the_smallest_image_at_least_as_big() {
        for (want, got) in [
            (16, 16),
            (20, 20),
            (24, 24),
            (28, 32),
            (32, 32),
            (40, 40),
            (48, 48),
            (56, 64),
            (64, 64),
            (96, 64),
            (1, 16),
        ] {
            let png = image(ICON, want).unwrap();
            assert!(png.starts_with(PNG), "{want}");
            assert_eq!(width(png), got, "{want}");
        }
    }

    // Ported from 0.2.0 `malformed_ico_is_refused`.
    #[test]
    fn malformed_ico_is_refused() {
        assert_eq!(image(&[], 16), None);
        assert_eq!(image(&ICON[..10], 16), None);
        assert_eq!(
            image(&ICON[..ICON.len() - 1], 64),
            None,
            "the last image is cut short"
        );
    }

    #[test]
    fn the_tooltip_tells_whether_auto_lookup_is_on() {
        assert!(tooltip(true).contains("已开启"));
        assert!(tooltip(false).contains("已关闭"));
        assert!(tooltip(true).encode_utf16().count() < 128);
    }
}
