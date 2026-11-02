//! WordSide's mark, drawn in code: a card popping up from a selected word.
//! The tray glyph is drawn at the size and color the taskbar needs; the app icon is
//! exported once to `assets/wordside.ico` with `--export-icon` and built into the exe.
use std::path::Path;
use windows::{
    core::{Interface, Result, BOOL},
    Win32::{
        Foundation::{E_FAIL, RECT},
        Graphics::{
            Direct2D::{Common::*, *},
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::*,
            Imaging::*,
        },
        System::Com::{
            CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
            STATFLAG_NONAME, STREAM_SEEK_SET,
        },
        UI::{
            Shell::SHCreateMemStream,
            WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO},
        },
    },
};
use windows_numerics::Matrix3x2;

/// The sizes Windows picks from for the exe, shortcuts and Explorer.
const APP_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

fn color(rgb: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 255) as f32 / 255.0,
        g: ((rgb >> 8) & 255) as f32 / 255.0,
        b: (rgb & 255) as f32 / 255.0,
        a,
    }
}

fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        },
        radiusX: r,
        radiusY: r,
    }
}

/// The card: a rounded rectangle with a pointer at the bottom middle, aimed at the word.
fn bubble(
    target: &ID2D1RenderTarget,
    area: D2D_RECT_F,
    radius: f32,
    tip: (f32, f32),
) -> Result<ID2D1PathGeometry> {
    let point = |x: f32, y: f32| windows_numerics::Vector2 { X: x, Y: y };
    let arc = |x: f32, y: f32| D2D1_ARC_SEGMENT {
        point: point(x, y),
        size: D2D_SIZE_F {
            width: radius,
            height: radius,
        },
        rotationAngle: 0.0,
        sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
        arcSize: D2D1_ARC_SIZE_SMALL,
    };
    let D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    } = area;
    let (middle, (half, depth)) = ((left + right) / 2.0, tip);
    unsafe {
        let geometry = target.GetFactory()?.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        sink.BeginFigure(point(left + radius, top), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(point(right - radius, top));
        sink.AddArc(&arc(right, top + radius));
        sink.AddLine(point(right, bottom - radius));
        sink.AddArc(&arc(right - radius, bottom));
        sink.AddLine(point(middle + half, bottom));
        sink.AddLine(point(middle, bottom + depth));
        sink.AddLine(point(middle - half, bottom));
        sink.AddLine(point(left + radius, bottom));
        sink.AddArc(&arc(left, bottom - radius));
        sink.AddLine(point(left, top + radius));
        sink.AddArc(&arc(left + radius, top));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        Ok(geometry)
    }
}

const fn area(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

/// The app icon on a 64-unit grid: on a graphite tile, a white card pops up from the
/// selected word, which is the one touch of blue.
fn draw_app(target: &ID2D1RenderTarget) -> Result<()> {
    unsafe {
        let graphite = target.CreateSolidColorBrush(&color(0x1c1e23, 1.0), None)?;
        let faint = target.CreateSolidColorBrush(&color(0x1c1e23, 0.3), None)?;
        let white = target.CreateSolidColorBrush(&color(0xffffff, 1.0), None)?;
        let blue = target.CreateSolidColorBrush(&color(0x4f8cff, 1.0), None)?;
        target.FillRoundedRectangle(&rounded(2.0, 2.0, 60.0, 60.0, 16.0), &graphite);
        let card = bubble(target, area(13.0, 12.0, 51.0, 36.0), 7.0, (5.0, 6.0))?;
        target.FillGeometry(&card, &white, None);
        target.FillRoundedRectangle(&rounded(19.0, 19.5, 18.0, 4.0, 2.0), &graphite);
        target.FillRoundedRectangle(&rounded(19.0, 26.5, 12.0, 3.5, 1.75), &faint);
        target.FillRoundedRectangle(&rounded(20.0, 47.0, 24.0, 5.0, 2.5), &blue);
    }
    Ok(())
}

/// The tray glyph on a 16-unit grid: the card's outline over the word's underline.
fn draw_tray(target: &ID2D1RenderTarget, ink: u32) -> Result<()> {
    unsafe {
        let solid = target.CreateSolidColorBrush(&color(ink, 1.0), None)?;
        let card = bubble(target, area(2.5, 2.25, 13.5, 9.75), 2.0, (1.25, 1.5))?;
        let round = target.GetFactory()?.CreateStrokeStyle(
            &D2D1_STROKE_STYLE_PROPERTIES {
                lineJoin: D2D1_LINE_JOIN_ROUND,
                ..Default::default()
            },
            None,
        )?;
        target.DrawGeometry(&card, &solid, 1.4, &round);
        target.FillRoundedRectangle(&rounded(4.5, 12.75, 7.0, 1.75, 0.875), &solid);
    }
    Ok(())
}

/// A top-down, premultiplied BGRA picture of `size` px, drawn on a `grid`-unit canvas.
struct Picture {
    dc: HDC,
    bitmap: HBITMAP,
    bits: *mut u8,
    size: u32,
}

impl Picture {
    fn draw(
        size: u32,
        grid: f32,
        paint: impl FnOnce(&ID2D1RenderTarget) -> Result<()>,
    ) -> Result<Self> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size as i32,
                    biHeight: -(size as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    let _ = DeleteDC(dc);
                    return Err(error);
                }
            };
            let picture = Self {
                dc,
                bitmap,
                bits: bits as *mut u8,
                size,
            };
            SelectObject(dc, bitmap.into());
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let target = factory.CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                ..Default::default()
            })?;
            let area = RECT {
                left: 0,
                top: 0,
                right: size as i32,
                bottom: size as i32,
            };
            target.BindDC(dc, &area)?;
            target.BeginDraw();
            target.Clear(Some(&color(0, 0.0)));
            let scale = size as f32 / grid;
            target.SetTransform(&Matrix3x2::scale(scale, scale));
            let drawn = paint(&target.cast::<ID2D1RenderTarget>()?);
            target.EndDraw(None, None)?;
            drawn?;
            let _ = GdiFlush();
            Ok(picture)
        }
    }

    fn pixels(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.size * self.size * 4) as usize) }
    }

    fn pixels_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.size * self.size * 4) as usize) }
    }
}

impl Drop for Picture {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Icon bitmaps carry straight alpha; Direct2D draws premultiplied.
fn unpremultiply(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = pixel[3] as u32;
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

/// The tray glyph at `size` px: near-black on a light taskbar, white on a dark one.
/// The caller destroys the icon.
pub fn tray(size: u32, light_taskbar: bool) -> Result<HICON> {
    let ink = if light_taskbar { 0x1a1a1a } else { 0xffffff };
    let mut picture = Picture::draw(size, 16.0, |target| draw_tray(target, ink))?;
    unpremultiply(picture.pixels_mut());
    unsafe {
        // All zero: the alpha channel alone decides what shows.
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: BOOL(1),
            hbmMask: mask,
            hbmColor: picture.bitmap,
            ..Default::default()
        });
        let _ = DeleteObject(mask.into());
        icon
    }
}

/// One PNG of the app icon, through the Windows imaging codecs; only `--export-icon` uses it.
fn app_png(size: u32) -> Result<Vec<u8>> {
    let picture = Picture::draw(size, 64.0, draw_app)?;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let bitmap = factory.CreateBitmapFromMemory(
            size,
            size,
            &GUID_WICPixelFormat32bppPBGRA,
            size * 4,
            picture.pixels(),
        )?;
        let stream = SHCreateMemStream(None).ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        frame.Initialize(None)?;
        frame.SetSize(size, size)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(&bitmap, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
        let mut stat = Default::default();
        stream.Stat(&mut stat, STATFLAG_NONAME)?;
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut png = vec![0u8; stat.cbSize as usize];
        let mut read = 0;
        stream
            .Read(
                png.as_mut_ptr() as *mut _,
                png.len() as u32,
                Some(&mut read),
            )
            .ok()?;
        png.truncate(read as usize);
        Ok(png)
    }
}

/// An ICO holding one PNG per size.
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in images {
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}

pub fn export(path: &Path) -> Result<()> {
    let images = APP_SIZES
        .iter()
        .map(|&size| app_png(size).map(|png| (size, png)))
        .collect::<Result<Vec<_>>>()?;
    std::fs::write(path, ico(&images))
        .map_err(|error| windows::core::Error::new(E_FAIL, error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tray_glyph_is_a_card_outline_over_a_solid_underline() {
        for light in [true, false] {
            let picture = Picture::draw(16, 16.0, |target| {
                draw_tray(target, if light { 0x1a1a1a } else { 0xffffff })
            })
            .unwrap();
            let alpha = |x: u32, y: u32| picture.pixels()[((y * 16 + x) * 4 + 3) as usize];
            assert!(alpha(8, 13) > 200, "the underlined word");
            assert!(alpha(8, 2) > 150, "the card's top edge");
            assert_eq!(alpha(8, 6), 0, "inside the card");
            assert_eq!(alpha(0, 0), 0);
            assert_eq!(alpha(15, 15), 0);
        }
        assert!(!tray(16, true).unwrap().is_invalid());
    }

    #[test]
    fn straight_alpha_restores_the_color() {
        let mut pixels = [128, 128, 128, 128, 0, 0, 0, 0, 255, 255, 255, 255];
        unpremultiply(&mut pixels);
        assert_eq!(pixels, [255, 255, 255, 128, 0, 0, 0, 0, 255, 255, 255, 255]);
    }

    #[test]
    fn the_ico_lists_every_image_where_it_is() {
        let file = ico(&[(16, vec![1, 2, 3]), (256, vec![4, 5])]);
        assert_eq!(&file[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(file[6], 16);
        assert_eq!(file[22], 0, "256 is written as 0");
        let offset = |entry: usize| {
            u32::from_le_bytes(
                file[6 + 16 * entry + 12..6 + 16 * entry + 16]
                    .try_into()
                    .unwrap(),
            ) as usize
        };
        assert_eq!(&file[offset(0)..offset(0) + 3], &[1, 2, 3]);
        assert_eq!(&file[offset(1)..], &[4, 5]);
    }

    #[test]
    fn the_app_icon_exports_as_png_images() {
        let png = app_png(32).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
