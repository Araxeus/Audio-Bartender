//! The card-style tray tooltips, drawn with Direct2D into a layered window so they get
//! rounded corners, a soft shadow, album art and the app's theme.
//!
//! Cards take the mouse: media buttons send keys, and the volume bar sets the master
//! volume on click or drag. A card never takes focus, so this doesn't pull the user
//! out of their app.
//!
//! Everything here runs on the tray thread.

use std::cell::RefCell;
use std::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering::Relaxed};
use std::time::Instant;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED,
    D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_BRUSH_PROPERTIES, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory,
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
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GWL_EXSTYLE, GetCursorPos, GetWindowLongPtrW, IDC_ARROW, KillTimer, LoadCursorW,
    MA_NOACTIVATE, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SetTimer, SetWindowLongPtrW, ShowWindow, ULW_ALPHA,
    UpdateLayeredWindow, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_TIMER, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};
use windows_numerics::{Matrix3x2, Vector2};

use crate::{Anchor, AppIcon, MediaButtons, MediaKey, Playback, TooltipStyle, mix};

/// What a card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Content {
    pub style: TooltipStyle,
    pub buttons: MediaButtons,
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
/// Width of the button dock on the compact Player card.
const DOCK: f32 = 102.;
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
    /// The window takes the mouse. Plain text tooltips don't.
    clickable: bool,
    /// The pointer is on the card, and since when, for the Hover buttons' fade.
    hovered: bool,
    hover_at: Instant,
    /// Asked Windows for a `WM_MOUSELEAVE`.
    tracking: bool,
    /// Index of the button under the pointer, and of the one held down.
    hot: Option<usize>,
    pressed: Option<usize>,
    /// The pointer is on the volume bar.
    bar_hot: bool,
    /// The level being dragged, shown until the audio state catches up.
    dragging: Option<f32>,
}

/// What a mouse event asks for.
enum Action {
    Press(MediaKey),
    Volume(f32),
}

/// Hover and press state for one frame.
#[derive(Clone, Copy, Default)]
struct Ui {
    hot: Option<usize>,
    pressed: Option<usize>,
    bar_hot: bool,
    dragging: Option<f32>,
    /// How far the Hover buttons have faded in, 0 to 1.
    reveal: f32,
}

/// Long enough ago that a fade has finished.
fn settled() -> Instant {
    let now = Instant::now();
    now.checked_sub(std::time::Duration::from_secs(1)).unwrap_or(now)
}

/// The card's screen rectangle without its shadow, all zero while hidden. The
/// mouse hook reads it from its own thread.
static SCREEN: [AtomicI32; 4] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];

thread_local! {
    static CARD: RefCell<Option<Card>> = const { RefCell::new(None) };
}

/// Creates the card window. Call once on the tray thread.
pub fn create() {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let class = w!("AudioBartender.Card");
        // Without a class cursor the pointer keeps whatever shape it last had.
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
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
                clickable: false,
                hovered: false,
                hover_at: settled(),
                tracking: false,
                hot: None,
                pressed: None,
                bar_hot: false,
                dragging: None,
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
        card.set_clickable(true);
        if !card.visible {
            card.visible = true;
            card.shown_at = Instant::now();
        }
        card.draw();
        unsafe {
            let _ = ShowWindow(card.hwnd, SW_SHOWNOACTIVATE);
        }
        crate::util::raise_topmost(card.hwnd);
        card.schedule();
    });
}

pub fn hide() {
    CARD.with_borrow_mut(|card| {
        let Some(card) = card else { return };
        card.visible = false;
        SCREEN.iter().for_each(|v| v.store(0, Relaxed));
        card.bar_hot = false;
        if card.pressed.take().is_some() | card.dragging.take().is_some() {
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        card.hot = None;
        card.hovered = false;
        card.hover_at = settled();
        unsafe {
            let _ = KillTimer(Some(card.hwnd), TIMER_FRAME);
            let _ = ShowWindow(card.hwnd, SW_HIDE);
        }
    });
}

/// True while a card shows, so the pointer may be on its way to it.
pub fn interactive() -> bool {
    CARD.with(|card| card.try_borrow().is_ok_and(|c| c.as_ref().is_some_and(|c| c.visible && c.clickable)))
}

/// True while the pointer is on the card.
pub fn pointer_on_card() -> bool {
    let mut at = POINT::default();
    unsafe { GetCursorPos(&mut at) }.is_ok() && contains(at)
}

/// True when `at` is on the showing card. Safe to call from any thread.
pub fn contains(at: POINT) -> bool {
    let [l, t, r, b] = SCREEN.each_ref().map(|v| v.load(Relaxed));
    at.x >= l && at.x < r && at.y >= t && at.y < b
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == TIMER_FRAME => {
            CARD.with_borrow_mut(|card| {
                if let Some(card) = card
                    && card.visible
                {
                    card.draw();
                    card.schedule();
                }
            });
            LRESULT(0)
        }
        // Clicks must not pull focus from the app the user is in.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE | WM_MOUSELEAVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
            let action = CARD.with_borrow_mut(|card| card.as_mut().and_then(|card| card.mouse(msg, lparam)));
            match action {
                Some(Action::Press(key)) => crate::input::press(key),
                Some(Action::Volume(level)) => crate::input::set_master_from_card(level),
                None => {}
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

impl Card {
    /// Lets the window take the mouse, or pass clicks through like a tooltip.
    fn set_clickable(&mut self, on: bool) {
        if self.clickable == on {
            return;
        }
        self.clickable = on;
        unsafe {
            let style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let transparent = WS_EX_TRANSPARENT.0 as isize;
            let style = if on { style & !transparent } else { style | transparent };
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, style);
        }
    }

    fn reveal(&self) -> f32 {
        let k = (self.hover_at.elapsed().as_secs_f32() * 1000. / FADE_MS).min(1.);
        if self.hovered { k } else { 1. - k }
    }

    /// Updates hover, press and drag state. Returns what the event asks for: a key
    /// from a finished click, or a volume from the bar.
    fn mouse(&mut self, msg: u32, lparam: LPARAM) -> Option<Action> {
        let content = self.content.as_ref()?;
        let scale = self.anchor.scale.max(1.0);
        let x = (lparam.0 & 0xffff) as u16 as i16 as f32 / scale;
        let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as f32 / scale;
        let buttons = buttons(content);
        let hit =
            buttons.iter().position(|b| x >= b.rect.left && x < b.rect.right && y >= b.rect.top && y < b.rect.bottom);
        let bar = volume_bar(content);
        // A generous target around the thin bar, and the level under the pointer.
        let on_bar = bar.is_some_and(|b| x >= b.left - 6. && x < b.right + 6. && y >= b.top - 9. && y < b.bottom + 9.);
        let level_at = |b: D2D_RECT_F| (((x - b.left) / (b.right - b.left)).clamp(0., 1.) * 100.).round() / 100.;
        let before = (self.hot, self.pressed, self.hovered, self.bar_hot, self.dragging);
        let mut key = None;
        match msg {
            WM_MOUSEMOVE => {
                if !self.tracking {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    self.tracking = unsafe { TrackMouseEvent(&mut track) }.is_ok();
                }
                if !self.hovered {
                    self.hovered = true;
                    self.hover_at = Instant::now();
                }
                self.hot = hit;
                self.bar_hot = on_bar || self.dragging.is_some();
                if self.dragging.is_some()
                    && let Some(b) = bar
                {
                    let level = level_at(b);
                    if self.dragging != Some(level) {
                        self.dragging = Some(level);
                        key = Some(Action::Volume(level));
                    }
                }
            }
            WM_MOUSELEAVE => {
                self.tracking = false;
                self.hovered = false;
                self.hover_at = Instant::now();
                self.hot = None;
                self.bar_hot = self.dragging.is_some();
            }
            WM_LBUTTONDOWN => {
                self.pressed = hit;
                if hit.is_some() {
                    unsafe { SetCapture(self.hwnd) };
                } else if on_bar && let Some(b) = bar {
                    let level = level_at(b);
                    self.dragging = Some(level);
                    unsafe { SetCapture(self.hwnd) };
                    key = Some(Action::Volume(level));
                }
            }
            _ => {
                if let Some(i) = self.pressed.take() {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    if hit == Some(i) {
                        key = Some(Action::Press(buttons[i].key));
                    }
                }
                if self.dragging.take().is_some() {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                }
                self.hot = hit;
                self.bar_hot = on_bar;
            }
        }
        if before != (self.hot, self.pressed, self.hovered, self.bar_hot, self.dragging) && self.visible {
            self.draw();
            self.schedule();
        }
        key
    }

    /// Keeps frames coming while the card or its Hover buttons fade, or its
    /// playing bars move.
    fn schedule(&self) {
        let fading = self.shown_at.elapsed().as_secs_f32() * 1000. < FADE_MS;
        let revealing = self.hover_at.elapsed().as_secs_f32() * 1000. < FADE_MS;
        let playing = self.content.as_ref().is_some_and(|c| c.media.playback == Playback::Playing && c.media.running);
        unsafe {
            if self.visible && (fading || revealing || playing) {
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
        let position = position(&self.anchor, size, scale);
        let shadow = (SHADOW * scale) as i32;
        let card =
            [position.x + shadow, position.y + shadow, position.x + size.0 - shadow, position.y + size.1 - shadow];
        for (slot, value) in SCREEN.iter().zip(card) {
            slot.store(value, Relaxed);
        }
        let ui = Ui {
            hot: self.hot,
            pressed: self.pressed,
            bar_hot: self.bar_hot,
            dragging: self.dragging,
            reveal: self.reveal(),
        };
        let Some(painter) = &mut self.painter else { return };
        if painter.paint(self.hwnd, &content, size, scale, t, ui, position, alpha).is_err() {
            // A lost device makes the next draw start over.
            self.painter = None;
        }
    }
}

/// Where the card for `content` would show by the icon that `anchor` describes,
/// without its shadow.
pub fn screen_rect(content: &Content, anchor: &Anchor) -> RECT {
    let scale = anchor.scale.max(1.0);
    let (width, height) = card_size(content);
    let size = (((width + 2. * SHADOW) * scale).ceil() as i32, ((height + 2. * SHADOW) * scale).ceil() as i32);
    let at = position(anchor, size, scale);
    let shadow = (SHADOW * scale) as i32;
    RECT { left: at.x + shadow, top: at.y + shadow, right: at.x + size.0 - shadow, bottom: at.y + size.1 - shadow }
}

/// Top-left corner of the window: the card centered on the icon, just off the
/// taskbar and inside the work area.
fn position(anchor: &Anchor, (w, h): (i32, i32), scale: f32) -> POINT {
    let (i, work) = (anchor.icon, anchor.work);
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

/// The buttons a card shows. An app that isn't running has nothing to control.
fn buttons_style(content: &Content) -> MediaButtons {
    let media = &content.media;
    if media.app.is_none() || media.running { content.buttons } else { MediaButtons::None }
}

fn card_size(content: &Content) -> (f32, f32) {
    let buttons = buttons_style(content);
    match content.style {
        TooltipStyle::Compact => match buttons {
            MediaButtons::Capsule => (COMPACT_WIDTH, 108.),
            MediaButtons::Player => (COMPACT_WIDTH + 70., 78.),
            _ => (COMPACT_WIDTH, 78.),
        },
        _ => {
            let (divider, gap) = divider(buttons);
            (NOW_PLAYING_WIDTH, divider + gap + 43.)
        }
    }
}

/// On the Now Playing card: the divider's offset from the card's top, and the gap
/// from it to the volume line.
fn divider(buttons: MediaButtons) -> (f32, f32) {
    match buttons {
        MediaButtons::Capsule => (86., 18.),
        MediaButtons::Player => (120., 11.),
        _ => (80., 11.),
    }
}

/// The volume bar's track, in window DIPs. Drawing and hit testing both use it.
fn volume_bar(content: &Content) -> Option<D2D_RECT_F> {
    let (width, height) = card_size(content);
    let card = rect(SHADOW, SHADOW, SHADOW + width, SHADOW + height);
    let style = buttons_style(content);
    let x = card.left + 12.;
    let (y, right) = match content.style {
        TooltipStyle::Text => return None,
        TooltipStyle::Compact => (card.top + 12., compact_right(&card, style)),
        _ => {
            let (offset, gap) = divider(style);
            (card.top + offset + gap, card.right - 12.)
        }
    };
    let top = y + 26.;
    Some(rect(x, top, right, top + 4.))
}

/// The media line on the compact card.
fn compact_line(card: &D2D_RECT_F, buttons: MediaButtons) -> f32 {
    card.top + if buttons == MediaButtons::Capsule { 78. } else { 48. }
}

/// Right edge of the compact card's content, left of the dock.
fn compact_right(card: &D2D_RECT_F, buttons: MediaButtons) -> f32 {
    card.right - 12. - if buttons == MediaButtons::Player { DOCK } else { 0. }
}

/// Title and subtitle of the Now Playing card.
fn titles(media: &Media) -> (String, Option<String>) {
    match (&media.app, &media.track) {
        (None, _) => ("Automatic".to_owned(), Some("Media keys go where Windows decides".to_owned())),
        (Some(app), _) if !media.running => (app.clone(), None),
        (Some(_), Some(track)) => match track.split_once(" — ") {
            Some((title, artist)) => (title.to_owned(), Some(artist.to_owned())),
            None => (track.clone(), None),
        },
        (Some(app), None) => (app.clone(), None),
    }
}

/// Offset of the status line from the top of the art, with or without a subtitle above it.
fn status_top(subtitle: bool) -> f32 {
    let lines = 2. + subtitle as u8 as f32;
    (ART - (20. + 18. * (lines - 1.))) / 2. + 20. + if subtitle { 18. } else { 0. }
}

/// Room the Inline buttons take at the end of a line.
const INLINE: f32 = 74.;

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    /// Muted icon that brightens on hover.
    Muted,
    /// Accent icon.
    Accent,
    /// White icon on an accent fill.
    Filled,
    /// White icon over album art.
    Overlay,
}

#[derive(Clone, Copy)]
struct Button {
    key: MediaKey,
    rect: D2D_RECT_F,
    radius: f32,
    tone: Tone,
    /// Icon size.
    icon: f32,
}

/// One button's look: width, height, corner radius, tone and icon size.
#[derive(Clone, Copy)]
struct Spec(f32, f32, f32, Tone, f32);

/// Previous, play/pause and next, centered on `cx`, `gap` apart.
fn trio(cx: f32, cy: f32, gap: f32, skip: Spec, main: Spec) -> Vec<Button> {
    let make = |key, cx: f32, s: Spec| Button {
        key,
        rect: rect(cx - s.0 / 2., cy - s.1 / 2., cx + s.0 / 2., cy + s.1 / 2.),
        radius: s.2,
        tone: s.3,
        icon: s.4,
    };
    let off = main.0 / 2. + gap + skip.0 / 2.;
    vec![
        make(MediaKey::Previous, cx - off, skip),
        make(MediaKey::PlayPause, cx, main),
        make(MediaKey::Next, cx + off, skip),
    ]
}

/// The card's buttons, in window DIPs. Drawing and hit testing both use these.
fn buttons(content: &Content) -> Vec<Button> {
    let style = buttons_style(content);
    let (width, height) = card_size(content);
    let card = rect(SHADOW, SHADOW, SHADOW + width, SHADOW + height);
    let (x, y) = (card.left + 12., card.top + 12.);
    let center = (card.left + card.right) / 2.;
    let inline = |right: f32, cy: f32| {
        trio(right + 4. - 35., cy, 2., Spec(22., 22., 5., Tone::Muted, 10.), Spec(22., 22., 11., Tone::Filled, 10.))
    };
    let capsule =
        |cy: f32| trio(center, cy, 2., Spec(26., 24., 12., Tone::Muted, 11.), Spec(34., 24., 12., Tone::Accent, 12.));
    match (content.style, style) {
        (_, MediaButtons::None) => Vec::new(),
        (TooltipStyle::Compact, _) => {
            let right = compact_right(&card, style);
            let line = compact_line(&card, style);
            match style {
                MediaButtons::Hover => {
                    let cx = (x + 26. + right + 4.) / 2.;
                    trio(
                        cx,
                        line + 9.,
                        6.,
                        Spec(40., 24., 6., Tone::Muted, 11.),
                        Spec(48., 24., 12., Tone::Filled, 12.),
                    )
                }
                MediaButtons::Inline => inline(right, line + 9.),
                MediaButtons::Capsule => capsule(card.top + 60.5),
                _ => {
                    let cy = (card.top + card.bottom) / 2.;
                    let skip = Spec(22., 30., 6., Tone::Muted, 11.);
                    trio(card.right - DOCK / 2., cy, 2., skip, Spec(34., 34., 17., Tone::Filled, 14.))
                }
            }
        }
        (_, MediaButtons::Hover) => {
            let skip = Spec(14., 24., 4., Tone::Overlay, 10.);
            trio(x + ART / 2., y + ART / 2., 0., skip, Spec(24., 24., 12., Tone::Overlay, 14.))
        }
        (_, MediaButtons::Inline) => {
            let subtitle = titles(&content.media).1.is_some();
            inline(card.right - 12., y + status_top(subtitle) + 9.)
        }
        (_, MediaButtons::Capsule) => capsule(card.top + divider(style).0 + 0.5),
        _ => {
            let skip = Spec(30., 30., 15., Tone::Muted, 14.);
            trio(center, card.top + 92., 14., skip, Spec(36., 36., 18., Tone::Filled, 15.))
        }
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
    factory: ID2D1Factory,
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
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
            let format = |family: PCWSTR, weight: DWRITE_FONT_WEIGHT, size: f32| -> Option<IDWriteTextFormat> {
                let format = dwrite
                    .CreateTextFormat(
                        family,
                        None,
                        weight,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size,
                        w!("en-us"),
                    )
                    .ok()?;
                let _ = format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                let trimming = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
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
            Some(Painter { factory, target, dwrite, wic, formats, image: None })
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
        ui: Ui,
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
                self.draw(content, t, ui);
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

    /// Fills a closed shape through `points`.
    fn polygon(&self, points: &[Vector2], rgb: u32, alpha: f32) {
        let Some((first, rest)) = points.split_first() else { return };
        unsafe {
            let Ok(geometry) = self.factory.CreatePathGeometry() else { return };
            let Ok(sink) = geometry.Open() else { return };
            sink.BeginFigure(*first, D2D1_FIGURE_BEGIN_FILLED);
            for point in rest {
                sink.AddLine(*point);
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            if sink.Close().is_err() {
                return;
            }
            if let Ok(brush) = self.target.CreateSolidColorBrush(&color(rgb, alpha), None) {
                self.target.FillGeometry(&geometry, &brush, None);
            }
        }
    }

    /// A media key's icon, drawn in a 16-unit box scaled to `size` around (`cx`, `cy`).
    #[allow(clippy::too_many_arguments)]
    fn icon(&self, key: MediaKey, playing: bool, cx: f32, cy: f32, size: f32, rgb: u32, alpha: f32) {
        let s = size / 16.;
        let (ox, oy) = (cx - size / 2., cy - size / 2.);
        let at = |x: f32, y: f32| Vector2 { X: ox + x * s, Y: oy + y * s };
        let bar = |l: f32, t: f32, r: f32, b: f32, radius: f32| {
            self.fill(rounded(rect(ox + l * s, oy + t * s, ox + r * s, oy + b * s), radius * s), rgb, alpha)
        };
        match key {
            MediaKey::PlayPause if playing => {
                bar(3., 2., 6.6, 14., 1.2);
                bar(9.4, 2., 13., 14., 1.2);
            }
            // Centered by its edges: as far from the left as its tip is from the right.
            MediaKey::PlayPause => self.polygon(&[at(3.15, 2.4), at(12.85, 8.), at(3.15, 13.6)], rgb, alpha),
            MediaKey::Previous => {
                bar(2., 2.5, 4., 13.5, 1.);
                self.polygon(&[at(14., 2.8), at(5., 8.), at(14., 13.2)], rgb, alpha);
            }
            MediaKey::Next => {
                bar(12., 2.5, 14., 13.5, 1.);
                self.polygon(&[at(2., 2.8), at(11., 8.), at(2., 13.2)], rgb, alpha);
            }
            MediaKey::Stop => bar(3., 3., 13., 13., 1.5),
        }
    }

    /// One button: its fill or hover tint, then its icon.
    #[allow(clippy::too_many_arguments)]
    fn button(&self, b: &Button, p: &Palette, content: &Content, playing: bool, hot: bool, pressed: bool, alpha: f32) {
        let shape = rounded(b.rect, b.radius);
        let tint = if pressed {
            0.14
        } else if hot {
            0.08
        } else {
            0.
        };
        let (ink, ink_alpha) = if content.dark { (0xffffff, tint) } else { (0, tint * 0.8) };
        let fg = match b.tone {
            Tone::Filled => {
                let accent = content.accent;
                // A soft glow under the big ones.
                if b.rect.right - b.rect.left >= 34. {
                    for i in 1..5 {
                        let spread = i as f32 * 1.5;
                        let r = rect(
                            b.rect.left - spread,
                            b.rect.top - spread + 2.,
                            b.rect.right + spread,
                            b.rect.bottom + spread + 2.,
                        );
                        self.fill(rounded(r, b.radius + spread), accent, alpha * 0.06);
                    }
                }
                let fill = if pressed {
                    mix(accent, 0, 0.12)
                } else if hot {
                    mix(accent, 0xffffff, 0.15)
                } else {
                    accent
                };
                self.fill(shape, fill, alpha);
                0xffffff
            }
            Tone::Overlay => {
                if tint > 0. {
                    self.fill(shape, 0xffffff, alpha * tint * 2.);
                }
                0xffffff
            }
            tone => {
                if tint > 0. {
                    self.fill(shape, ink, alpha * ink_alpha);
                }
                match tone {
                    Tone::Accent => content.accent,
                    _ if hot || pressed => p.text,
                    _ => p.muted,
                }
            }
        };
        let (cx, cy) = ((b.rect.left + b.rect.right) / 2., (b.rect.top + b.rect.bottom) / 2.);
        self.icon(b.key, playing, cx, cy, b.icon, fg, alpha);
    }

    /// The pill behind the Capsule buttons, with a small shadow.
    fn capsule(&self, buttons: &[Button], p: &Palette, dark: bool) {
        let (Some(first), Some(last)) = (buttons.first(), buttons.last()) else { return };
        let top = buttons.iter().map(|b| b.rect.top).fold(f32::MAX, f32::min);
        let bottom = buttons.iter().map(|b| b.rect.bottom).fold(f32::MIN, f32::max);
        let pill = rect(first.rect.left - 4., top - 4., last.rect.right + 4., bottom + 4.);
        let radius = (pill.bottom - pill.top) / 2.;
        for i in 0..4 {
            let spread = 4. - i as f32;
            let r = rect(pill.left - spread, pill.top - spread + 2., pill.right + spread, pill.bottom + spread + 2.);
            self.fill(rounded(r, radius + spread), 0, if dark { 0.06 } else { 0.025 });
        }
        self.fill(rounded(pill, radius), p.border, 1.);
        let inner = rect(pill.left + 1., pill.top + 1., pill.right - 1., pill.bottom - 1.);
        self.fill(rounded(inner, radius - 1.), p.card, 1.);
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

    fn draw(&mut self, content: &Content, t: f32, ui: Ui) {
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
        let style = buttons_style(content);
        let buttons = buttons(content);
        let mut alpha = 1.;
        match content.style {
            TooltipStyle::Compact => {
                let right = compact_right(&card, style);
                let line = compact_line(&card, style);
                self.volume(content, p, x, y, right, ui);
                self.avatar(&content.media, content.accent, x, line, 18.);
                let status_right = if style == MediaButtons::Inline { right + 4. - INLINE } else { right };
                self.status(&content.media, p, x + 26., line, status_right, true, false, t);
                match style {
                    MediaButtons::Capsule => {
                        let seam = card.top + 60.;
                        self.fill(rounded(rect(card.left, seam, card.right, seam + 1.), 0.), p.border, 1.);
                        self.capsule(&buttons, p, content.dark);
                    }
                    MediaButtons::Player => {
                        // Opaque, so the square patch over the rounded left corners blends in.
                        let dock = card.right - DOCK;
                        let tint = mix(p.card, p.text, 0.03);
                        self.fill(rounded(rect(dock, card.top + 1., card.right - 1., card.bottom - 1.), 7.), tint, 1.);
                        self.fill(rounded(rect(dock, card.top + 1., dock + 8., card.bottom - 1.), 0.), tint, 1.);
                        self.fill(rounded(rect(dock, card.top + 1., dock + 1., card.bottom - 1.), 0.), p.border, 1.);
                    }
                    MediaButtons::Hover => {
                        // The status fades out under the buttons.
                        alpha = ui.reveal;
                        self.fill(rounded(rect(x + 22., line - 3., right + 4., line + 21.), 4.), p.card, alpha);
                    }
                    _ => {}
                }
            }
            _ => {
                let right = card.right - 12.;
                self.avatar(&content.media, content.accent, x, y, ART);
                let inline = style == MediaButtons::Inline;
                let status_right = if inline { right + 4. - INLINE } else { right };
                self.now_playing(&content.media, p, x + ART + 12., y, right, status_right, inline, t);
                let (offset, gap) = divider(style);
                let divider = card.top + offset;
                self.fill(rounded(rect(card.left, divider, card.right, divider + 1.), 0.), p.border, 1.);
                self.volume(content, p, x, divider + gap, right, ui);
                match style {
                    MediaButtons::Capsule => self.capsule(&buttons, p, content.dark),
                    MediaButtons::Hover => {
                        alpha = ui.reveal;
                        let art = rect(x, y, x + ART, y + ART);
                        self.fill(rounded(art, (ART * 0.12).max(4.)), 0x0a0a10, 0.52 * alpha);
                    }
                    _ => {}
                }
            }
        }
        if alpha <= 0. {
            return;
        }
        let playing = content.media.playback == Playback::Playing && content.media.running;
        for (i, b) in buttons.iter().enumerate() {
            let pressed = ui.pressed == Some(i);
            let hot = ui.hot == Some(i) && (ui.pressed.is_none() || pressed);
            self.button(b, p, content, playing, hot, pressed, alpha);
        }
    }

    /// Glyph, device and percentage on one line, then the volume bar. A hovered or
    /// dragged bar grows and shows a thumb, so it reads as something to grab.
    #[allow(clippy::too_many_arguments)]
    fn volume(&self, content: &Content, p: &Palette, x: f32, y: f32, right: f32, ui: Ui) {
        let level = ui.dragging.unwrap_or(content.level).clamp(0., 1.);
        let muted = content.muted && ui.dragging.is_none_or(|l| l == 0.);
        let tint = if muted { p.danger } else { p.text };
        self.text(volume_glyph(level, muted), &self.formats.glyph, rect(x, y, x + 18., y + 20.), tint);
        let value = if muted { "Muted".to_owned() } else { format!("{}%", (level * 100.).round()) };
        self.text(&value, &self.formats.value, rect(right - 60., y, right, y + 20.), tint);
        let device = content.device.as_deref().unwrap_or("No output device");
        self.text(device, &self.formats.body, rect(x + 24., y, right - 44., y + 20.), p.text);

        let grow = if ui.bar_hot { 1. } else { 0. };
        let (top, bottom) = (y + 26. - grow, y + 30. + grow);
        let radius = (bottom - top) / 2.;
        self.fill(rounded(rect(x, top, right, bottom), radius), p.track, 1.);
        let fill = x + (right - x) * level;
        let fill_color = if muted { p.muted } else { content.accent };
        if fill > x {
            self.fill(rounded(rect(x, top, fill.max(x + 4.), bottom), radius), fill_color, 1.);
        }
        if ui.bar_hot {
            let (cx, cy) = (fill.clamp(x + 6., right - 6.), (top + bottom) / 2.);
            self.fill(rounded(rect(cx - 7., cy - 7., cx + 7., cy + 7.), 7.), p.card, 1.);
            self.fill(rounded(rect(cx - 5.5, cy - 5.5, cx + 5.5, cy + 5.5), 5.5), fill_color, 1.);
        }
    }

    /// Title, artist and status, stacked and centered beside the art. `reserve`
    /// keeps the status line's room for the Inline buttons even without a status.
    #[allow(clippy::too_many_arguments)]
    fn now_playing(
        &self,
        media: &Media,
        p: &Palette,
        x: f32,
        y: f32,
        right: f32,
        status_right: f32,
        reserve: bool,
        t: f32,
    ) {
        let (title, subtitle) = titles(media);
        let status = media.app.is_some();
        let lines = 1. + subtitle.is_some() as u8 as f32 + (status || reserve) as u8 as f32;
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
            self.status(media, p, x, line, status_right, false, trail, t);
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
            let decoder =
                self.wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
            let frame = decoder.GetFrame(0).ok()?;
            let converter = self.wic.CreateFormatConverter().ok()?;
            converter
                .Initialize(
                    &frame,
                    &GUID_WICPixelFormat32bppPBGRA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeMedianCut,
                )
                .ok()?;
            self.target.CreateBitmapFromWicBitmap(&converter, None).ok()?
        };
        self.image = Some((icon.data.clone(), bitmap.clone()));
        Some(bitmap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

    /// Draws `content` and returns its premultiplied BGRA pixels.
    fn render(painter: &mut Painter, content: &Content, ui: Ui, scale: f32) -> (i32, i32, Vec<u8>) {
        let (width, height) = card_size(content);
        let (w, h) = (((width + 2. * SHADOW) * scale).ceil() as i32, ((height + 2. * SHADOW) * scale).ceil() as i32);
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
            let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let old = SelectObject(dc, bitmap.into());
            painter.target.BindDC(dc, &RECT { left: 0, top: 0, right: w, bottom: h }).unwrap();
            painter.target.SetDpi(96. * scale, 96. * scale);
            painter.target.BeginDraw();
            painter.target.Clear(Some(&color(0, 0.)));
            painter.draw(content, 1.3, ui);
            painter.target.EndDraw(None, None).unwrap();
            let pixels = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
            SelectObject(dc, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            (w, h, pixels)
        }
    }

    /// A gradient square standing in for album art.
    fn art() -> AppIcon {
        let n = 64usize;
        let mut rgba = Vec::with_capacity(n * n * 4);
        for y in 0..n {
            for x in 0..n {
                let t = (x + y) as f32 / (2 * n) as f32;
                let c = if t < 0.5 { mix(0xf97316, 0xdb2777, t * 2.) } else { mix(0xdb2777, 0x4c1d95, t * 2. - 1.) };
                rgba.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
            }
        }
        let data = crate::icon::encode_png(n as u32, n as u32, &rgba);
        AppIcon { data: data.into(), kind: crate::ImageKind::Png, art: true }
    }

    /// Renders every card style with every button style into one PNG, for eyeballing:
    /// `CARD_PREVIEW=out.png cargo test -p audio_bartender_core card_preview -- --ignored`
    #[test]
    #[ignore]
    fn card_preview() {
        let Ok(out) = std::env::var("CARD_PREVIEW") else { return };
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let mut painter = Painter::new().unwrap();
        let scale = 1.5;
        let all = [
            MediaButtons::None,
            MediaButtons::Hover,
            MediaButtons::Inline,
            MediaButtons::Capsule,
            MediaButtons::Player,
        ];
        let media = Media {
            app: Some("Spotify".into()),
            playback: Playback::Playing,
            running: true,
            track: Some("Midnight City — M83".into()),
            icon: Some(art()),
        };
        let (cell_w, cell_h) =
            (((330. + 2. * SHADOW) * scale) as usize + 30, ((174. + 2. * SHADOW) * scale) as usize + 30);
        let (cols, rows) = (all.len(), 4);
        let (img_w, img_h) = (cell_w * cols, cell_h * rows);
        let mut img = vec![0u8; img_w * img_h * 4];
        for (row, (style, dark)) in [
            (TooltipStyle::NowPlaying, true),
            (TooltipStyle::NowPlaying, false),
            (TooltipStyle::Compact, true),
            (TooltipStyle::Compact, false),
        ]
        .into_iter()
        .enumerate()
        {
            let stage = if dark { 0x0d0f17 } else { 0xdfe3ec };
            for (col, buttons) in all.into_iter().enumerate() {
                let content = Content {
                    style,
                    buttons,
                    dark,
                    accent: if dark { 0x3b82f6 } else { 0x2563eb },
                    device: Some("Speakers (Realtek(R) Audio)".into()),
                    level: 0.62,
                    muted: false,
                    media: media.clone(),
                };
                // Next and the volume bar are hovered, and Hover buttons are fully shown.
                let ui = Ui { hot: Some(2), pressed: None, bar_hot: true, dragging: None, reveal: 1. };
                let (w, h, px) = render(&mut painter, &content, ui, scale);
                for y in 0..cell_h {
                    for x in 0..cell_w {
                        let (sx, sy) = (x as i32 - 15, y as i32 - 15);
                        let src = (sx >= 0 && sy >= 0 && sx < w && sy < h).then(|| ((sy * w + sx) * 4) as usize);
                        let o = (((row * cell_h + y) * img_w) + col * cell_w + x) * 4;
                        for (i, shift) in [16, 8, 0].into_iter().enumerate() {
                            let bg = ((stage >> shift) & 0xff) as f32;
                            img[o + i] = match src {
                                // BGRA, premultiplied: c + bg * (1 - a).
                                Some(s) => (px[s + 2 - i] as f32 + bg * (1. - px[s + 3] as f32 / 255.)) as u8,
                                None => bg as u8,
                            };
                        }
                        img[o + 3] = 255;
                    }
                }
            }
        }
        std::fs::write(out, crate::icon::encode_png(img_w as u32, img_h as u32, &img)).unwrap();
    }
}
