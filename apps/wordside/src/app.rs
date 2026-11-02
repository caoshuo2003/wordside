//! The window, the message loop and everything that reacts to input.
use crate::{
    audio::{self, Player},
    card::{
        allowed_link, pressed_combo, sense_text, Content, EntryView, FailureView, SettingsChange,
        SettingsKey, SettingsView, Sound,
    },
    dictionary,
    entry::{read_api_entry, Entry, LookupError},
    menu,
    presentation::shortcut_notice,
    render::{self, Action, Frame, Theme, DARK, INPUT_H, LIGHT, WIDTH},
    selection_input::selected_word,
    service::Service,
    settings::{self, Settings},
    shortcut,
    tray::{self, Command, WM_TRAY},
};
use serde_json::json;
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Instant,
};
use windows::{
    core::{s, w, Interface, Result, BOOL, HSTRING},
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dwm::*,
            Dxgi::Common::*,
            Gdi::*,
        },
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
            },
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
            Threading::*,
        },
        UI::{
            Controls::{MARGINS, WM_MOUSELEAVE},
            HiDpi::*,
            Input::{
                Ime::{ImmAssociateContext, HIMC},
                KeyboardAndMouse::*,
            },
            Shell::ShellExecuteW,
            WindowsAndMessaging::*,
        },
    },
};

const WM_RESULT: u32 = WM_APP + 1;
/// A pronunciation finished downloading.
const WM_CLIP: u32 = WM_APP + 2;
/// The player ended (lparam 0) or failed (lparam 1); wparam is the sound's serial.
const WM_SOUND: u32 = WM_APP + 3;
/// Another launch asks the running copy to show the card.
const WM_SHOW: u32 = WM_APP + 5;
const CF_UNICODETEXT: u32 = 13;
const SHOW_HOTKEY: i32 = 1;
const CLASS: windows::core::PCWSTR = w!("WordSideCard");
/// Per session, named after the app identifier so it never collides with other apps.
/// Its existence marks the running copy; signalling it asks that copy to show the card.
const INSTANCE: windows::core::PCWSTR = w!("Local\\WordSide-local.wordside.dictionary.api-preview");
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// Grayscale contrast for text on glass; Windows' default is 1.0.
const GLASS_TEXT_CONTRAST: f32 = 1.6;

type Answer = (u64, String, Result2);
type Result2 = std::result::Result<Entry, LookupError>;
type Clip = (u64, String, std::result::Result<Vec<u8>, ()>);

thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; }
static ANSWER: Mutex<Option<Answer>> = Mutex::new(None);
static CLIP: Mutex<Option<Clip>> = Mutex::new(None);
static RUNTIME: OnceLock<(tokio::runtime::Runtime, Arc<Service>)> = OnceLock::new();

/// Created on first use, so a card that was never used costs nothing here.
fn runtime() -> &'static (tokio::runtime::Runtime, Arc<Service>) {
    RUNTIME.get_or_init(|| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        (runtime, Arc::new(Service::default()))
    })
}

#[derive(Clone, Copy, PartialEq)]
enum ThemeChoice {
    System,
    Light,
    Dark,
}

struct App {
    hwnd: HWND,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    target: Option<ID2D1HwndRenderTarget>,
    frame: Frame,
    content: Content,
    query: String,
    scroll: f32,
    /// Where the mouse is over the card, for the row tint; None once it leaves.
    mouse: Option<(f32, f32)>,
    pinned: bool,
    choice: ThemeChoice,
    dark: bool,
    glass: bool,
    serial: u64,
    last_word: String,
    task: Option<tokio::task::JoinHandle<()>>,
    /// The sound being loaded or played: its serial and chip.
    sound: Option<(u64, usize)>,
    sound_serial: u64,
    sound_task: Option<tokio::task::JoinHandle<()>>,
    player: Option<Player>,
    /// Sounds of the current entry, so replaying doesn't download again.
    clips: HashMap<String, Vec<u8>>,
    settings: Settings,
    settings_path: Option<PathBuf>,
    /// The settings page, while it is open in place of the card.
    page: Option<SettingsView>,
    /// Whether the saved shortcut is registered; another program may hold it.
    hotkey_active: bool,
    report_paint: Option<Instant>,
}

/// Registers the lookup shortcut written as in the settings file. An empty one is off,
/// which counts as success. Until capture lands in N3 this is a plain `RegisterHotKey`.
fn register_hotkey(hwnd: HWND, text: &str) -> bool {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), SHOW_HOTKEY);
        match shortcut::parse(text) {
            Ok(Some(hotkey)) => RegisterHotKey(
                Some(hwnd),
                SHOW_HOTKEY,
                HOT_KEY_MODIFIERS(hotkey.modifiers) | MOD_NOREPEAT,
                hotkey.key,
            )
            .is_ok(),
            Ok(None) => true,
            Err(_) => false,
        }
    }
}

fn flag(name: &str) -> bool {
    std::env::args().any(|arg| arg == name)
}

fn option(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn personalize(value: windows::core::PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            value,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

/// Uses the undocumented composition attribute because it keeps the blur while the card
/// doesn't have focus. Anything short of a successful call leaves the card opaque.
unsafe fn enable_glass(hwnd: HWND) -> bool {
    if flag("--opaque") || personalize(w!("EnableTransparency")) == Some(0) {
        return false;
    }
    #[repr(C)]
    struct Accent {
        state: u32,
        flags: u32,
        gradient: u32,
        animation: u32,
    }
    #[repr(C)]
    struct Data {
        attribute: u32,
        data: *mut Accent,
        size: usize,
    }
    type SetAttribute = unsafe extern "system" fn(HWND, *mut Data) -> BOOL;
    let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else {
        return false;
    };
    let Some(address) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else {
        return false;
    };
    let set: SetAttribute = std::mem::transmute(address);
    // 4 = ACCENT_ENABLE_ACRYLICBLURBEHIND, 19 = WCA_ACCENT_POLICY.
    let mut accent = Accent {
        state: 4,
        flags: 0,
        gradient: 0x0100_0000,
        animation: 0,
    };
    let mut data = Data {
        attribute: 19,
        data: &mut accent,
        size: size_of::<Accent>(),
    };
    if !set(hwnd, &mut data).as_bool() {
        return false;
    }
    let margins = MARGINS {
        cxLeftWidth: -1,
        cxRightWidth: -1,
        cyTopHeight: -1,
        cyBottomHeight: -1,
    };
    let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
    true
}

impl App {
    fn theme(&self) -> &'static Theme {
        if self.dark {
            &DARK
        } else {
            &LIGHT
        }
    }

    fn refresh_theme(&mut self) {
        self.dark = match self.choice {
            ThemeChoice::Dark => true,
            ThemeChoice::Light => false,
            ThemeChoice::System => personalize(w!("AppsUseLightTheme")) == Some(0),
        };
        let dark = BOOL::from(self.dark);
        unsafe {
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const _ as _,
                size_of::<BOOL>() as u32,
            );
        }
    }

    /// Rebuilds the layout. The window follows on the next `fit`, outside this borrow.
    fn rebuild(&mut self) {
        let frame = match &self.page {
            Some(page) => render::build_settings(&self.dwrite, page, &self.settings),
            None => render::build(&self.dwrite, &self.content, &self.query, self.pinned),
        };
        if let Ok(frame) = frame {
            self.frame = frame;
        }
        self.scroll = self.scroll.clamp(0.0, self.frame.max_scroll());
    }

    fn fit(&self) {
        let scale = unsafe { GetDpiForWindow(self.hwnd) } as f32 / 96.0;
        let width = (WIDTH * scale).ceil() as i32;
        let height = (self.frame.height() * scale).ceil() as i32;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn ensure_target(&mut self) -> Result<()> {
        unsafe {
            let mut client = RECT::default();
            GetClientRect(self.hwnd, &mut client)?;
            let size = D2D_SIZE_U {
                width: client.right.max(1) as u32,
                height: client.bottom.max(1) as u32,
            };
            if let Some(target) = &self.target {
                // WM_SIZE can arrive while `fit` holds the app, so match the client area here.
                return target.Resize(&size);
            }
            let dpi = GetDpiForWindow(self.hwnd) as f32;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: if flag("--gpu") {
                    D2D1_RENDER_TARGET_TYPE_DEFAULT
                } else {
                    D2D1_RENDER_TARGET_TYPE_SOFTWARE
                },
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: if self.glass {
                        D2D1_ALPHA_MODE_PREMULTIPLIED
                    } else {
                        D2D1_ALPHA_MODE_IGNORE
                    },
                },
                dpiX: dpi,
                dpiY: dpi,
                ..Default::default()
            };
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: self.hwnd,
                pixelSize: size,
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let target = self.d2d.CreateHwndRenderTarget(&props, &hwnd_props)?;
            // Glass rules out ClearType, and grayscale text comes out thin; more contrast
            // gives the strokes back their weight.
            if self.glass {
                if let Ok(factory) = self.dwrite.cast::<IDWriteFactory1>() {
                    let defaults = self.dwrite.CreateRenderingParams()?;
                    if let Ok(params) = factory.CreateCustomRenderingParams(
                        defaults.GetGamma(),
                        defaults.GetEnhancedContrast(),
                        GLASS_TEXT_CONTRAST,
                        0.0,
                        DWRITE_PIXEL_GEOMETRY_FLAT,
                        DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                    ) {
                        target.SetTextRenderingParams(&params);
                    }
                }
            }
            self.target = Some(target);
        }
        Ok(())
    }

    fn paint(&mut self) -> Result<()> {
        self.ensure_target()?;
        let target = self.target.clone().unwrap();
        let drawn = render::paint(render::Paint {
            target: &target,
            theme: self.theme(),
            glass: self.glass,
            frame: &self.frame,
            scroll: self.scroll,
            mouse: self.mouse,
        });
        if let Err(error) = drawn {
            if error.code() != D2DERR_RECREATE_TARGET {
                return Err(error);
            }
            self.target = None;
        }
        if let Some(start) = self.report_paint.take() {
            println!(
                "first_paint_ms={:.1}",
                start.elapsed().as_secs_f64() * 1000.0
            );
            unsafe { PostQuitMessage(0) };
        }
        Ok(())
    }

    fn show_failure(&mut self, failure: FailureView) {
        self.content = Content::Failure(failure);
        self.scroll = 0.0;
        self.rebuild();
    }

    fn submit(&mut self, refresh: bool) {
        let Some(word) = selected_word(&self.query) else {
            self.show_failure(FailureView::invalid_input());
            return;
        };
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.stop_sound();
        self.clips.clear();
        self.serial += 1;
        let serial = self.serial;
        self.last_word = word.clone();
        self.content = Content::Loading;
        self.scroll = 0.0;
        self.rebuild();
        let (runtime, service) = runtime();
        let service = service.clone();
        let hwnd = self.hwnd.0 as isize;
        self.task = Some(runtime.spawn(async move {
            let result = service.lookup(&word, refresh).await;
            *ANSWER.lock().unwrap() = Some((serial, word, result));
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_RESULT, WPARAM(0), LPARAM(0));
            }
        }));
    }

    fn receive(&mut self) {
        let Some((serial, word, result)) = ANSWER.lock().unwrap().take() else {
            return;
        };
        if serial != self.serial {
            return;
        }
        self.task = None;
        self.scroll = 0.0;
        self.content = match result {
            Ok(entry) => Content::Entry(Box::new(EntryView::new(entry))),
            Err(error) => Content::Failure(FailureView::from_error(&error, &word)),
        };
        self.rebuild();
    }

    /// Stops loading or playing. The views keep any failure so it stays visible.
    fn stop_sound(&mut self) {
        if let Some(task) = self.sound_task.take() {
            task.abort();
        }
        self.player = None;
        self.sound = None;
        if let Content::Entry(view) = &mut self.content {
            view.stop_sounds();
        }
    }

    /// Clicking the chip that is loading or playing stops it; any other chip starts.
    fn toggle_sound(&mut self, index: usize) {
        let Content::Entry(view) = &self.content else {
            return;
        };
        let Some(url) = view.sound_url(index).map(str::to_owned) else {
            return;
        };
        let active = self.sound.is_some_and(|(_, chip)| chip == index);
        self.stop_sound();
        if !active {
            self.start_sound(index, url);
        }
        self.rebuild();
    }

    fn start_sound(&mut self, index: usize, url: String) {
        self.sound_serial += 1;
        let serial = self.sound_serial;
        self.sound = Some((serial, index));
        if let Content::Entry(view) = &mut self.content {
            view.set_sound(index, Sound::Loading);
        }
        // `--sound <file>` plays a local file in place of every pronunciation.
        let local = option("--sound").map(|path| std::fs::read(path).map_err(|_| ()));
        if let Some(clip) = local.or_else(|| self.clips.get(&url).cloned().map(Ok)) {
            *CLIP.lock().unwrap() = Some((serial, url, clip));
            self.receive_clip();
            return;
        }
        let hwnd = self.hwnd.0 as isize;
        self.sound_task = Some(runtime().0.spawn(async move {
            let clip = audio::fetch(&url).await;
            *CLIP.lock().unwrap() = Some((serial, url, clip));
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLIP, WPARAM(0), LPARAM(0));
            }
        }));
    }

    fn receive_clip(&mut self) {
        let Some((serial, url, clip)) = CLIP.lock().unwrap().take() else {
            return;
        };
        let Some((current, index)) = self.sound.filter(|(current, _)| *current == serial) else {
            return;
        };
        self.sound_task = None;
        let name = option("--sound").unwrap_or_else(|| audio::clip_name(&url).to_owned());
        let played = clip.ok().and_then(|clip| {
            let player = Player::play(self.hwnd, WM_SOUND, current as usize, &name, &clip).ok();
            self.clips.insert(url, clip);
            player
        });
        let sound = if played.is_some() {
            Sound::Playing
        } else {
            self.sound = None;
            Sound::Failed
        };
        self.player = played;
        if let Content::Entry(view) = &mut self.content {
            view.set_sound(index, sound);
        }
        self.rebuild();
    }

    fn sound_ended(&mut self, serial: u64, failed: bool) {
        let Some((_, index)) = self.sound.filter(|(current, _)| *current == serial) else {
            return;
        };
        self.player = None;
        self.sound = None;
        if let Content::Entry(view) = &mut self.content {
            view.set_sound(index, if failed { Sound::Failed } else { Sound::Idle });
        }
        self.rebuild();
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::Sound(index) => self.toggle_sound(index),
            Action::OpenSettings => self.open_settings(),
            Action::Settings(row, change) => {
                if let Some(page) = &mut self.page {
                    // A click elsewhere while recording leaves the old shortcut in place.
                    if page.recording {
                        self.stop_recording();
                    }
                }
                if let Some(page) = &mut self.page {
                    page.row = row;
                }
                self.change(change);
            }
            Action::Pin => {
                self.pinned = !self.pinned;
                self.rebuild();
            }
            Action::Tab(tab) => {
                // The readings change with the tab, so whatever plays belongs to the old one.
                self.stop_sound();
                if let Content::Entry(view) = &mut self.content {
                    view.select_tab(tab);
                    self.scroll = 0.0;
                    self.rebuild();
                }
            }
            Action::Sense(index) => {
                if let Content::Entry(view) = &mut self.content {
                    view.click_sense(index);
                    self.rebuild();
                    self.scroll = self.frame.scroll_to_selected(self.scroll);
                }
            }
            Action::Suggestion(word) => {
                self.query = word;
                self.submit(false);
            }
            Action::Retry => {
                self.query = self.last_word.clone();
                self.submit(true);
            }
            Action::Open(url) => {
                if allowed_link(&url) {
                    unsafe {
                        ShellExecuteW(
                            None,
                            w!("open"),
                            &HSTRING::from(url),
                            None,
                            None,
                            SW_SHOWNORMAL,
                        );
                    }
                }
            }
        }
    }

    fn key(&mut self, key: u16) {
        if let Some(page) = &self.page {
            if page.recording {
                self.record_key(key);
            } else {
                self.settings_key(key);
            }
            return;
        }
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        if ctrl && key == VK_OEM_COMMA.0 {
            self.open_settings();
        } else if key == VK_RETURN.0 {
            self.submit(false);
        } else if key == VK_ESCAPE.0 {
            self.hide();
        } else if ctrl && key == u16::from(b'V') {
            self.paste();
        } else if ctrl && key == VK_BACK.0 {
            self.query.clear();
            self.rebuild();
        } else if ctrl && key == u16::from(b'P') {
            let next = match &self.content {
                Content::Entry(view) => view.next_sound(),
                _ => None,
            };
            if let Some(index) = next {
                self.stop_sound();
                if let Content::Entry(view) = &self.content {
                    let url = view.sound_url(index).unwrap_or_default().to_owned();
                    self.start_sound(index, url);
                }
                self.rebuild();
            }
        } else if key == VK_TAB.0 {
            // The readings change with the tab, so whatever plays belongs to the old one.
            self.stop_sound();
            if let Content::Entry(view) = &mut self.content {
                view.next_tab();
                self.scroll = 0.0;
                self.rebuild();
            }
        } else if let Content::Entry(view) = &mut self.content {
            if key == VK_DOWN.0 || key == VK_UP.0 {
                view.move_selection(if key == VK_DOWN.0 { 1 } else { -1 });
            } else if key == VK_RIGHT.0 || key == VK_LEFT.0 {
                view.set_expanded(key == VK_RIGHT.0);
            } else {
                return;
            }
            self.rebuild();
            self.scroll = self.frame.scroll_to_selected(self.scroll);
        }
    }

    fn paste(&mut self) {
        let Some(text) = read_clipboard_text() else {
            return;
        };
        let line = text.lines().next().unwrap_or("").trim();
        self.query.extend(line.chars().take(100));
        self.rebuild();
    }

    /// Hiding gives everything back: no render target, no layouts, trimmed working set.
    fn hide(&mut self) {
        // The next show is the lookup card again.
        if self.page.is_some() {
            self.close_settings();
        }
        self.stop_sound();
        self.mouse = None;
        self.target = None;
        self.frame = Frame::default();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
        }
    }

    /// The right-click menu for the point `at` (screen pixels), or for the selected row
    /// when it came from the keyboard. Empty on the settings page.
    fn context_menu(&self, at: Option<POINT>) -> Vec<(String, Option<Pick>)> {
        if self.page.is_some() {
            return Vec::new();
        }
        let mut items: Vec<(String, Option<Pick>)> = Vec::new();
        let mut source = None;
        if let Content::Entry(view) = &self.content {
            let word = view.entry.word.clone();
            items.push((format!("复制“{word}”"), Some(Pick::Copy(word))));
            let row = match at {
                None => Some(view.selected),
                Some(mut point) => unsafe {
                    let _ = ScreenToClient(self.hwnd, &mut point);
                    let scale = GetDpiForWindow(self.hwnd) as f32 / 96.0;
                    match self.frame.hit(
                        point.x as f32 / scale,
                        point.y as f32 / scale,
                        self.scroll,
                    ) {
                        Some(Action::Sense(index)) => Some(index),
                        _ => None,
                    }
                },
            };
            if let Some(definition) = row.and_then(|index| view.visible().get(index).copied()) {
                items.push((
                    "复制这条释义".into(),
                    Some(Pick::Copy(sense_text(definition))),
                ));
            }
            source = Some(view.entry.source_url.clone());
        } else if let Content::Failure(failure) = &self.content {
            source = failure.source_url.clone();
        }
        if let Some(url) = source.filter(|url| allowed_link(url)) {
            items.push((String::new(), None));
            items.push(("在 Cambridge 打开".into(), Some(Pick::Open(url))));
        }
        if !items.is_empty() {
            items.push((String::new(), None));
        }
        items.push(("设置\tCtrl+,".into(), Some(Pick::Settings)));
        items
    }

    fn pick(&mut self, pick: Pick) {
        match pick {
            Pick::Copy(text) => {
                write_clipboard_text(self.hwnd, &text);
            }
            Pick::Open(url) => self.act(Action::Open(url)),
            Pick::Settings => self.open_settings(),
        }
    }

    /// Every change goes through here and only takes once the file is saved, so what the
    /// card and the tray show is always what the next start will read.
    fn save(&mut self, next: Settings) -> bool {
        let saved = self
            .settings_path
            .as_deref()
            .is_some_and(|path| settings::save(path, &next).is_ok());
        if saved {
            self.settings = next;
        } else if let Some(page) = &mut self.page {
            page.notice = Some("设置未能保存，请稍后再试。".into());
        }
        saved
    }

    fn toggle_auto(&mut self) {
        let mut next = self.settings.clone();
        next.enabled = !next.enabled;
        if self.save(next) {
            tray::update(self.hwnd, self.settings.enabled);
        }
    }

    fn set_theme(&mut self, theme: &str) {
        let mut next = self.settings.clone();
        next.theme = theme.to_owned();
        if self.save(next) {
            self.choice = match theme {
                "light" => ThemeChoice::Light,
                "dark" => ThemeChoice::Dark,
                _ => ThemeChoice::System,
            };
            self.refresh_theme();
        }
    }

    fn open_settings(&mut self) {
        let notice = (!self.hotkey_active && !self.settings.lookup_hotkey.is_empty())
            .then(|| shortcut_notice("占用").to_owned());
        self.page = Some(SettingsView {
            notice,
            ..Default::default()
        });
        self.scroll = 0.0;
        self.rebuild();
    }

    fn close_settings(&mut self) {
        if self.page.as_ref().is_some_and(|page| page.recording) {
            self.stop_recording();
        }
        self.page = None;
        self.scroll = 0.0;
        self.rebuild();
    }

    fn change(&mut self, change: SettingsChange) {
        match change {
            SettingsChange::Nothing => {}
            SettingsChange::Close => self.close_settings(),
            SettingsChange::ToggleAuto => self.toggle_auto(),
            SettingsChange::Theme(theme) => self.set_theme(theme),
            SettingsChange::StopRecording => self.stop_recording(),
            SettingsChange::Record => {
                // Off while recording, so pressing the current combination records it too.
                unsafe {
                    let _ = UnregisterHotKey(Some(self.hwnd), SHOW_HOTKEY);
                }
                if let Some(page) = &mut self.page {
                    page.recording = true;
                    page.notice = None;
                }
            }
        }
        if self.page.is_some() {
            self.rebuild();
        }
    }

    /// Back to the shortcut that was saved.
    fn stop_recording(&mut self) {
        self.hotkey_active = register_hotkey(self.hwnd, &self.settings.lookup_hotkey);
        if let Some(page) = &mut self.page {
            page.recording = false;
        }
    }

    /// A key while recording. The new shortcut replaces the old one only once it is both
    /// registered and saved; otherwise the old one stays and the reason shows.
    fn record_key(&mut self, key: u16) {
        let down = |vk: VIRTUAL_KEY| unsafe { GetKeyState(vk.0 as i32) } < 0;
        let (ctrl, alt, shift) = (down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT));
        let text = if key == VK_ESCAPE.0 {
            self.stop_recording();
            self.rebuild();
            return;
        } else if key == VK_BACK.0 && !ctrl && !alt && !shift {
            String::new()
        } else {
            let Some(combo) = pressed_combo(ctrl, alt, shift, key) else {
                return;
            };
            match shortcut::parse(&combo) {
                Ok(Some(hotkey)) => hotkey.text,
                Ok(None) => String::new(),
                Err(error) => {
                    if let Some(page) = &mut self.page {
                        page.notice = Some(shortcut_notice(&error).to_owned());
                    }
                    self.rebuild();
                    return;
                }
            }
        };
        let notice = if !register_hotkey(self.hwnd, &text) {
            Some(shortcut_notice("占用"))
        } else {
            let mut next = self.settings.clone();
            next.lookup_hotkey = text;
            if self.save(next) {
                None
            } else {
                Some("快捷键未能保存，请稍后再试。")
            }
        };
        self.stop_recording();
        if let Some(page) = &mut self.page {
            page.notice = notice.map(str::to_owned);
        }
        self.rebuild();
    }

    fn settings_key(&mut self, key: u16) {
        let key = match VIRTUAL_KEY(key) {
            VK_UP => SettingsKey::Up,
            VK_DOWN => SettingsKey::Down,
            VK_LEFT => SettingsKey::Left,
            VK_RIGHT => SettingsKey::Right,
            VK_RETURN | VK_SPACE => SettingsKey::Enter,
            VK_ESCAPE => SettingsKey::Escape,
            _ => return,
        };
        let theme = self.settings.theme.clone();
        let Some(page) = &mut self.page else {
            return;
        };
        let change = page.key(key, &theme);
        self.change(change);
        self.rebuild();
    }

    fn show(&mut self) {
        self.rebuild();
        self.fit();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
    }
}

/// What a right-click menu item does; the text is taken when the menu opens.
#[derive(Clone)]
enum Pick {
    Copy(String),
    Open(String),
    Settings,
}

fn write_clipboard_text(hwnd: HWND, text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let written = (|| {
            EmptyClipboard().ok()?;
            let global = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
            let data = GlobalLock(global) as *mut u16;
            if data.is_null() {
                let _ = GlobalFree(Some(global));
                return None;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), data, wide.len());
            let _ = GlobalUnlock(global);
            // On success the clipboard owns the memory.
            if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(global.0))).is_err() {
                let _ = GlobalFree(Some(global));
                return None;
            }
            Some(())
        })();
        let _ = CloseClipboard();
        written.is_some()
    }
}

fn read_clipboard_text() -> Option<String> {
    unsafe {
        OpenClipboard(None).ok()?;
        let text = (|| {
            let handle = GetClipboardData(CF_UNICODETEXT).ok()?;
            let global = HGLOBAL(handle.0);
            let data = GlobalLock(global) as *const u16;
            if data.is_null() {
                return None;
            }
            let length = (0..4096).take_while(|&i| *data.add(i) != 0).count();
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(data, length));
            let _ = GlobalUnlock(global);
            Some(text)
        })();
        let _ = CloseClipboard();
        text
    }
}

/// Returns false when a message arrives while the app is already borrowed.
fn with_app(run: impl FnOnce(&mut App)) -> bool {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut app) => {
            if let Some(app) = app.as_mut() {
                run(app);
            }
            true
        }
        Err(_) => false,
    })
}

fn scaled_point(hwnd: HWND, lparam: LPARAM) -> (f32, f32) {
    let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
    let x = (lparam.0 & 0xffff) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
    (x / scale, y / scale)
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Layout changes may resize the window, and resizing re-enters this procedure, so the
/// resize runs after the state update has released the app.
fn update_and_fit(run: impl FnOnce(&mut App)) {
    with_app(run);
    with_app(|app| app.fit());
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
                with_app(|app| {
                    let _ = app.paint();
                });
                LRESULT(0)
            }
            WM_SIZE => {
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let bounds = &*(lparam.0 as *const RECT);
                with_app(|app| {
                    let dpi = (wparam.0 & 0xffff) as f32;
                    if let Some(target) = &app.target {
                        target.SetDpi(dpi, dpi);
                    }
                });
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    bounds.left,
                    bounds.top,
                    bounds.right - bounds.left,
                    bounds.bottom - bounds.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                with_app(|app| app.fit());
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_NCHITTEST => {
                let mut point = POINT {
                    x: (lparam.0 & 0xffff) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
                };
                let _ = ScreenToClient(hwnd, &mut point);
                let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
                let (x, y) = (point.x as f32 / scale, point.y as f32 / scale);
                let mut over_action = false;
                with_app(|app| over_action = app.frame.hit(x, y, app.scroll).is_some());
                let drag = y < INPUT_H && !over_action;
                LRESULT(if drag {
                    HTCAPTION as isize
                } else {
                    HTCLIENT as isize
                })
            }
            WM_SETCURSOR => {
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                let _ = ScreenToClient(hwnd, &mut point);
                let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
                let mut over_action = false;
                with_app(|app| {
                    over_action = app
                        .frame
                        .hit(point.x as f32 / scale, point.y as f32 / scale, app.scroll)
                        .is_some()
                });
                if let Ok(cursor) =
                    LoadCursorW(None, if over_action { IDC_HAND } else { IDC_ARROW })
                {
                    SetCursor(Some(cursor));
                }
                LRESULT(1)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = scaled_point(hwnd, lparam);
                update_and_fit(|app| {
                    if let Some(action) = app.frame.hit(x, y, app.scroll) {
                        app.act(action);
                    }
                });
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let point = scaled_point(hwnd, lparam);
                let mut entered = false;
                with_app(|app| {
                    entered = app.mouse.is_none();
                    app.mouse = Some(point);
                });
                if entered {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut track);
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with_app(|app| app.mouse = None);
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xffff) as i16 as f32;
                with_app(|app| {
                    app.scroll =
                        (app.scroll - delta / 120.0 * 48.0).clamp(0.0, app.frame.max_scroll())
                });
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_CHAR => {
                let c = char::from_u32(wparam.0 as u32).unwrap_or('\0');
                with_app(|app| {
                    if app.page.is_some() {
                        return;
                    }
                    if c == '\u{8}' {
                        app.query.pop();
                    } else if !c.is_control() && app.query.chars().count() < 100 {
                        app.query.push(c);
                    } else {
                        return;
                    }
                    app.rebuild();
                });
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let key = wparam.0 as u16;
                update_and_fit(|app| app.key(key));
                LRESULT(0)
            }
            WM_CONTEXTMENU => {
                // From the keyboard (menu key, Shift+F10) lparam is -1: use the selected row,
                // and open the menu under the input row.
                let keyboard = lparam.0 as i32 == -1;
                let at = if keyboard {
                    let mut corner = POINT { x: 24, y: 60 };
                    let _ = ClientToScreen(hwnd, &mut corner);
                    corner
                } else {
                    POINT {
                        x: (lparam.0 & 0xffff) as i16 as i32,
                        y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
                    }
                };
                let mut entries = Vec::new();
                with_app(|app| entries = app.context_menu((!keyboard).then_some(at)));
                if entries.is_empty() {
                    return LRESULT(0);
                }
                let items: Vec<menu::Item> = entries
                    .iter()
                    .map(|(label, pick)| match pick {
                        Some(_) => menu::Item::command(label),
                        None => menu::Item::Separator,
                    })
                    .collect();
                // The menu runs its own message loop, so the app is not borrowed here.
                if let Some(pick) =
                    menu::popup(hwnd, &items, at).and_then(|index| entries[index].1.clone())
                {
                    update_and_fit(|app| app.pick(pick));
                }
                LRESULT(0)
            }
            // Alt combinations arrive as system keys; only recording a shortcut wants them.
            WM_SYSKEYDOWN | WM_SYSCHAR => {
                let mut recording = false;
                with_app(|app| recording = app.page.as_ref().is_some_and(|page| page.recording));
                if !recording {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                if msg == WM_SYSKEYDOWN {
                    let key = wparam.0 as u16;
                    update_and_fit(|app| app.key(key));
                }
                LRESULT(0)
            }
            WM_ACTIVATE => {
                if wparam.0 & 0xffff == WA_INACTIVE as usize {
                    with_app(|app| {
                        if !app.pinned && !flag("--stay") {
                            app.hide();
                        }
                    });
                }
                LRESULT(0)
            }
            WM_HOTKEY if wparam.0 as i32 == SHOW_HOTKEY => {
                with_app(|app| app.show());
                LRESULT(0)
            }
            WM_SHOW => {
                with_app(|app| app.show());
                LRESULT(0)
            }
            WM_TRAY => {
                match (lparam.0 as u32) & 0xffff {
                    WM_LBUTTONUP => {
                        with_app(|app| app.show());
                    }
                    WM_RBUTTONUP => {
                        let (mut enabled, mut hotkey) = (false, String::new());
                        with_app(|app| {
                            enabled = app.settings.enabled;
                            if app.hotkey_active {
                                hotkey = app.settings.lookup_hotkey.clone();
                            }
                        });
                        // The menu runs its own message loop, so the app is not borrowed here.
                        match tray::menu(hwnd, enabled, &hotkey) {
                            Some(Command::ToggleAuto) => {
                                with_app(|app| app.toggle_auto());
                            }
                            Some(Command::Lookup) => {
                                with_app(|app| app.show());
                            }
                            Some(Command::Quit) => {
                                let _ = DestroyWindow(hwnd);
                            }
                            Some(Command::Settings) => {
                                update_and_fit(|app| {
                                    app.open_settings();
                                    app.show();
                                });
                            }
                            None => {}
                        }
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                let mut enabled = false;
                with_app(|app| enabled = app.settings.enabled);
                tray::add(hwnd, enabled);
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                update_and_fit(|app| {
                    app.refresh_theme();
                    app.rebuild();
                });
                LRESULT(0)
            }
            WM_RESULT => {
                update_and_fit(|app| app.receive());
                LRESULT(0)
            }
            WM_CLIP => {
                update_and_fit(|app| app.receive_clip());
                LRESULT(0)
            }
            WM_SOUND => {
                update_and_fit(|app| app.sound_ended(wparam.0 as u64, lparam.0 != 0));
                LRESULT(0)
            }
            WM_DESTROY => {
                tray::remove(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// `--payload` and `--parse` show saved data, for development without a network.
fn initial_content() -> Content {
    if let Some(path) = option("--parse") {
        let html = std::fs::read_to_string(path).unwrap_or_default();
        let payload = match dictionary::parser::parse_page(&html) {
            Ok(entry) => json!({"entry":entry}),
            Err(reason) => {
                json!({"entry":null,"apiError":reason.message,"apiStatus":reason.status,"apiCode":reason.code})
            }
        };
        return content_from(&payload);
    }
    if let Some(path) = option("--payload") {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        return content_from(&serde_json::from_str(&text).unwrap_or(json!({})));
    }
    Content::Empty
}

fn content_from(payload: &serde_json::Value) -> Content {
    let word = payload["entry"]["word"].as_str().unwrap_or("").to_owned();
    match read_api_entry(payload, &word, "en-cn") {
        Ok(entry) => Content::Entry(Box::new(EntryView::new(entry))),
        Err(error) => Content::Failure(FailureView::from_error(&error, &word)),
    }
}

enum Instance {
    /// This copy runs; the event is signalled whenever the app is launched again.
    First(HANDLE),
    /// Another copy runs and has been asked to show the card.
    Again,
    /// A development run, which may sit beside the real app.
    Development,
}

/// A named event rather than a window search, which fails across sandboxes and desktops.
fn claim_instance() -> Instance {
    unsafe {
        let Ok(event) = CreateEventW(None, false, false, INSTANCE) else {
            return Instance::Development;
        };
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return Instance::First(event);
        }
        // This launch came from the user, so it may hand the foreground to the running copy.
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        let _ = SetEvent(event);
        Instance::Again
    }
}

/// Waits on the event for the life of the process and asks the window to show the card.
fn wait_for_launches(event: HANDLE, hwnd: HWND) {
    let (event, hwnd) = (event.0 as isize, hwnd.0 as isize);
    std::thread::spawn(move || loop {
        unsafe {
            if WaitForSingleObject(HANDLE(event as *mut _), INFINITE) != WAIT_OBJECT_0 {
                break;
            }
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_SHOW, WPARAM(0), LPARAM(0));
        }
    });
}

pub fn run() -> Result<()> {
    let start = Instant::now();
    // Development runs show saved data and may sit beside the real app.
    let development = ["--payload", "--parse", "--first-paint"]
        .into_iter()
        .any(flag);
    let launch = if development {
        Instance::Development
    } else {
        claim_instance()
    };
    if let Instance::Again = launch {
        return Ok(());
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(None)?;
        let class = CLASS;
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class,
            w!("WordSide"),
            WS_POPUP,
            GetSystemMetrics(SM_CXSCREEN) / 3,
            GetSystemMetrics(SM_CYSCREEN) / 10,
            400,
            120,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        // The box only takes English, and an active IME would swallow Enter as VK_PROCESSKEY.
        let _ = ImmAssociateContext(hwnd, HIMC::default());
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as _,
            size_of_val(&corner) as u32,
        );
        let glass = enable_glass(hwnd);
        let settings_path = settings::path();
        let preferences = settings_path
            .as_deref()
            .map(settings::load)
            .unwrap_or_default();
        let hotkey_active = register_hotkey(hwnd, &preferences.lookup_hotkey);
        TASKBAR_CREATED.store(tray::taskbar_created(), Ordering::Relaxed);
        tray::add(hwnd, preferences.enabled);
        if let Instance::First(event) = launch {
            wait_for_launches(event, hwnd);
        }
        let choice = if flag("--light") {
            ThemeChoice::Light
        } else if flag("--dark") {
            ThemeChoice::Dark
        } else {
            match preferences.theme.as_str() {
                "light" => ThemeChoice::Light,
                "dark" => ThemeChoice::Dark,
                _ => ThemeChoice::System,
            }
        };
        let mut app = App {
            hwnd,
            d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
            dwrite: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
            target: None,
            frame: Frame::default(),
            content: initial_content(),
            query: String::new(),
            scroll: 0.0,
            mouse: None,
            pinned: false,
            choice,
            dark: true,
            glass,
            serial: 0,
            last_word: String::new(),
            task: None,
            sound: None,
            sound_serial: 0,
            sound_task: None,
            player: None,
            clips: HashMap::new(),
            settings: preferences,
            settings_path,
            page: None,
            hotkey_active,
            report_paint: flag("--first-paint").then_some(start),
        };
        app.refresh_theme();
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        // A normal start waits in the tray; the shortcut, the tray or another launch shows
        // the card. Development runs show their data right away.
        if development {
            with_app(|app| app.show());
        }
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).into() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        // `--first-paint` quits without destroying the window; don't leave a dead icon behind.
        tray::remove(hwnd);
    }
    Ok(())
}
