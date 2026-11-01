//! The window, the message loop and everything that reacts to input.
use crate::{
    card::{allowed_link, Content, EntryView, FailureView},
    dictionary,
    entry::{read_api_entry, Entry, LookupError},
    render::{self, Action, Frame, Theme, DARK, INPUT_H, LIGHT, WIDTH},
    selection_input::selected_word,
    service::Service,
};
use serde_json::json;
use std::{
    cell::RefCell,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};
use windows::{
    core::{s, w, Result, BOOL, HSTRING},
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
            DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            Memory::{GlobalLock, GlobalUnlock},
            Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
            Threading::*,
        },
        UI::{
            Controls::MARGINS,
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
const CF_UNICODETEXT: u32 = 13;
const SHOW_HOTKEY: i32 = 1;

type Answer = (u64, String, Result2);
type Result2 = std::result::Result<Entry, LookupError>;

thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; }
static ANSWER: Mutex<Option<Answer>> = Mutex::new(None);
static RUNTIME: OnceLock<(tokio::runtime::Runtime, Arc<Service>)> = OnceLock::new();

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
    pinned: bool,
    choice: ThemeChoice,
    dark: bool,
    glass: bool,
    serial: u64,
    last_word: String,
    task: Option<tokio::task::JoinHandle<()>>,
    report_paint: Option<Instant>,
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
        if let Ok(frame) = render::build(&self.dwrite, &self.content, &self.query, self.pinned) {
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
            self.target = Some(self.d2d.CreateHwndRenderTarget(&props, &hwnd_props)?);
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
        self.serial += 1;
        let serial = self.serial;
        self.last_word = word.clone();
        self.content = Content::Loading;
        self.scroll = 0.0;
        self.rebuild();
        // Created on the first lookup, so a card that was never used costs nothing here.
        let (runtime, service) = RUNTIME.get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            (runtime, Arc::new(Service::default()))
        });
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

    fn act(&mut self, action: Action) {
        match action {
            Action::Pin => {
                self.pinned = !self.pinned;
                self.rebuild();
            }
            Action::Tab(tab) => {
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
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        if key == VK_RETURN.0 {
            self.submit(false);
        } else if key == VK_ESCAPE.0 {
            self.hide();
        } else if ctrl && key == u16::from(b'V') {
            self.paste();
        } else if ctrl && key == VK_BACK.0 {
            self.query.clear();
            self.rebuild();
        } else if let Content::Entry(view) = &mut self.content {
            if key == VK_DOWN.0 || key == VK_UP.0 {
                view.move_selection(if key == VK_DOWN.0 { 1 } else { -1 });
            } else if key == VK_RIGHT.0 || key == VK_LEFT.0 {
                view.set_expanded(key == VK_RIGHT.0);
            } else if key == VK_TAB.0 {
                view.next_tab();
                self.scroll = 0.0;
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
        self.target = None;
        self.frame = Frame::default();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
        }
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
            WM_DESTROY => {
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

pub fn run() -> Result<()> {
    let start = Instant::now();
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(None)?;
        let class = w!("WordSideCard");
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
        // Temporary until capture lands in N3, which owns the real shortcut.
        let _ = RegisterHotKey(
            Some(hwnd),
            SHOW_HOTKEY,
            MOD_ALT | MOD_CONTROL,
            u32::from(b'A'),
        );
        let choice = if flag("--light") {
            ThemeChoice::Light
        } else if flag("--dark") {
            ThemeChoice::Dark
        } else {
            ThemeChoice::System
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
            pinned: false,
            choice,
            dark: true,
            glass,
            serial: 0,
            last_word: String::new(),
            task: None,
            report_paint: flag("--first-paint").then_some(start),
        };
        app.refresh_theme();
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        with_app(|app| app.show());
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).into() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}
