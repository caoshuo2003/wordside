//! Lays the card out with DirectWrite and draws it with Direct2D.
use crate::card::{Content, Coverage, Sound};
use crate::presentation::{source_label, PronunciationChip};
use windows::{
    core::{w, Result, PCWSTR},
    Win32::Graphics::{
        Direct2D::{Common::*, *},
        DirectWrite::*,
    },
};
use windows_numerics::{Matrix3x2, Vector2};

pub const WIDTH: f32 = 388.0;
const PAD: f32 = 16.0;
pub const INPUT_H: f32 = 54.0;
const FOOT_H: f32 = 34.0;
/// The whole card stops growing here; the body scrolls instead.
const MAX_CARD: f32 = 420.0;
/// A tall header (many pronunciations) still leaves this much list in view.
const MIN_BODY: f32 = 120.0;
const FADE_H: f32 = 36.0;
const GLASS_OPACITY: f32 = 0.92;
const ICON_FONT: PCWSTR = w!("Segoe MDL2 Assets");
const TEXT_FONT: PCWSTR = w!("Segoe UI");

#[derive(Clone, Copy, PartialEq)]
pub struct Rgba(u32, f32);

pub struct Theme {
    fill: u32,
    ink: u32,
    ink2: u32,
    muted: u32,
    accent: u32,
    on_accent: u32,
    row: Rgba,
    line: Rgba,
}

pub const DARK: Theme = Theme {
    fill: 0x161a22,
    ink: 0xffffff,
    ink2: 0xe3e7ec,
    muted: 0xc3cad3,
    accent: 0x7fd0ff,
    on_accent: 0x10141a,
    row: Rgba(0xffffff, 0.11),
    line: Rgba(0xffffff, 0.14),
};
pub const LIGHT: Theme = Theme {
    fill: 0xfafbfc,
    ink: 0x10141a,
    ink2: 0x2c333c,
    muted: 0x454d57,
    accent: 0x0a5f9f,
    on_accent: 0xffffff,
    row: Rgba(0x000000, 0.07),
    line: Rgba(0x000000, 0.11),
};

#[derive(Clone, Copy)]
enum Ink {
    Ink,
    Ink2,
    Muted,
    Accent,
    OnAccent,
}

#[derive(Clone, Copy)]
enum Fill {
    Row,
    Accent,
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
    caret_x: f32,
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
    /// The definition row under the mouse, in body coordinates.
    fn hovered_row(&self, x: f32, y: f32, scroll: f32) -> Option<D2D_RECT_F> {
        let foot_top = self.head_h + self.visible_body();
        if y < self.head_h || y >= foot_top {
            return None;
        }
        let y = y - self.head_h + scroll;
        self.hits
            .iter()
            .find(|hit| {
                matches!(hit.action, Action::Sense(_))
                    && x >= hit.rect.left
                    && x < hit.rect.right
                    && y >= hit.rect.top
                    && y < hit.rect.bottom
            })
            .map(|hit| hit.rect)
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
    font: PCWSTR,
}

const fn style(size: f32, bold: bool, ink: Ink) -> Style {
    Style {
        size,
        bold,
        ink,
        font: TEXT_FONT,
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
            let format = self.dwrite.CreateTextFormat(
                style.font,
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                style.size,
                w!("zh-CN"),
            )?;
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

    /// A pronunciation pill. Returns its width and height; the caller wraps the row.
    fn chip(
        &mut self,
        x: f32,
        y: f32,
        chip: &PronunciationChip,
        sound: Sound,
        index: usize,
    ) -> Result<(f32, f32)> {
        let prefix = if chip.pos.is_empty() {
            String::new()
        } else {
            format!("{} ", chip.pos)
        };
        let label = format!("{prefix}{} {}", chip.lang, chip.pron);
        let voiced = !chip.url.is_empty();
        let ink = match sound {
            Sound::Loading | Sound::Playing => Ink::Accent,
            Sound::Failed => Ink::Muted,
            Sound::Idle if voiced => Ink::Ink,
            Sound::Idle => Ink::Ink2,
        };
        let (w, h) = self.text(
            Zone::Head,
            &label,
            x + 10.0,
            y + 4.0,
            300.0,
            style(13.0, false, ink),
        )?;
        let mut width = w + 20.0;
        if voiced {
            let icon = match sound {
                Sound::Loading | Sound::Playing => "\u{E71A}",
                Sound::Failed => "\u{E72C}",
                Sound::Idle => "\u{E767}",
            };
            let (iw, _) = self.text(
                Zone::Head,
                icon,
                x + 10.0 + w + 4.0,
                y + 7.0,
                20.0,
                Style {
                    size: 12.0,
                    bold: false,
                    ink,
                    font: ICON_FONT,
                },
            )?;
            width += iw + 4.0;
        }
        let pill = rect(x, y, width, h + 8.0);
        if voiced {
            self.shape(Zone::Head, pill, (h + 8.0) / 2.0, Fill::Row);
            self.hit(Zone::Head, pill, Action::Sound(index));
        }
        Ok((width, h + 8.0))
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
        let ink = if primary { Ink::OnAccent } else { Ink::Ink };
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
        19.0,
        20.0,
        Style {
            size: 15.0,
            bold: false,
            ink: Ink::Muted,
            font: ICON_FONT,
        },
    )?;
    let input_x = PAD + 26.0;
    if query.is_empty() {
        b.text(
            Zone::Head,
            "输入英文单词，回车查询",
            input_x,
            15.0,
            WIDTH - input_x - 48.0,
            style(17.0, false, Ink::Muted),
        )?;
        b.frame.caret_x = input_x - 2.0;
    } else {
        let (w, _) = b.text(
            Zone::Head,
            query,
            input_x,
            15.0,
            2000.0,
            style(17.0, false, Ink::Ink),
        )?;
        b.frame.caret_x = (input_x + w).min(WIDTH - 50.0);
    }
    let pin_rect = rect(WIDTH - 42.0, 11.0, 32.0, 32.0);
    if pinned {
        b.shape(Zone::Head, pin_rect, 8.0, Fill::Row);
    }
    b.text(
        Zone::Head,
        if pinned { "\u{E840}" } else { "\u{E718}" },
        WIDTH - 34.0,
        19.0,
        20.0,
        Style {
            size: 15.0,
            bold: false,
            ink: if pinned { Ink::Accent } else { Ink::Ink2 },
            font: ICON_FONT,
        },
    )?;
    b.hit(Zone::Head, pin_rect, Action::Pin);
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
                style(14.0, true, Ink::Ink),
            )?;
            y += h + 4.0;
            let (_, h) = b.text(
                Zone::Body,
                failure.notice.message,
                PAD,
                y,
                width,
                style(13.0, false, Ink::Ink2),
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
                    style(14.0, false, Ink::Ink),
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
            let (_, h) = b.text(
                Zone::Head,
                &view.entry.word,
                PAD,
                head,
                width,
                style(26.0, true, Ink::Ink),
            )?;
            head += h + 2.0;
            if let Some(origin) = view.origin() {
                let (_, h) = b.text(
                    Zone::Head,
                    &origin,
                    PAD,
                    head,
                    width,
                    style(12.0, false, Ink::Muted),
                )?;
                head += h + 2.0;
            }
            if !view.chips.is_empty() {
                head += 4.0;
                let mut x = PAD;
                let mut line_h = 0.0f32;
                for (index, chip) in view.chips.iter().enumerate() {
                    let pieces = b.frame.pieces.len();
                    let shapes = b.frame.shapes.len();
                    let hits = b.frame.hits.len();
                    let (w, h) = b.chip(x, head, chip, view.sounds[index], index)?;
                    // Too wide for what is left of the row: move the pill to a new one.
                    if x > PAD && x + w > WIDTH - PAD {
                        let (dx, dy) = (PAD - x, line_h + 6.0);
                        for piece in &mut b.frame.pieces[pieces..] {
                            piece.x += dx;
                            piece.y += dy;
                        }
                        for shape in &mut b.frame.shapes[shapes..] {
                            shape.rect = moved(shape.rect, dx, dy);
                        }
                        for hit in &mut b.frame.hits[hits..] {
                            hit.rect = moved(hit.rect, dx, dy);
                        }
                        head += dy;
                        x = PAD;
                        line_h = 0.0;
                    }
                    x += w + 6.0;
                    line_h = line_h.max(h);
                }
                head += line_h + 6.0;
                if view.sounds.contains(&Sound::Failed) {
                    let (_, h) = b.text(
                        Zone::Head,
                        "发音暂时无法播放，点击重试。",
                        PAD,
                        head,
                        width,
                        style(12.0, false, Ink::Muted),
                    )?;
                    head += h + 4.0;
                }
            }
            match view.coverage {
                Coverage::English => {
                    let (_, h) = b.text(
                        Zone::Head,
                        "这个词暂无中文释义，下面是英文解释。",
                        PAD,
                        head,
                        width,
                        style(12.0, false, Ink::Muted),
                    )?;
                    head += h + 6.0;
                }
                Coverage::Partial => {
                    let (_, h) = b.text(
                        Zone::Head,
                        "部分释义仅有英文。",
                        PAD,
                        head,
                        width,
                        style(12.0, false, Ink::Muted),
                    )?;
                    head += h + 6.0;
                }
                _ => {}
            }
            if view.groups.len() > 1 {
                let mut x = PAD;
                for (index, group) in view.groups.iter().enumerate() {
                    let current = index == view.tab;
                    let label = format!("{} {}", group.label, group.count);
                    let (w, h) = b.text(
                        Zone::Head,
                        &label,
                        x,
                        head,
                        200.0,
                        style(13.0, current, if current { Ink::Ink } else { Ink::Muted }),
                    )?;
                    if current {
                        b.shape(
                            Zone::Head,
                            rect(x, head + h + 3.0, w, 2.0),
                            1.0,
                            Fill::Accent,
                        );
                    }
                    b.hit(
                        Zone::Head,
                        rect(x - 4.0, head - 4.0, w + 8.0, h + 12.0),
                        Action::Tab(index),
                    );
                    x += w + 16.0;
                }
                head += 28.0;
            } else {
                head += 2.0;
            }

            y = 2.0;
            let x = PAD + 6.0;
            let row_w = WIDTH - 16.0;
            let inner = WIDTH - x - PAD - 2.0;
            for (index, definition) in view.visible().into_iter().enumerate() {
                let top = y;
                y += 8.0;
                let number = format!("{}", index + 1);
                let (_, _) = b.text(
                    Zone::Body,
                    &number,
                    20.0,
                    y + 2.0,
                    14.0,
                    style(12.0, false, Ink::Muted),
                )?;
                let tx = x + 16.0;
                let tw = inner - 16.0;
                if !definition.translation.trim().is_empty() {
                    let (_, h) = b.text(
                        Zone::Body,
                        &definition.translation,
                        tx,
                        y,
                        tw,
                        style(15.0, true, Ink::Ink),
                    )?;
                    y += h + 2.0;
                } else if view.coverage == Coverage::Partial {
                    let (_, h) = b.text(
                        Zone::Body,
                        "英文释义",
                        tx,
                        y,
                        tw,
                        style(12.0, false, Ink::Muted),
                    )?;
                    y += h + 2.0;
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
                        style(13.0, false, Ink::Ink2),
                    )?;
                    y += h;
                }
                if view.expanded == Some(index) {
                    for example in &definition.examples {
                        y += 6.0;
                        let (_, h) = b.text(
                            Zone::Body,
                            &example.text,
                            tx,
                            y,
                            tw,
                            style(13.0, false, Ink::Muted),
                        )?;
                        y += h;
                        if !example.translation.trim().is_empty() {
                            let (_, h) = b.text(
                                Zone::Body,
                                &example.translation,
                                tx,
                                y,
                                tw,
                                style(13.0, false, Ink::Muted),
                            )?;
                            y += h;
                        }
                    }
                } else if !definition.examples.is_empty() {
                    y += 2.0;
                    let (_, h) = b.text(
                        Zone::Body,
                        &format!("例句 {} →", definition.examples.len()),
                        tx,
                        y,
                        tw,
                        style(12.0, false, Ink::Muted),
                    )?;
                    y += h;
                }
                if !definition.source.is_empty() {
                    y += 2.0;
                    let (_, h) = b.text(
                        Zone::Body,
                        &source_label(&definition.source),
                        tx,
                        y,
                        tw,
                        style(12.0, false, Ink::Muted),
                    )?;
                    y += h;
                }
                y += 8.0;
                let row = rect(8.0, top, row_w, y - top);
                // The selected row only gets its bar; the row tint follows the mouse.
                if index == view.selected {
                    b.shape(
                        Zone::Body,
                        rect(12.0, top + 11.0, 3.0, y - top - 22.0),
                        1.5,
                        Fill::Accent,
                    );
                    b.frame.selected = Some((top, y));
                }
                b.hit(Zone::Body, row, Action::Sense(index));
                y += 2.0;
            }
            if view.entry.definitions.is_empty() {
                let (_, h) = b.text(
                    Zone::Body,
                    "暂无释义，可以查看原词典。",
                    PAD,
                    y + 8.0,
                    width,
                    style(13.0, false, Ink::Ink2),
                )?;
                y += h + 16.0;
            }
            y += 6.0;

            b.frame.has_foot = true;
            b.text(
                Zone::Foot,
                if view.has_sound() {
                    "↑↓ 选择   → 例句   Ctrl+P 发音   Esc 关闭"
                } else {
                    "↑↓ 选择   → 例句   Esc 关闭"
                },
                PAD,
                9.0,
                WIDTH - PAD * 2.0 - 66.0,
                style(12.0, false, Ink::Muted),
            )?;
            let (w, _) = b.text(
                Zone::Foot,
                "原词典 ↗",
                WIDTH - PAD - 60.0,
                9.0,
                60.0,
                style(12.0, false, Ink::Accent),
            )?;
            b.hit(
                Zone::Foot,
                rect(WIDTH - PAD - 66.0, 0.0, w + 12.0, FOOT_H),
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
    let hovered = mouse.and_then(|(x, y)| frame.hovered_row(x, y, scroll));
    unsafe {
        let brush = |rgb: u32, a: f32| target.CreateSolidColorBrush(&color(rgb, a), None);
        let ink = brush(theme.ink, 1.0)?;
        let ink2 = brush(theme.ink2, 1.0)?;
        let muted = brush(theme.muted, 1.0)?;
        let accent = brush(theme.accent, 1.0)?;
        let on_accent = brush(theme.on_accent, 1.0)?;
        let row = brush(theme.row.0, theme.row.1)?;
        let line = brush(theme.line.0, theme.line.1)?;
        let ink_of = |value: Ink| match value {
            Ink::Ink => &ink,
            Ink::Ink2 => &ink2,
            Ink::Muted => &muted,
            Ink::Accent => &accent,
            Ink::OnAccent => &on_accent,
        };
        let fill_of = |value: Fill| match value {
            Fill::Row => &row,
            Fill::Accent => &accent,
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
            if let (Zone::Body, Some(r)) = (zone, hovered) {
                target.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: moved(r, 0.0, dy),
                        radiusX: 9.0,
                        radiusY: 9.0,
                    },
                    &row,
                );
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
        target.FillRectangle(&rect(frame.caret_x, 16.0, 1.5, 22.0), &accent);
        target.PopAxisAlignedClip();
        target.DrawLine(
            Vector2 { X: 0.0, Y: INPUT_H },
            Vector2 {
                X: WIDTH,
                Y: INPUT_H,
            },
            &line,
            1.0,
            None,
        );

        if body_h > 0.0 {
            target.PushAxisAlignedClip(
                &rect(0.0, body_top, WIDTH, body_h),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            draw(Zone::Body, body_top - scroll);
            target.PopAxisAlignedClip();
            // More below: fade the last lines out; gone once the list reaches its end.
            if scroll < frame.max_scroll() - 1.0 {
                let to = if glass { 0.96 } else { 1.0 };
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
