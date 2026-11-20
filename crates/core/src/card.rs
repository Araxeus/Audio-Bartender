//! The card-style tray tooltips, drawn with Direct2D into a layered window so they get
//! rounded corners, a soft shadow, album art and the app's theme.
//!
//! Everything here runs on the tray thread.

use std::cell::RefCell;
use std::f32::consts::TAU;
use std::sync::Arc;
use std::time::Instant;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_BRUSH_PROPERTIES, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap, ID2D1DCRenderTarget,
    ID2D1Factory,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, SelectObject,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory, WICBitmapDitherTypeNone,
    WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, KillTimer, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SetTimer, ShowWindow,
    ULW_ALPHA, UpdateLayeredWindow, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};
use windows_numerics::Matrix3x2;

use crate::{Anchor, AppIcon, Playback, TooltipStyle};

/// What a card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Content {
    pub style: TooltipStyle,
    pub dark: bool,
    /// The accent color, `0xRRGGBB`.
    pub accent: u32,
    pub device: Option<String>,
    pub level: f32,
    pub muted: bool,
    pub media: Media,
}

/// The app that gets the media keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Media {
    /// `None` when Windows decides.
    pub app: Option<String>,
    pub playback: Playback,
    pub running: bool,
    /// "Title — Artist".
    pub track: Option<String>,
    pub icon: Option<AppIcon>,
}

/// Card widths, in DIPs.
const NOW_PLAYING_WIDTH: f32 = 300.;
const COMPACT_WIDTH: f32 = 260.;
/// Room around the card for its shadow.
const SHADOW: f32 = 10.;
/// Gap between the card and the taskbar.
const GAP: f32 = 4.;
const ART: f32 = 56.;
const FADE_MS: f32 = 120.;
const TIMER_FRAME: usize = 1;

struct Palette {
    card: u32,
    border: u32,
    text: u32,
    muted: u32,
    track: u32,
    playing: u32,
    danger: u32,
}

const DARK: Palette = Palette {
    card: 0x1c1c25,
    border: 0x2f2f3d,
    text: 0xececf3,
    muted: 0x8e8ea3,
    track: 0x33333f,
    playing: 0x4ade80,
    danger: 0xf87171,
};

const LIGHT: Palette = Palette {
    card: 0xffffff,
    border: 0xe4e4ea,
    text: 0x1b1b22,
    muted: 0x6b6b78,
    track: 0xdcdce4,
    playing: 0x16a34a,
    danger: 0xdc2626,
};

const AVATAR_COLORS: [u32; 8] = [0x6d5ef5, 0x0ea5e9, 0xf97316, 0x10b981, 0xec4899, 0xeab308, 0xef4444, 0x14b8a6];

/// Segoe MDL2 Assets glyphs.
const GLYPH_MUSIC: &str = "\u{E8D6}";
const GLYPH_MUTE: &str = "\u{E74F}";

/// True when Windows uses the light theme for apps.
pub fn apps_light() -> bool {
    let mut value = 1u32;
    let mut size = 4u32;
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as _),
            Some(&mut size),
        )
    };
    found.is_err() || value == 1
}

struct Card {
    hwnd: HWND,
    painter: Option<Painter>,
    content: Option<Content>,
    anchor: Anchor,
    shown_at: Instant,
    visible: bool,
}

thread_local! {
    static CARD: RefCell<Option<Card>> = const { RefCell::new(None) };
}

/// Creates the card window. Call once on the tray thread.
pub fn create() {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let class = w!("MediaKeysSelector.Card");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            None,
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            return;
        };
        CARD.with_borrow_mut(|card| {
            *card = Some(Card {
                hwnd,
                painter: Painter::new(),
                content: None,
                anchor: Anchor::default(),
                shown_at: Instant::now(),
                visible: false,
            })
        });
    }
}

/// Shows the card for `content` next to the icon that `anchor` describes, or
/// redraws a showing card whose content changed.
pub fn show(content: &Content, anchor: Anchor) {
    CARD.with_borrow_mut(|card| {
        let Some(card) = card else { return };
        if card.visible && card.anchor == anchor && card.content.as_ref() == Some(content) {
            return;
        }
        card.anchor = anchor;
        card.content = Some(content.clone());
        if !card.visible {
            card.visible = true;
            card.shown_at = Instant::now();
        }
        card.draw();
        unsafe {
            let _ = ShowWindow(card.hwnd, SW_SHOWNOACTIVATE);
        }
        card.schedule();
    });
}

pub fn hide() {
    CARD.with_borrow_mut(|card| {
        let Some(card) = card else { return };
        card.visible = false;
        unsafe {
            let _ = KillTimer(Some(card.hwnd), TIMER_FRAME);
            let _ = ShowWindow(card.hwnd, SW_HIDE);
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_TIMER && wparam.0 == TIMER_FRAME {
        CARD.with_borrow_mut(|card| {
            if let Some(card) = card
                && card.visible
            {
                card.draw();
                card.schedule();
            }
        });
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl Card {
    /// Keeps frames coming while the card fades in or its playing bars move.
    fn schedule(&self) {
        let fading = self.shown_at.elapsed().as_secs_f32() * 1000. < FADE_MS;
        let playing = self.content.as_ref().is_some_and(|c| c.media.playback == Playback::Playing && c.media.running);
        unsafe {
            if self.visible && (fading || playing) {
                SetTimer(Some(self.hwnd), TIMER_FRAME, 16, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_FRAME);
            }
        }
    }

    fn draw(&mut self) {
        let Some(content) = self.content.clone() else { return };
        if self.painter.is_none() {
            self.painter = Painter::new();
        }
        let scale = self.anchor.scale.max(1.0);
        let t = self.shown_at.elapsed().as_secs_f32();
        let (width, height) = card_size(&content);
        let size = (((width + 2. * SHADOW) * scale).ceil() as i32, ((height + 2. * SHADOW) * scale).ceil() as i32);
        let alpha = ((t * 1000. / FADE_MS).min(1.0) * 255.) as u8;
        let position = self.position(size, scale);
        let Some(painter) = &mut self.painter else { return };
        if painter.paint(self.hwnd, &content, size, scale, t, position, alpha).is_err() {
            // A lost device makes the next draw start over.
            self.painter = None;
        }
    }

    /// Top-left corner of the window: the card centered on the icon, just off the
    /// taskbar and inside the work area.
    fn position(&self, (w, h): (i32, i32), scale: f32) -> POINT {
        let (i, work) = (self.anchor.icon, self.anchor.work);
        let shadow = (SHADOW * scale) as i32;
        let gap = (GAP * scale) as i32;
        let (card_w, card_h) = (w - 2 * shadow, h - 2 * shadow);
        let margin = (8. * scale) as i32;
        let clamp_x = |x: i32| x.clamp(work.left + margin, (work.right - card_w - margin).max(work.left + margin));
        let clamp_y = |y: i32| y.clamp(work.top + margin, (work.bottom - card_h - margin).max(work.top + margin));
        let center_x = (i.left + i.right) / 2 - card_w / 2;
        let center_y = (i.top + i.bottom) / 2 - card_h / 2;
        let (x, y) = if i.bottom <= work.top {
            (clamp_x(center_x), work.top + gap)
        } else if i.top < work.bottom && i.left >= work.right {
            (work.right - card_w - gap, clamp_y(center_y))
        } else if i.top < work.bottom && i.right <= work.left {
            (work.left + gap, clamp_y(center_y))
        } else if i.top >= work.bottom {
            (clamp_x(center_x), work.bottom - card_h - gap)
        } else {
            // The icon sits inside the work area (overflow): above it.
            (clamp_x(center_x), clamp_y(i.top - card_h - gap))
        };
        POINT { x: x - shadow, y: y - shadow }
    }
}

fn card_size(content: &Content) -> (f32, f32) {
    match content.style {
        TooltipStyle::Compact => (COMPACT_WIDTH, 78.),
        _ => (NOW_PLAYING_WIDTH, 134.),
    }
}

fn color(rgb: u32, alpha: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 0xff) as f32 / 255.,
        g: ((rgb >> 8) & 0xff) as f32 / 255.,
        b: (rgb & 0xff) as f32 / 255.,
        a: alpha,
    }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right, bottom }
}

fn rounded(r: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Same hash as the front ends, so each app gets the same color everywhere.
fn avatar_color(name: &str) -> u32 {
    let hash = name.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32));
    AVATAR_COLORS[hash.unsigned_abs() as usize % AVATAR_COLORS.len()]
}

fn volume_glyph(level: f32, muted: bool) -> &'static str {
    if muted || level == 0.0 {
        GLYPH_MUTE
    } else if level < 0.34 {
        "\u{E993}"
    } else if level < 0.67 {
        "\u{E994}"
    } else {
        "\u{E995}"
    }
}

struct Formats {
    title: IDWriteTextFormat,
    body: IDWriteTextFormat,
    strong: IDWriteTextFormat,
    value: IDWriteTextFormat,
    glyph: IDWriteTextFormat,
    letter: IDWriteTextFormat,
    small_letter: IDWriteTextFormat,
    small_glyph: IDWriteTextFormat,
    tile_glyph: IDWriteTextFormat,
}

struct Painter {
    target: ID2D1DCRenderTarget,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    formats: Formats,
    /// The decoded image of the last icon drawn, keyed by its data.
    image: Option<(Arc<[u8]>, ID2D1Bitmap)>,
}

impl Painter {
    fn new() -> Option<Painter> {
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 0.,
                dpiY: 0.,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let target = factory.CreateDCRenderTarget(&props).ok()?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
            let format = |family: PCWSTR, weight: DWRITE_FONT_WEIGHT, size: f32| -> Option<IDWriteTextFormat> {
                let format = dwrite
                    .CreateTextFormat(family, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("en-us"))
                    .ok()?;
                let _ = format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                if let Ok(sign) = dwrite.CreateEllipsisTrimmingSign(&format) {
                    let _ = format.SetTrimming(&trimming, &sign);
                }
                Some(format)
            };
            let ui = w!("Segoe UI");
            let mdl2 = w!("Segoe MDL2 Assets");
            let formats = Formats {
                title: format(ui, DWRITE_FONT_WEIGHT_SEMI_BOLD, 14.)?,
                body: format(ui, DWRITE_FONT_WEIGHT_NORMAL, 12.)?,
                strong: format(ui, DWRITE_FONT_WEIGHT_SEMI_BOLD, 12.)?,
                value: format(ui, DWRITE_FONT_WEIGHT_SEMI_BOLD, 12.)?,
                glyph: format(mdl2, DWRITE_FONT_WEIGHT_NORMAL, 14.)?,
                letter: format(ui, DWRITE_FONT_WEIGHT_SEMI_BOLD, 20.)?,
                small_letter: format(ui, DWRITE_FONT_WEIGHT_SEMI_BOLD, 11.)?,
                small_glyph: format(mdl2, DWRITE_FONT_WEIGHT_NORMAL, 11.)?,
                tile_glyph: format(mdl2, DWRITE_FONT_WEIGHT_NORMAL, 22.)?,
            };
            let _ = formats.value.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING);
            let _ = formats.letter.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = formats.small_letter.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = formats.tile_glyph.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = formats.small_glyph.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            Some(Painter { target, dwrite, wic, formats, image: None })
        }
    }

    /// Draws the card into a fresh bitmap and puts it on the layered window.
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &mut self,
        hwnd: HWND,
        content: &Content,
        (w, h): (i32, i32),
        scale: f32,
        t: f32,
        position: POINT,
        alpha: u8,
    ) -> windows::core::Result<()> {
        unsafe {
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let dc = CreateCompatibleDC(None);
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old = SelectObject(dc, bitmap.into());
            let result = (|| {
                self.target.BindDC(dc, &RECT { left: 0, top: 0, right: w, bottom: h })?;
                self.target.SetDpi(96. * scale, 96. * scale);
                self.target.BeginDraw();
                self.target.Clear(Some(&color(0, 0.)));
                self.draw(content, t);
                self.target.EndDraw(None, None)
            })();
            if result.is_ok() {
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: alpha,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let _ = UpdateLayeredWindow(
                    hwnd,
                    None,
                    Some(&position),
                    Some(&SIZE { cx: w, cy: h }),
                    Some(dc),
                    Some(&POINT::default()),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                );
            }
            SelectObject(dc, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            result
        }
    }

    fn fill(&self, r: D2D1_ROUNDED_RECT, rgb: u32, alpha: f32) {
        unsafe {
            if let Ok(brush) = self.target.CreateSolidColorBrush(&color(rgb, alpha), None) {
                self.target.FillRoundedRectangle(&r, &brush);
            }
        }
    }

    fn text(&self, text: &str, format: &IDWriteTextFormat, r: D2D_RECT_F, rgb: u32) {
        unsafe {
            if let Ok(brush) = self.target.CreateSolidColorBrush(&color(rgb, 1.), None) {
                self.target.DrawText(
                    &wide(text),
                    format,
                    &r,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
        }
    }

    /// Width of `text` in one line, up to `max`.
    fn measure(&self, text: &str, format: &IDWriteTextFormat, max: f32) -> f32 {
        unsafe {
            let Ok(layout) = self.dwrite.CreateTextLayout(&wide(text), format, max, 100.) else { return max };
            let mut metrics = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut metrics).is_err() {
                return max;
            }
            metrics.widthIncludingTrailingWhitespace.min(max)
        }
    }

    fn draw(&mut self, content: &Content, t: f32) {
        let p = if content.dark { &DARK } else { &LIGHT };
        let (width, height) = card_size(content);
        let card = rect(SHADOW, SHADOW, SHADOW + width, SHADOW + height);

        // A soft shadow from stacked translucent outlines, then the card and its border.
        for i in 0..8 {
            let spread = 8. - i as f32;
            let r = rect(card.left - spread, card.top - spread + 2., card.right + spread, card.bottom + spread + 2.);
            self.fill(rounded(r, 8. + spread), 0, if content.dark { 0.045 } else { 0.022 });
        }
        self.fill(rounded(card, 8.), p.border, 1.);
        let inner = rect(card.left + 1., card.top + 1., card.right - 1., card.bottom - 1.);
        self.fill(rounded(inner, 7.), p.card, 1.);

        let (x, y) = (card.left + 12., card.top + 12.);
        let right = card.right - 12.;
        match content.style {
            TooltipStyle::Compact => {
                self.volume(content, p, x, y, right);
                let line = y + 36.;
                self.avatar(&content.media, content.accent, x, line, 18.);
                self.status(&content.media, p, x + 26., line, right, true, false, t);
            }
            _ => {
                self.avatar(&content.media, content.accent, x, y, ART);
                self.now_playing(&content.media, p, x + ART + 12., y, right, t);
                let divider = y + ART + 12.;
                self.fill(rounded(rect(card.left, divider, card.right, divider + 1.), 0.), p.border, 1.);
                self.volume(content, p, x, divider + 11., right);
            }
        }
    }

    /// Glyph, device and percentage on one line, then the volume bar.
    fn volume(&self, content: &Content, p: &Palette, x: f32, y: f32, right: f32) {
        let tint = if content.muted { p.danger } else { p.text };
        self.text(volume_glyph(content.level, content.muted), &self.formats.glyph, rect(x, y, x + 18., y + 20.), tint);
        let value = if content.muted { "Muted".to_owned() } else { format!("{}%", (content.level * 100.).round()) };
        self.text(&value, &self.formats.value, rect(right - 60., y, right, y + 20.), tint);
        let device = content.device.as_deref().unwrap_or("No output device");
        self.text(device, &self.formats.body, rect(x + 24., y, right - 44., y + 20.), p.text);

        let bar = y + 26.;
        self.fill(rounded(rect(x, bar, right, bar + 4.), 2.), p.track, 1.);
        let fill = x + (right - x) * content.level.clamp(0., 1.);
        let fill_color = if content.muted { p.muted } else { content.accent };
        if fill > x {
            self.fill(rounded(rect(x, bar, fill.max(x + 4.), bar + 4.), 2.), fill_color, 1.);
        }
    }

    /// Title, artist and status, stacked and centered beside the art.
    fn now_playing(&self, media: &Media, p: &Palette, x: f32, y: f32, right: f32, t: f32) {
        let (title, subtitle) = match (&media.app, &media.track) {
            (None, _) => ("Automatic".to_owned(), Some("Media keys go where Windows decides".to_owned())),
            (Some(app), _) if !media.running => (app.clone(), None),
            (Some(_), Some(track)) => match track.split_once(" — ") {
                Some((title, artist)) => (title.to_owned(), Some(artist.to_owned())),
                None => (track.clone(), None),
            },
            (Some(app), None) => (app.clone(), None),
        };
        let status = media.app.is_some();
        let lines = 1. + subtitle.is_some() as u8 as f32 + status as u8 as f32;
        let mut line = y + (ART - (20. + 18. * (lines - 1.))) / 2.;
        self.text(&title, &self.formats.title, rect(x, line, right, line + 20.), p.text);
        line += 20.;
        if let Some(subtitle) = subtitle {
            self.text(&subtitle, &self.formats.body, rect(x, line, right, line + 18.), p.muted);
            line += 18.;
        }
        if status {
            // The app name only adds something when the title is the track.
            let trail = media.track.is_some() && media.running;
            self.status(media, p, x, line, right, false, trail, t);
        }
    }

    /// "▮▮▮ Playing", with the bars and state in green while playing. `lead` puts the
    /// app name first, as on the compact card; `trail` adds " · App" after the state.
    #[allow(clippy::too_many_arguments)]
    fn status(&self, media: &Media, p: &Palette, x: f32, y: f32, right: f32, lead: bool, trail: bool, t: f32) {
        let (mut x, h) = (x, 18.);
        let Some(app) = media.app.as_deref() else {
            self.text("Media keys: Automatic", &self.formats.body, rect(x, y, right, y + h), p.muted);
            return;
        };
        if lead {
            let width = self.measure(app, &self.formats.strong, right - x);
            self.text(app, &self.formats.strong, rect(x, y, right, y + h), p.text);
            x += width + 6.;
        }
        if !media.running {
            self.text("Not running", &self.formats.body, rect(x, y, right, y + h), p.muted);
            return;
        }
        let playing = media.playback == Playback::Playing;
        if playing {
            for i in 0..3 {
                let offset = [0.0, 0.6, 0.3][i];
                let phase = ((t + offset) % 1.0) * TAU;
                let bar = 3. + 7. * (0.5 - 0.5 * phase.cos());
                let left = x + i as f32 * 4.;
                let bottom = y + h / 2. + 5.;
                self.fill(rounded(rect(left, bottom - bar, left + 2., bottom), 0.), p.playing, 1.);
            }
            x += 16.;
        }
        let state = match media.playback {
            Playback::Playing => "Playing",
            Playback::Paused => "Paused",
            Playback::Stopped => "Stopped",
            Playback::Unknown => "Idle",
        };
        let width = self.measure(state, &self.formats.body, right - x);
        self.text(state, &self.formats.body, rect(x, y, right, y + h), if playing { p.playing } else { p.muted });
        if trail {
            self.text(&format!(" · {app}"), &self.formats.body, rect(x + width, y, right, y + h), p.muted);
        }
    }

    /// The app's art or icon, else its initial on a colored square, or the
    /// automatic tile.
    fn avatar(&mut self, media: &Media, accent: u32, x: f32, y: f32, size: f32) {
        let r = rect(x, y, x + size, y + size);
        let radius = (size * 0.12).max(4.);
        let Some(app) = &media.app else {
            self.fill(rounded(r, radius), accent, 1.);
            let format = if size > 30. { &self.formats.tile_glyph } else { &self.formats.small_glyph };
            self.text(GLYPH_MUSIC, format, r, 0xffffff);
            return;
        };
        if let Some(icon) = &media.icon
            && let Some(bitmap) = self.bitmap(icon)
        {
            unsafe {
                let size_px = bitmap.GetSize();
                let (iw, ih) = (size_px.width.max(1.), size_px.height.max(1.));
                // Art fills the square, cropped; app icons sit inside it.
                let (scale, area) = if icon.art {
                    ((size / iw).max(size / ih), r)
                } else {
                    let inset = size * 0.1;
                    let area = rect(x + inset, y + inset, x + size - inset, y + size - inset);
                    (((size - 2. * inset) / iw).min((size - 2. * inset) / ih), area)
                };
                let (dw, dh) = (iw * scale, ih * scale);
                let ox = (area.left + area.right - dw) / 2.;
                let oy = (area.top + area.bottom - dh) / 2.;
                let props = D2D1_BITMAP_BRUSH_PROPERTIES {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                };
                if let Ok(brush) = self.target.CreateBitmapBrush(&bitmap, Some(&props), None) {
                    brush.SetTransform(&Matrix3x2 { M11: scale, M12: 0., M21: 0., M22: scale, M31: ox, M32: oy });
                    let clip = if icon.art { r } else { rect(ox, oy, ox + dw, oy + dh) };
                    self.target.FillRoundedRectangle(&rounded(clip, if icon.art { radius } else { 0. }), &brush);
                }
            }
            return;
        }
        self.fill(rounded(r, radius), avatar_color(app), 1.);
        let initial = app.chars().next().unwrap_or('?').to_uppercase().to_string();
        let format = if size > 30. { &self.formats.letter } else { &self.formats.small_letter };
        self.text(&initial, format, r, 0xffffff);
    }

    /// Decodes an icon or album art, keeping the last one for the next frames.
    fn bitmap(&mut self, icon: &AppIcon) -> Option<ID2D1Bitmap> {
        if let Some((data, bitmap)) = &self.image
            && Arc::ptr_eq(data, &icon.data)
        {
            return Some(bitmap.clone());
        }
        let bitmap = unsafe {
            let stream = self.wic.CreateStream().ok()?;
            stream.InitializeFromMemory(&icon.data).ok()?;
            let decoder = self.wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
            let frame = decoder.GetFrame(0).ok()?;
            let converter = self.wic.CreateFormatConverter().ok()?;
            converter
                .Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)
                .ok()?;
            self.target.CreateBitmapFromWicBitmap(&converter, None).ok()?
        };
        self.image = Some((icon.data.clone(), bitmap.clone()));
        Some(bitmap)
    }
}
