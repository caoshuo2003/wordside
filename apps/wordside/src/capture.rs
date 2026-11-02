//! Reading the selection: automatic lookup on select, the lookup shortcut, and the guarded
//! clipboard fallback. Brought over from 0.2.0 with its decision rules and timings
//! unchanged; only the link to the window is new (`Bridge` and `Card` below).
// 0.2.0 never ran clippy. These lints only ask to rewrite the decision code, which stays as
// it was proven in use; anything that is not a rewrite of that code was fixed instead.
#![allow(
    clippy::precedence,
    clippy::too_many_arguments,
    clippy::if_same_then_else,
    clippy::manual_range_patterns,
    clippy::question_mark,
    clippy::enum_variant_names,
    clippy::chunks_exact_to_as_chunks
)]
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU64},
        Arc,
    },
};
mod compatibility;
mod copy_policy;
#[cfg(windows)]
mod diagnostics;
#[cfg(windows)]
pub(crate) mod hotkey;
#[cfg(windows)]
mod input;
mod policy;
#[cfg(windows)]
pub(crate) mod probe;
pub mod selection;
pub mod shortcut;
mod timing;
#[cfg(windows)]
mod windows_copy;
#[cfg(test)]
use policy::selected_word;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_edit;
#[cfg(windows)]
pub(crate) mod windows_text;

/// What the window shows for a capture. Called on the window's thread.
pub trait Card {
    /// A capture started: put away what the card showed, unless it is pinned.
    fn capture_started(&mut self, manual: bool);
    /// Show the card for a captured word near `point` (screen pixels), without taking the
    /// focus from the window the word came from.
    fn captured(&mut self, word: &str, point: (i32, i32), manual: bool);
    /// The capture found nothing usable; `code` picks the notice.
    fn capture_failed(&mut self, code: &str, point: (i32, i32), manual: bool);
    /// Clicked elsewhere, or the source went away.
    fn dismiss(&mut self);
    /// The input hooks could not be installed; manual lookup still works.
    fn capture_unavailable(&mut self);
}

/// A job for the window's thread.
pub type Job = Box<dyn FnOnce(&mut dyn Card) + Send>;

/// State the capture thread and the window share, and the way to reach the window.
pub struct Bridge {
    /// Automatic lookup on select.
    pub enabled: AtomicBool,
    pub pinned: AtomicBool,
    /// Moves on whenever what the card shows no longer belongs to a capture in flight:
    /// the switch changed, the shortcut changed, the card was dismissed.
    pub capture_epoch: AtomicU64,
    pub capture_serial: AtomicU64,
    /// The card's window handle, published once it exists.
    pub card: AtomicIsize,
    pub hotkeys: Arc<hotkey::Control>,
    /// `capture-status.json`, beside the settings.
    pub status_file: PathBuf,
    /// Runs a job on the window's thread.
    pub post: Box<dyn Fn(Job) + Send + Sync>,
}

fn poll_interval(enabled: bool, popup_visible: bool) -> u64 {
    if !enabled && !popup_visible {
        250
    } else {
        20
    }
}
pub fn place(x: i32, y: i32, width: i32, height: i32, work: (i32, i32, i32, i32)) -> (i32, i32) {
    let (l, t, r, b) = work;
    let px = (x + 14).min(r - width - 8).max(l + 8);
    let py = if y + 18 + height > b - 8 {
        y - height - 12
    } else {
        y + 18
    };
    (px, py.min(b - height - 8).max(t + 8))
}
/// Starts capturing on its own thread, with `hotkey` as written in the settings.
pub fn start(bridge: Arc<Bridge>, hotkey: String) {
    std::thread::spawn(move || windows::run(bridge, hotkey));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_sources_and_self_exclusion() {
        use policy::permits_source;
        assert!(permits_source(2, 1));
        assert!(permits_source(3, 1));
        assert!(!permits_source(1, 1));
        assert!(!permits_source(0, 1));
    }
    #[test]
    fn intentional_mouse_and_copy_inputs() {
        use policy::{copy_chord, mouse_selection};
        assert!(!mouse_selection((0, 0), (2, 1), false, false));
        assert!(mouse_selection((0, 0), (5, 0), false, false));
        assert!(mouse_selection((0, 0), (0, 0), true, false));
        assert!(mouse_selection((0, 0), (0, 0), false, true));
        assert!(copy_chord(true, true, false, false, false));
        assert!(copy_chord(true, false, true, false, false));
        assert!(!copy_chord(false, true, false, false, false));
        assert!(!copy_chord(true, true, false, true, false));
        assert!(!copy_chord(true, true, false, false, true));
    }
    #[test]
    fn words_and_phrases() {
        assert_eq!(selected_word(" `terminal,` ").as_deref(), Some("terminal"));
        assert_eq!(
            selected_word("run on the spot").as_deref(),
            Some("run on the spot")
        );
        assert!(selected_word("don't").is_some());
        assert_eq!(selected_word("terminal\r\n").as_deref(), Some("terminal"));
    }
    #[test]
    fn idle_polling() {
        assert_eq!(poll_interval(false, false), 250);
        assert_eq!(poll_interval(true, false), 20);
        assert_eq!(poll_interval(false, true), 20);
    }
    #[test]
    fn punctuation_selection() {
        for s in [
            "“runtime”。",
            "(runtime),",
            "runtime()",
            "--runtime",
            "-runtime",
        ] {
            assert_eq!(selected_word(s).as_deref(), Some("runtime"));
        }
        assert_eq!(selected_word("termnal").as_deref(), Some("termnal"));
    }
    #[test]
    fn rejects_logs_paths_and_commands() {
        for s in [
            "foo\nbar",
            "C:\\Users\\Admin",
            "https://example.com",
            "rm -rf /",
            "12345",
            "",
        ] {
            assert!(selected_word(s).is_none(), "{s}");
        }
    }
    #[test]
    fn limits_clipboard() {
        assert!(selected_word(&"x".repeat(81)).is_none());
        assert!(selected_word("one two three four five six").is_none());
    }
    #[test]
    fn monitor_edges_and_negative_coordinates() {
        assert_eq!(place(1910, 1070, 480, 680, (0, 0, 1920, 1080)), (1432, 378));
        let (x, y) = place(-1900, 100, 480, 680, (-1920, 0, 0, 1080));
        assert!(x >= -1912 && x + 480 <= -8 && y >= 8 && y + 680 <= 1072);
    }
}
