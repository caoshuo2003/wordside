//! Lays the card out with DirectWrite and draws it with Direct2D.
use crate::card::{Content, Coverage, SettingsChange, SettingsView, Sound, THEMES};
use crate::presentation::{source_label, PronunciationChip};
use crate::settings::Settings;
use std::sync::OnceLock;
use windows::{
    core::{w, Result, BOOL, PCWSTR},
    Win32::Graphics::{
        Direct2D::{Common::*, *},
        DirectWrite::*,
    },
};
use windows_numerics::{Matrix3x2, Vector2};

pub const WIDTH: f32 = 400.0;
const PAD: f32 = 20.0;
pub const INPUT_H: f32 = 52.0;
const FOOT_H: f32 = 36.0;
/// Definitions and examples, from their left edge to the card's right padding.
const TEXT_X: f32 = 26.0;
/// The whole card stops growing here; the body scrolls instead.
const MAX_CARD: f32 = 420.0;
/// A tall header (many pronunciations) still leaves this much list in view.
const MIN_BODY: f32 = 120.0;
const FADE_H: f32 = 36.0;
const GLASS_OPACITY: f32 = 0.75;
const ICON_FONT: PCWSTR = w!("Segoe MDL2 Assets");
/// Stands for Segoe UI Variable Text where Windows 11 has it, else Segoe UI. Chinese falls
/// back to Microsoft YaHei UI through the zh-CN locale.
const TEXT_FONT: PCWSTR = w!("Segoe UI");

fn text_family(dwrite: &IDWriteFactory) -> PCWSTR {
    static VARIABLE: OnceLock<bool> = OnceLock::new();
    let variable = *VARIABLE.get_or_init(|| unsafe {
        let mut collection = None;
        if dwrite
            .GetSystemFontCollection(&mut collection, false)
            .is_err()
        {
            return false;
        }
        let Some(collection) = collection else {
            return false;
        };
        let (mut index, mut exists) = (0, BOOL(0));
        collection
            .FindFamilyName(w!("Segoe UI Variable Text"), &mut index, &mut exists)
            .is_ok()
            && exists.as_bool()
    });
    if variable {
        w!("Segoe UI Variable Text")
    } else {
        TEXT_FONT
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct Rgba(u32, f32);

pub struct Theme {
    fill: u32,
    ink: u32,
    ink2: u32,
    muted: u32,
    accent: u32,
    on_accent: u32,
    /// A change that didn't take, such as a shortcut another program holds.
    warn: u32,
    row: Rgba,
    line: Rgba,
    /// The raised tile under the mouse, its highlight edge and the shadow under it.
    raised: Rgba,
    edge: Rgba,
    shadow: Rgba,
}

pub const DARK: Theme = Theme {
    fill: 0x161a22,
    ink: 0xffffff,
    ink2: 0xe3e7ec,
    muted: 0xc3cad3,
    accent: 0x7fd0ff,
    on_accent: 0x10141a,
    warn: 0xffc48a,
    row: Rgba(0xffffff, 0.11),
    line: Rgba(0xffffff, 0.14),
    raised: Rgba(0xffffff, 0.10),
    edge: Rgba(0xffffff, 0.18),
    shadow: Rgba(0x000000, 0.16),
};
pub const LIGHT: Theme = Theme {
    fill: 0xfafbfc,
    ink: 0x10141a,
    ink2: 0x2c333c,
    muted: 0x454d57,
    accent: 0x0a5f9f,
    on_accent: 0xffffff,
    warn: 0x8a3d00,
    row: Rgba(0x000000, 0.07),
    line: Rgba(0x000000, 0.11),
    raised: Rgba(0xffffff, 0.72),
    edge: Rgba(0xffffff, 0.95),
    shadow: Rgba(0x14233c, 0.05),
};

#[derive(Clone, Copy)]
enum Ink {
    Strong,
    Soft,
    Muted,
    Accent,
    OnAccent,
    Warn,
}

#[derive(Clone, Copy, PartialEq)]
enum Fill {
    Row,
    Accent,
    Muted,
    OnAccent,
}

#[derive(Clone, Copy, PartialEq)]
enum Zone {
    Head,
    Body,
    Foot,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Pin,
    Tab(usize),
    Sense(usize),
    Suggestion(String),
    Retry,
    Open(String),
    Sound(usize),
    OpenSettings,
    /// A click on a settings row: select it, then make the change.
    Settings(usize, SettingsChange),
}

impl Action {
    /// Rows and the readings tint under the mouse.
    fn tints(&self) -> bool {
        matches!(
            self,
            Action::Sense(_) | Action::Settings(..) | Action::Sound(_)
        )
    }
}

struct Piece {
    layout: IDWriteTextLayout,
    x: f32,
    y: f32,
    ink: Ink,
    zone: Zone,
}

struct Shape {
    rect: D2D_RECT_F,
    radius: f32,
    fill: Fill,
    zone: Zone,
}

struct Hit {
    rect: D2D_RECT_F,
    zone: Zone,
    action: Action,
}

#[derive(Default)]
pub struct Frame {
    pieces: Vec<Piece>,
    shapes: Vec<Shape>,
    hits: Vec<Hit>,
    head_h: f32,
    body_h: f32,
    has_foot: bool,
    selected: Option<(f32, f32)>,
    /// Where the input caret goes; None on pages without the input.
    caret_x: Option<f32>,
}

impl Frame {
    fn foot_h(&self) -> f32 {
        if self.has_foot {
            FOOT_H
        } else {
            0.0
        }
    }
    pub fn visible_body(&self) -> f32 {
        let room = (MAX_CARD - self.head_h - self.foot_h()).max(MIN_BODY);
        self.body_h.min(room)
    }
    pub fn height(&self) -> f32 {
        self.head_h + self.visible_body() + self.foot_h()
    }
    pub fn max_scroll(&self) -> f32 {
        (self.body_h - self.visible_body()).max(0.0)
    }
    /// The scroll position that keeps the selected row on screen.
    pub fn scroll_to_selected(&self, scroll: f32) -> f32 {
        let Some((top, bottom)) = self.selected else {
            return scroll;
        };
        let view = self.visible_body();
        let scroll = if bottom - scroll > view {
            bottom - view
        } else {
            scroll
        };
        (if top < scroll { top } else { scroll }).clamp(0.0, self.max_scroll())
    }
    pub fn hit(&self, x: f32, y: f32, scroll: f32) -> Option<Action> {
        let foot_top = self.head_h + self.visible_body();
        let (zone, y) = if y < self.head_h {
            (Zone::Head, y)
        } else if y < foot_top {
            (Zone::Body, y - self.head_h + scroll)
        } else {
            (Zone::Foot, y - foot_top)
        };
        self.hits
            .iter()
            .find(|hit| {
                hit.zone == zone
                    && x >= hit.rect.left
                    && x < hit.rect.right
                    && y >= hit.rect.top
                    && y < hit.rect.bottom
            })
            .map(|hit| hit.action.clone())
    }
    /// The row or reading under the mouse, in its zone's coordinates.
    fn hovered(&self, x: f32, y: f32, scroll: f32) -> Option<(Zone, D2D_RECT_F)> {
        let foot_top = self.head_h + self.visible_body();
        let (zone, y) = if y < self.head_h {
            (Zone::Head, y)
        } else if y < foot_top {
            (Zone::Body, y - self.head_h + scroll)
        } else {
            return None;
        };
        self.hits
            .iter()
            .find(|hit| {
                hit.zone == zone
                    && hit.action.tints()
                    && x >= hit.rect.left
                    && x < hit.rect.right
                    && y >= hit.rect.top
                    && y < hit.rect.bottom
            })
            .map(|hit| (zone, hit.rect))
    }
}

struct Builder<'a> {
    dwrite: &'a IDWriteFactory,
    frame: Frame,
}

struct Style {
    size: f32,
    bold: bool,
    ink: Ink,
    /// None is the text family.
    font: Option<PCWSTR>,
    /// Line height as a multiple of the size; 0 keeps the font's own.
    line: f32,
}

const fn style(size: f32, bold: bool, ink: Ink) -> Style {
    Style {
        size,
        bold,
        ink,
        font: None,
        line: 0.0,
    }
}

const fn icon(size: f32, ink: Ink) -> Style {
    Style {
        size,
        bold: false,
        ink,
        font: Some(ICON_FONT),
        line: 0.0,
    }
}

impl Style {
    /// Paragraphs that wrap read easier with a little more room between lines.
    const fn lines(mut self, line: f32) -> Self {
        self.line = line;
        self
    }
}

impl Builder<'_> {
    /// Returns the size the text actually takes.
    fn text(
        &mut self,
        zone: Zone,
        text: &str,
        x: f32,
        y: f32,
        width: f32,
        style: Style,
    ) -> Result<(f32, f32)> {
        let ink = style.ink;
        let (layout, size) = self.layout(text, width, style)?;
        self.frame.pieces.push(Piece {
            layout,
            x,
            y,
            ink,
            zone,
        });
        Ok(size)
    }

    fn layout(
        &self,
        text: &str,
        width: f32,
        style: Style,
    ) -> Result<(IDWriteTextLayout, (f32, f32))> {
        unsafe {
            let weight = if style.bold {
                DWRITE_FONT_WEIGHT_SEMI_BOLD
            } else {
                DWRITE_FONT_WEIGHT_NORMAL
            };
            let family = style.font.unwrap_or_else(|| text_family(self.dwrite));
            let format = self.dwrite.CreateTextFormat(
                family,
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                style.size,
                w!("zh-CN"),
            )?;
            if style.line > 0.0 {
                let height = style.size * style.line;
                format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, height, height * 0.8)?;
            }
            let wide: Vec<u16> = text.encode_utf16().collect();
            let layout = self
                .dwrite
                .CreateTextLayout(&wide, &format, width, 4000.0)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            Ok((
                layout,
                (metrics.widthIncludingTrailingWhitespace, metrics.height),
            ))
        }
    }

    /// One reading as a quiet button: region, IPA, then the speaker. It tints only under the
    /// mouse. Returns its width; the caller wraps the row.
    fn reading(
        &mut self,
        x: f32,
        y: f32,
        chip: &PronunciationChip,
        sound: Sound,
        index: usize,
    ) -> Result<f32> {
        let voiced = !chip.url.is_empty();
        let (ipa_ink, icon_ink) = match sound {
            Sound::Loading | Sound::Playing => (Ink::Accent, Ink::Accent),
            Sound::Failed => (Ink::Muted, Ink::Muted),
            Sound::Idle => (Ink::Soft, Ink::Accent),
        };
        let mut x2 = x + 6.0;
        let (w, _) = self.text(
            Zone::Head,
            &chip.lang,
            x2,
            y + 6.0,
            40.0,
            style(12.5, false, Ink::Muted),
        )?;
        x2 += w + 5.0;
        let (w, _) = self.text(
            Zone::Head,
            &chip.pron,
            x2,
            y + 4.0,
            220.0,
            style(14.0, false, ipa_ink),
        )?;
        x2 += w;
        if voiced {
            let glyph = match sound {
                Sound::Loading | Sound::Playing => "\u{E71A}",
                Sound::Failed => "\u{E72C}",
                Sound::Idle => "\u{E767}",
            };
            let (w, _) = self.text(
                Zone::Head,
                glyph,
                x2 + 5.0,
                y + 8.0,
                20.0,
                icon(12.0, icon_ink),
            )?;
            x2 += 5.0 + w;
        }
        let width = x2 + 6.0 - x;
        if voiced {
            self.hit(Zone::Head, rect(x, y, width, 28.0), Action::Sound(index));
        }
        Ok(width)
    }

    fn shape(&mut self, zone: Zone, rect: D2D_RECT_F, radius: f32, fill: Fill) {
        self.frame.shapes.push(Shape {
            rect,
            radius,
            fill,
            zone,
        });
    }

    fn hit(&mut self, zone: Zone, rect: D2D_RECT_F, action: Action) {
        self.frame.hits.push(Hit { rect, zone, action });
    }

    fn button(
        &mut self,
        zone: Zone,
        x: f32,
        y: f32,
        label: &str,
        primary: bool,
        action: Action,
    ) -> Result<f32> {
        let ink = if primary { Ink::OnAccent } else { Ink::Strong };
        let (w, h) = self.text(
            zone,
            label,
            x + 12.0,
            y + 7.0,
            300.0,
            style(13.0, primary, ink),
        )?;
        let rect = rect(x, y, w + 24.0, h + 14.0);
        // Shapes are drawn first, so the text above still lands on top.
        self.shape(
            zone,
            rect,
            8.0,
            if primary { Fill::Accent } else { Fill::Row },
        );
        self.hit(zone, rect, action);
        Ok(rect.right - x + 6.0)
    }
}

const fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

const fn moved(r: D2D_RECT_F, dx: f32, dy: f32) -> D2D_RECT_F {
    rect(r.left + dx, r.top + dy, r.right - r.left, r.bottom - r.top)
}

/// The settings page in place of the card: a title row, three setting rows, the version.
pub fn build_settings(
    dwrite: &IDWriteFactory,
    view: &SettingsView,
    settings: &Settings,
) -> Result<Frame> {
    let mut b = Builder {
        dwrite,
        frame: Frame::default(),
    };
    b.text(
        Zone::Head,
        "设置",
        PAD,
        15.0,
        200.0,
        style(17.0, false, Ink::Strong),
    )?;
    let text_x = PAD + 6.0;
    let row_w = WIDTH - 16.0;
    let right = WIDTH - PAD - 4.0;
    let mut y = 6.0;
    // Each row: its top, then the title and the line under it; the control sits on the right.
    let title = |b: &mut Builder, y: f32, text: &str| -> Result<f32> {
        Ok(b.text(
            Zone::Body,
            text,
            text_x,
            y,
            200.0,
            style(14.0, true, Ink::Strong),
        )?
        .1)
    };
    let finish_row =
        |b: &mut Builder, index: usize, top: f32, bottom: f32, change: SettingsChange| {
            let row = rect(8.0, top, row_w, bottom - top);
            if index == view.row {
                b.shape(
                    Zone::Body,
                    rect(12.0, top + 12.0, 3.0, bottom - top - 24.0),
                    1.5,
                    Fill::Accent,
                );
                b.frame.selected = Some((top, bottom));
            }
            b.hit(Zone::Body, row, Action::Settings(index, change));
        };

    // Automatic lookup, with its switch.
    let top = y;
    y += 10.0;
    let h = title(&mut b, y, "自动划词")?;
    let (_, line) = b.text(
        Zone::Body,
        "选中单词后自动弹出卡片",
        text_x,
        y + h + 2.0,
        240.0,
        style(13.0, false, Ink::Muted),
    )?;
    let middle = y + (h + 2.0 + line) / 2.0;
    let track = rect(right - 40.0, middle - 11.0, 40.0, 22.0);
    let on = settings.enabled;
    b.shape(
        Zone::Body,
        track,
        11.0,
        if on { Fill::Accent } else { Fill::Row },
    );
    let knob_x = if on {
        track.right - 19.0
    } else {
        track.left + 3.0
    };
    b.shape(
        Zone::Body,
        rect(knob_x, middle - 8.0, 16.0, 16.0),
        8.0,
        if on { Fill::OnAccent } else { Fill::Muted },
    );
    y += h + 2.0 + line + 10.0;
    finish_row(&mut b, 0, top, y, SettingsChange::ToggleAuto);
    y += 2.0;

    // The lookup shortcut: a key chip that records a new one.
    let top = y;
    y += 10.0;
    let h = title(&mut b, y, "查词快捷键")?;
    let hint = if view.recording {
        "按下新的组合键，Esc 取消，Backspace 停用"
    } else if settings.lookup_hotkey.is_empty() {
        "已停用，按 Enter 重新设置"
    } else {
        "任何地方按下即可查词"
    };
    let (_, line) = b.text(
        Zone::Body,
        hint,
        text_x,
        y + h + 2.0,
        right - 130.0 - text_x,
        style(13.0, false, Ink::Muted),
    )?;
    let chip = if view.recording {
        "按下组合键…".to_owned()
    } else if settings.lookup_hotkey.is_empty() {
        "未设置".to_owned()
    } else {
        settings.lookup_hotkey.replace('+', " + ")
    };
    let ink = if view.recording {
        Ink::Accent
    } else {
        Ink::Strong
    };
    let pieces = b.frame.pieces.len();
    let (w, ch) = b.text(Zone::Body, &chip, 0.0, 0.0, 200.0, style(13.0, false, ink))?;
    let chip_rect = rect(
        right - w - 20.0,
        y + (h + 2.0 + line) / 2.0 - (ch + 12.0) / 2.0,
        w + 20.0,
        ch + 12.0,
    );
    b.frame.pieces[pieces].x = chip_rect.left + 10.0;
    b.frame.pieces[pieces].y = chip_rect.top + 6.0;
    b.shape(Zone::Body, chip_rect, 8.0, Fill::Row);
    y += h + 2.0 + line;
    if let Some(notice) = &view.notice {
        let (_, nh) = b.text(
            Zone::Body,
            notice,
            text_x,
            y + 6.0,
            right - text_x,
            style(13.0, false, Ink::Warn),
        )?;
        y += nh + 6.0;
    }
    y += 10.0;
    finish_row(&mut b, 1, top, y, SettingsChange::Record);
    y += 2.0;

    // The appearance: three segments; the chosen one filled with the accent.
    let top = y;
    y += 10.0;
    let h = title(&mut b, y, "外观")?;
    y += h + 8.0;
    let group = rect(text_x, y, right - text_x, 34.0);
    b.shape(Zone::Body, group, 10.0, Fill::Row);
    let segment = (group.right - group.left - 6.0) / THEMES.len() as f32;
    let current = crate::card::theme_index(&settings.theme);
    for (index, (id, label)) in THEMES.iter().enumerate() {
        let cell = rect(
            group.left + 3.0 + segment * index as f32,
            y + 3.0,
            segment,
            28.0,
        );
        let chosen = index == current;
        if chosen {
            b.shape(Zone::Body, cell, 8.0, Fill::Accent);
        }
        let pieces = b.frame.pieces.len();
        let (w, th) = b.text(
            Zone::Body,
            label,
            0.0,
            0.0,
            segment,
            style(
                13.0,
                chosen,
                if chosen { Ink::OnAccent } else { Ink::Strong },
            ),
        )?;
        b.frame.pieces[pieces].x = cell.left + (segment - w) / 2.0;
        b.frame.pieces[pieces].y = cell.top + (28.0 - th) / 2.0;
        // Pushed before the row, so a click on a segment finds it first.
        b.hit(
            Zone::Body,
            cell,
            Action::Settings(2, SettingsChange::Theme(id)),
        );
    }
    y += 34.0 + 12.0;
    finish_row(&mut b, 2, top, y, SettingsChange::Nothing);
    y += 8.0;

    let (_, h) = b.text(
        Zone::Body,
        &format!(
            "WordSide {} · 词条来自 Cambridge Dictionary",
            env!("CARGO_PKG_VERSION")
        ),
        PAD,
        y,
        WIDTH - PAD * 2.0,
        style(12.5, false, Ink::Muted),
    )?;
    y += h + 12.0;

    b.frame.has_foot = true;
    b.text(
        Zone::Foot,
        if view.recording {
            "Esc 取消 · Backspace 停用快捷键"
        } else {
            "↑↓ 选择 · Enter 修改 · Esc 返回"
        },
        PAD,
        10.0,
        WIDTH - PAD * 2.0,
        style(12.5, false, Ink::Muted),
    )?;
    b.frame.head_h = INPUT_H;
    b.frame.body_h = y;
    Ok(b.frame)
}

pub fn build(
    dwrite: &IDWriteFactory,
    content: &Content,
    query: &str,
    pinned: bool,
) -> Result<Frame> {
    let mut b = Builder {
        dwrite,
        frame: Frame::default(),
    };

    // The input row.
    b.text(
        Zone::Head,
        "\u{E721}",
        PAD,
        18.0,
        20.0,
        icon(14.0, Ink::Muted),
    )?;
    let input_x = PAD + 24.0;
    if query.is_empty() {
        b.text(
            Zone::Head,
            "输入英文单词，回车查询",
            input_x,
            15.0,
            WIDTH - input_x - 82.0,
            style(15.0, false, Ink::Muted),
        )?;
        b.frame.caret_x = Some(input_x - 2.0);
    } else {
        let (w, _) = b.text(
            Zone::Head,
            query,
            input_x,
            14.0,
            2000.0,
            style(16.0, false, Ink::Strong),
        )?;
        b.frame.caret_x = Some((input_x + w).min(WIDTH - 84.0));
    }
    let pin_rect = rect(WIDTH - 42.0, 10.0, 32.0, 32.0);
    if pinned {
        b.shape(Zone::Head, pin_rect, 8.0, Fill::Row);
    }
    b.text(
        Zone::Head,
        if pinned { "\u{E840}" } else { "\u{E718}" },
        WIDTH - 33.0,
        18.0,
        20.0,
        icon(14.0, if pinned { Ink::Accent } else { Ink::Muted }),
    )?;
    b.hit(Zone::Head, pin_rect, Action::Pin);
    let gear_rect = rect(WIDTH - 76.0, 10.0, 32.0, 32.0);
    b.text(
        Zone::Head,
        "\u{E713}",
        WIDTH - 67.0,
        18.0,
        20.0,
        icon(14.0, Ink::Muted),
    )?;
    b.hit(Zone::Head, gear_rect, Action::OpenSettings);
    let mut head = INPUT_H;
    let mut y = 0.0;

    match content {
        Content::Empty => {}
        Content::Loading => {
            y = 14.0;
            let (_, h) = b.text(
                Zone::Body,
                "正在查询 Cambridge…",
                PAD,
                y,
                WIDTH - PAD * 2.0,
                style(14.0, false, Ink::Muted),
            )?;
            y += h + 14.0;
        }
        Content::Failure(failure) => {
            y = 14.0;
            let width = WIDTH - PAD * 2.0;
            let (_, h) = b.text(
                Zone::Body,
                failure.notice.title,
                PAD,
                y,
                width,
                style(14.0, true, Ink::Strong),
            )?;
            y += h + 4.0;
            let (_, h) = b.text(
                Zone::Body,
                failure.notice.message,
                PAD,
                y,
                width,
                style(13.0, false, Ink::Soft),
            )?;
            y += h + 10.0;
            for word in &failure.suggestions {
                let row = rect(8.0, y, WIDTH - 16.0, 36.0);
                b.text(
                    Zone::Body,
                    word,
                    PAD + 6.0,
                    y + 8.0,
                    width,
                    style(14.0, false, Ink::Strong),
                )?;
                b.hit(Zone::Body, row, Action::Suggestion(word.clone()));
                y += 38.0;
            }
            if failure.can_retry() {
                let mut x = PAD;
                if let Some(url) = &failure.source_url {
                    x += b.button(
                        Zone::Body,
                        x,
                        y,
                        "在浏览器打开",
                        true,
                        Action::Open(url.clone()),
                    )?;
                }
                b.button(Zone::Body, x, y, "重试", false, Action::Retry)?;
                y += 44.0;
            }
            y += 4.0;
        }
        Content::Entry(view) => {
            let width = WIDTH - PAD * 2.0;
            head += 12.0;
            // The word, with this part of speech's readings on the same line when they fit.
            let (word_w, word_h) = b.text(
                Zone::Head,
                &view.entry.word,
                PAD,
                head,
                width,
                style(26.0, true, Ink::Strong),
            )?;
            let mut x = PAD + word_w + 4.0;
            let mut line_top = head + (word_h - 28.0) / 2.0 + 2.0;
            let mut bottom = head + word_h;
            for index in view.tab_chips() {
                let pieces = b.frame.pieces.len();
                let hits = b.frame.hits.len();
                let w = b.reading(x, line_top, &view.chips[index], view.sounds[index], index)?;
                if x > PAD && x + w > WIDTH - PAD + 6.0 {
                    // Too wide for what is left: start a new line under the word.
                    let (dx, dy) = (PAD - 6.0 - x, bottom.max(line_top + 28.0) + 2.0 - line_top);
                    for piece in &mut b.frame.pieces[pieces..] {
                        piece.x += dx;
                        piece.y += dy;
                    }
                    for hit in &mut b.frame.hits[hits..] {
                        hit.rect = moved(hit.rect, dx, dy);
                    }
                    line_top += dy;
                    x = PAD - 6.0;
                }
                x += w + 2.0;
                bottom = bottom.max(line_top + 28.0);
            }
            head = bottom + 2.0;
            let notes = [
                view.origin(),
                view.tab_chips()
                    .any(|index| view.sounds[index] == Sound::Failed)
                    .then(|| "发音暂时无法播放，点击重试。".to_owned()),
                (view.coverage == Coverage::English)
                    .then(|| "这个词暂无中文释义，下面是英文解释。".to_owned()),
            ];
            for note in notes.into_iter().flatten() {
                let (_, h) = b.text(
                    Zone::Head,
                    &note,
                    PAD,
                    head,
                    width,
                    style(12.5, false, Ink::Muted),
                )?;
                head += h + 2.0;
            }
            if view.groups.len() > 1 {
                head += 2.0;
                let mut x = PAD;
                for (index, group) in view.groups.iter().enumerate() {
                    let current = index == view.tab;
                    let (w, _) = b.text(
                        Zone::Head,
                        &group.label,
                        x,
                        head + 5.0,
                        200.0,
                        style(
                            14.0,
                            current,
                            if current { Ink::Strong } else { Ink::Muted },
                        ),
                    )?;
                    let (cw, _) = b.text(
                        Zone::Head,
                        &group.count.to_string(),
                        x + w + 4.0,
                        head + 5.0,
                        60.0,
                        style(14.0, false, Ink::Muted),
                    )?;
                    let tab_w = w + 4.0 + cw;
                    if current {
                        b.shape(
                            Zone::Head,
                            rect(x, head + 28.0, tab_w, 2.0),
                            1.0,
                            Fill::Accent,
                        );
                    }
                    b.hit(
                        Zone::Head,
                        rect(x - 4.0, head, tab_w + 8.0, 30.0),
                        Action::Tab(index),
                    );
                    x += tab_w + 20.0;
                }
                head += 30.0;
            } else {
                head += 8.0;
            }

            y = 6.0;
            let row_w = WIDTH - 16.0;
            let tx = TEXT_X;
            let tw = WIDTH - PAD - tx;
            for (index, definition) in view.visible().into_iter().enumerate() {
                let top = y;
                y += 8.0;
                // First line: the translation (or the English-only tag) left; the source and,
                // when there are examples, an accent "例句 N" with an arrow on the right, so
                // a row that opens shows it.
                let open = view.expanded == Some(index);
                let meta_pieces = b.frame.pieces.len();
                let mut meta: Vec<(f32, f32)> = Vec::new();
                if !definition.source.is_empty() {
                    let label = if definition.examples.is_empty() {
                        source_label(&definition.source)
                    } else {
                        format!("{} · ", source_label(&definition.source))
                    };
                    meta.push(b.text(
                        Zone::Body,
                        &label,
                        0.0,
                        0.0,
                        tw,
                        style(12.5, false, Ink::Muted),
                    )?);
                }
                if !definition.examples.is_empty() {
                    let count = format!("例句 {}", definition.examples.len());
                    meta.push(b.text(
                        Zone::Body,
                        &count,
                        0.0,
                        0.0,
                        tw,
                        style(12.5, false, Ink::Accent),
                    )?);
                    let arrow = if open { "\u{E70E}" } else { "\u{E70D}" };
                    meta.push(b.text(Zone::Body, arrow, 0.0, 0.0, 20.0, icon(9.0, Ink::Accent))?);
                }
                let gaps = if definition.examples.is_empty() {
                    0.0
                } else {
                    4.0
                };
                let meta_w = meta.iter().map(|(w, _)| w).sum::<f32>() + gaps;
                let meta_h = meta.iter().map(|(_, h)| *h).fold(0.0, f32::max);
                let left_w = tw - if meta.is_empty() { 0.0 } else { meta_w + 12.0 };
                let first_h = if !definition.translation.trim().is_empty() {
                    b.text(
                        Zone::Body,
                        &definition.translation,
                        tx,
                        y,
                        left_w,
                        style(15.5, true, Ink::Strong).lines(1.35),
                    )?
                    .1
                } else if view.coverage == Coverage::Partial {
                    let pieces = b.frame.pieces.len();
                    let (w, h) = b.text(
                        Zone::Body,
                        "仅英文",
                        0.0,
                        0.0,
                        80.0,
                        style(12.0, false, Ink::Muted),
                    )?;
                    b.frame.pieces[pieces].x = tx + 5.0;
                    b.frame.pieces[pieces].y = y + 2.0;
                    b.shape(
                        Zone::Body,
                        rect(tx, y + 1.0, w + 10.0, h + 2.0),
                        4.0,
                        Fill::Row,
                    );
                    h + 4.0
                } else {
                    0.0
                };
                let first_h = first_h.max(meta_h);
                let meta_y = y + (first_h.min(22.0) - meta_h) / 2.0 + 1.0;
                let mut x = WIDTH - PAD - meta_w;
                for (offset, (w, h)) in meta.iter().enumerate() {
                    let piece = &mut b.frame.pieces[meta_pieces + offset];
                    let is_arrow = !definition.examples.is_empty() && offset + 1 == meta.len();
                    if is_arrow {
                        x += 4.0;
                    }
                    piece.x = x;
                    // The arrow glyph sits a little low next to the text.
                    piece.y = meta_y
                        + if is_arrow {
                            (meta_h - h) / 2.0 + 1.0
                        } else {
                            0.0
                        };
                    x += w;
                }
                if first_h > 0.0 {
                    y += first_h + 2.0;
                }
                let meaning = if definition.text.trim().is_empty()
                    && definition.translation.trim().is_empty()
                {
                    "暂无释义"
                } else {
                    definition.text.as_str()
                };
                if !meaning.trim().is_empty() {
                    let (_, h) = b.text(
                        Zone::Body,
                        meaning,
                        tx,
                        y,
                        tw,
                        style(14.0, false, Ink::Soft).lines(1.45),
                    )?;
                    y += h;
                }
                if view.expanded == Some(index) {
                    y += 4.0;
                    for example in &definition.examples {
                        y += 6.0;
                        let (_, h) = b.text(
                            Zone::Body,
                            &example.text,
                            tx,
                            y,
                            tw,
                            style(13.5, false, Ink::Soft).lines(1.45),
                        )?;
                        y += h;
                        if !example.translation.trim().is_empty() {
                            let (_, h) = b.text(
                                Zone::Body,
                                &example.translation,
                                tx,
                                y,
                                tw,
                                style(13.5, false, Ink::Muted).lines(1.45),
                            )?;
                            y += h;
                        }
                    }
                }
                y += 8.0;
                let row = rect(8.0, top, row_w, y - top);
                // The selected row gets its accent bar; whichever row the mouse is over
                // floats up as a glass tile, drawn at paint time.
                if index == view.selected {
                    b.shape(
                        Zone::Body,
                        rect(13.0, top + 11.0, 3.0, y - top - 22.0),
                        1.5,
                        Fill::Accent,
                    );
                    b.frame.selected = Some((top, y));
                }
                b.hit(Zone::Body, row, Action::Sense(index));
            }
            if view.entry.definitions.is_empty() {
                let (_, h) = b.text(
                    Zone::Body,
                    "暂无释义，可以查看原词典。",
                    PAD,
                    y + 8.0,
                    width,
                    style(13.0, false, Ink::Soft),
                )?;
                y += h + 16.0;
            }
            y += 6.0;

            b.frame.has_foot = true;
            b.text(
                Zone::Foot,
                "↑↓ 选择 · → 例句",
                PAD,
                10.0,
                WIDTH - PAD * 2.0 - 80.0,
                style(12.5, false, Ink::Muted),
            )?;
            let pieces = b.frame.pieces.len();
            let (w, _) = b.text(
                Zone::Foot,
                "原词典 ↗",
                0.0,
                10.0,
                100.0,
                style(12.5, false, Ink::Accent),
            )?;
            b.frame.pieces[pieces].x = WIDTH - PAD - w;
            b.hit(
                Zone::Foot,
                rect(WIDTH - PAD - w - 6.0, 0.0, w + 12.0, FOOT_H),
                Action::Open(view.entry.source_url.clone()),
            );
        }
    }
    b.frame.head_h = head;
    b.frame.body_h = y;
    Ok(b.frame)
}

fn color(rgb: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 255) as f32 / 255.0,
        g: ((rgb >> 8) & 255) as f32 / 255.0,
        b: (rgb & 255) as f32 / 255.0,
        a,
    }
}

pub struct Paint<'a> {
    pub target: &'a ID2D1HwndRenderTarget,
    pub theme: &'a Theme,
    pub glass: bool,
    pub frame: &'a Frame,
    pub scroll: f32,
    /// The mouse position in the card, while it is over the card.
    pub mouse: Option<(f32, f32)>,
}

pub fn paint(p: Paint) -> Result<()> {
    let Paint {
        target,
        theme,
        glass,
        frame,
        scroll,
        mouse,
    } = p;
    let hovered = mouse.and_then(|(x, y)| frame.hovered(x, y, scroll));
    unsafe {
        let brush = |rgb: u32, a: f32| target.CreateSolidColorBrush(&color(rgb, a), None);
        let ink = brush(theme.ink, 1.0)?;
        let ink2 = brush(theme.ink2, 1.0)?;
        let muted = brush(theme.muted, 1.0)?;
        let accent = brush(theme.accent, 1.0)?;
        let on_accent = brush(theme.on_accent, 1.0)?;
        let warn = brush(theme.warn, 1.0)?;
        let row = brush(theme.row.0, theme.row.1)?;
        let line = brush(theme.line.0, theme.line.1)?;
        let raised = brush(theme.raised.0, theme.raised.1)?;
        let edge = brush(theme.edge.0, theme.edge.1)?;
        let shadow = brush(theme.shadow.0, theme.shadow.1)?;
        let ink_of = |value: Ink| match value {
            Ink::Strong => &ink,
            Ink::Soft => &ink2,
            Ink::Muted => &muted,
            Ink::Accent => &accent,
            Ink::OnAccent => &on_accent,
            Ink::Warn => &warn,
        };
        let fill_of = |value: Fill| match value {
            Fill::Row => &row,
            Fill::Accent => &accent,
            Fill::Muted => &muted,
            Fill::OnAccent => &on_accent,
        };

        target.BeginDraw();
        target.SetTransform(&Matrix3x2::identity());
        if glass {
            // ClearType needs an opaque background, so glass text is grayscale.
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            target.Clear(Some(&color(0, 0.0)));
            let tint = brush(theme.fill, GLASS_OPACITY)?;
            target.FillRectangle(&rect(0.0, 0.0, WIDTH + 2.0, 4000.0), &tint);
        } else {
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_DEFAULT);
            target.Clear(Some(&color(theme.fill, 1.0)));
        }

        let body_top = frame.head_h;
        let body_h = frame.visible_body();
        let foot_top = body_top + body_h;
        let draw = |zone: Zone, dy: f32| {
            if let Some((_, r)) = hovered.filter(|(hovered, _)| *hovered == zone) {
                let r = moved(r, 0.0, dy);
                if zone == Zone::Head {
                    // The readings are small; a plain tint is enough there.
                    target.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: r,
                            radiusX: 8.0,
                            radiusY: 8.0,
                        },
                        &row,
                    );
                } else {
                    // A row under the mouse floats up: a soft shadow stepping out below,
                    // then a lighter glass tile with a thin highlight edge.
                    let radius = 10.0;
                    for (grow, drop) in [(2.5, 4.0), (1.0, 2.0), (0.0, 1.0)] {
                        target.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: rect(
                                    r.left - grow,
                                    r.top - grow + drop,
                                    r.right - r.left + grow * 2.0,
                                    r.bottom - r.top + grow * 2.0,
                                ),
                                radiusX: radius + grow,
                                radiusY: radius + grow,
                            },
                            &shadow,
                        );
                    }
                    let tile = D2D1_ROUNDED_RECT {
                        rect: r,
                        radiusX: radius,
                        radiusY: radius,
                    };
                    target.FillRoundedRectangle(&tile, &raised);
                    // Half a pixel in, so the edge stays sharp.
                    let inner = D2D1_ROUNDED_RECT {
                        rect: rect(
                            r.left + 0.5,
                            r.top + 0.5,
                            r.right - r.left - 1.0,
                            r.bottom - r.top - 1.0,
                        ),
                        ..tile
                    };
                    target.DrawRoundedRectangle(&inner, &edge, 1.0, None);
                }
            }
            for shape in frame.shapes.iter().filter(|s| s.zone == zone) {
                let mut r = shape.rect;
                r.top += dy;
                r.bottom += dy;
                target.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: r,
                        radiusX: shape.radius,
                        radiusY: shape.radius,
                    },
                    fill_of(shape.fill),
                );
            }
            for piece in frame.pieces.iter().filter(|p| p.zone == zone) {
                let y = piece.y + dy;
                if zone == Zone::Body && (y > foot_top || y < body_top - 2000.0) {
                    continue;
                }
                target.DrawTextLayout(
                    Vector2 { X: piece.x, Y: y },
                    &piece.layout,
                    ink_of(piece.ink),
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        };

        // The query can be longer than the row, so it is clipped before the pin.
        target.PushAxisAlignedClip(
            &rect(0.0, 0.0, WIDTH, frame.head_h),
            D2D1_ANTIALIAS_MODE_ALIASED,
        );
        draw(Zone::Head, 0.0);
        if let Some(caret_x) = frame.caret_x {
            target.FillRectangle(&rect(caret_x, 15.0, 1.5, 22.0), &accent);
        }
        target.PopAxisAlignedClip();
        // Under the input row, and under an entry's header when it has one.
        for y in [INPUT_H, frame.head_h] {
            if y == INPUT_H || y > INPUT_H + 1.0 {
                target.DrawLine(
                    Vector2 { X: 0.0, Y: y },
                    Vector2 { X: WIDTH, Y: y },
                    &line,
                    1.0,
                    None,
                );
            }
        }

        if body_h > 0.0 {
            target.PushAxisAlignedClip(
                &rect(0.0, body_top, WIDTH, body_h),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            draw(Zone::Body, body_top - scroll);
            target.PopAxisAlignedClip();
            // More below: fade the last lines out; gone once the list reaches its end.
            if scroll < frame.max_scroll() - 1.0 {
                let to = if glass { 0.9 } else { 1.0 };
                let stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: color(theme.fill, 0.0),
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: color(theme.fill, to),
                    },
                ];
                let collection = target.CreateGradientStopCollection(
                    &stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                )?;
                let fade = target.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: Vector2 {
                            X: 0.0,
                            Y: foot_top - FADE_H,
                        },
                        endPoint: Vector2 {
                            X: 0.0,
                            Y: foot_top,
                        },
                    },
                    None,
                    &collection,
                )?;
                target.FillRectangle(&rect(0.0, foot_top - FADE_H, WIDTH, FADE_H), &fade);
            }
            if frame.max_scroll() > 0.0 {
                let track = body_h - 8.0;
                let thumb = (track * body_h / frame.body_h).max(24.0);
                let top = body_top + 4.0 + (track - thumb) * scroll / frame.max_scroll();
                target.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect(WIDTH - 5.0, top, 3.0, thumb),
                        radiusX: 1.5,
                        radiusY: 1.5,
                    },
                    &muted,
                );
            }
        }
        if frame.has_foot {
            target.DrawLine(
                Vector2 {
                    X: 0.0,
                    Y: foot_top,
                },
                Vector2 {
                    X: WIDTH,
                    Y: foot_top,
                },
                &line,
                1.0,
                None,
            );
            draw(Zone::Foot, foot_top);
        }
        target.EndDraw(None, None)
    }
}
