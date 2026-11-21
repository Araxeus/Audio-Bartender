//! Draws the tray icon as a volume indicator, in the style the settings pick (see
//! [`TrayStyle`]). Muted shows the Windows mute glyph in every style.

use std::cell::RefCell;
use std::f32::consts::TAU;
use std::sync::Arc;

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateBitmap, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER, DT_NOCLIP, DT_SINGLELINE, DT_VCENTER,
    DeleteDC, DeleteObject, DrawTextW, FF_DONTCARE, FW_SEMIBOLD, HBITMAP, HDC, OUT_DEFAULT_PRECIS, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory, WICBitmapDitherTypeNone,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnDemand, WICRect,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};
use windows::core::{PCWSTR, w};

use crate::spectrum::{BANDS, Spectrum};
use crate::{AppIcon, PulseStyle, TrayMiddle, TrayStyle};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub level: f32,
    pub muted: bool,
    pub style: TrayStyle,
    pub middle: TrayMiddle,
    pub pulse: PulseStyle,
    pub light_taskbar: bool,
    /// The sound playing, for the live styles.
    pub spectrum: Spectrum,
    /// Whether what the media keys control plays, or `None` without media.
    pub playing: Option<bool>,
    /// The cover of what's playing.
    pub art: Option<AppIcon>,
    /// Milliseconds, which moves the equalizer's bars.
    pub clock: u64,
}

/// Segoe MDL2 Assets glyphs, the font Windows draws its own volume icon with.
const SPEAKER: &str = "\u{E992}";
/// The speaker with one, two and three waves.
const WAVES: [&str; 3] = ["\u{E993}", "\u{E994}", "\u{E995}"];
const MUTE: &str = "\u{E74F}";

/// True when the taskbar uses the light theme, so the icon must be dark.
pub fn light_taskbar() -> bool {
    let mut value = 0u32;
    let mut size = 4u32;
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as _),
            Some(&mut size),
        )
    };
    found.is_ok() && value == 1
}

pub fn render(look: &Look, size: i32) -> Option<HICON> {
    let pixels = bgra(look, size)?;
    unsafe { icon_from_bgra(size, &pixels) }
}

/// The icon as straight-alpha BGRA pixels.
pub fn bgra(look: &Look, size: i32) -> Option<Vec<u8>> {
    let fg = if look.light_taskbar { [28u8, 28, 28] } else { [255u8, 255, 255] };
    if !look.muted
        && look.style.has_middle()
        && look.middle == TrayMiddle::AlbumArt
        && let Some(art) = &look.art
        && let Some(pixels) = album_art(look, art, size, fg)
    {
        return Some(pixels);
    }
    let alpha = alpha(look, size)?;
    let pixels = alpha.iter().flat_map(|&a| [fg[2], fg[1], fg[0], (a.clamp(0.0, 1.0) * 255.0).round() as u8]).collect();
    Some(pixels)
}

/// Each pixel's opacity in the taskbar's text color.
fn alpha(look: &Look, size: i32) -> Option<Vec<f32>> {
    if look.muted {
        return glyph(size, MUTE, 0.9, 0.0);
    }
    let s = size as f32;
    let unit = |shape: &dyn Fn(f32, f32) -> f32| coverage(size, |x, y| shape(x / s, y / s));
    let level = look.level.clamp(0.0, 1.0);
    let sound = look.spectrum.level.clamp(0.0, 1.0);
    if look.style.has_middle() {
        let middle = middle(look, size)?;
        return Some(match frame(look, size) {
            Some(frame) => frame.iter().zip(middle).map(|(a, b)| a.max(b)).collect(),
            None => middle,
        });
    }
    Some(match look.style {
        TrayStyle::Ring | TrayStyle::Dial | TrayStyle::Plain => unreachable!("drawn with a middle"),
        TrayStyle::SpeakerWaves => glyph(size, [SPEAKER, WAVES[0], WAVES[1], WAVES[2]][steps(level)], 0.9, 0.0)?,
        TrayStyle::GradedWaves => waves(size, |i| {
            // A wave reaches its tips halfway through its third of the volume, then
            // keeps brightening through the rest.
            let f = (level * 3.0 - i as f32).clamp(0.0, 1.0);
            ((f / 0.5).min(1.0), 0.55 + 0.45 * f, WAVE_TRACK)
        })?,
        TrayStyle::LiveWaves => {
            // The waves the volume shows light up from the speaker outward as the
            // sound rises, like a meter.
            let shown = steps(level);
            waves(size, |i| {
                let lit = (sound * 3.0 - i as f32).clamp(0.0, 1.0);
                (if i < shown { 1.0 } else { 0.0 }, WAVE_TRACK + (1.0 - WAVE_TRACK) * lit, 0.0)
            })?
        }
        TrayStyle::SignalSteps => unit(&|x, y| signal_steps(x, y, level)),
        TrayStyle::FillingSpeaker => unit(&|x, y| filling_speaker(x, y, level)),
        TrayStyle::Equalizer => {
            let heights = equalizer(level, &look.spectrum);
            unit(&|x, y| heights.iter().enumerate().map(|(i, h)| bar(x, y, i, *h)).fold(0.0, f32::max))
        }
    })
}

/// The ring or dial around the middle, or `None` for a plain icon.
fn frame(look: &Look, size: i32) -> Option<Vec<f32>> {
    let s = size as f32;
    let level = look.level.clamp(0.0, 1.0);
    let (start, sweep) = match look.style {
        TrayStyle::Ring => (0.0, TAU),
        TrayStyle::Dial => (DIAL_START, DIAL_SWEEP),
        _ => return None,
    };
    Some(coverage(size, |x, y| arc_band(x / s, y / s, start, sweep, level)))
}

/// How much bigger the middle draws without a frame around it.
const PLAIN_SCALE: f32 = 1.55;

/// The middle in the taskbar's color. Album art draws in color in [`album_art`], and
/// falls back to the speaker here, as play state does without media.
fn middle(look: &Look, size: i32) -> Option<Vec<f32>> {
    let s = size as f32;
    let plain = look.style == TrayStyle::Plain;
    let k = if plain { PLAIN_SCALE } else { 1.0 };
    // Shapes are drawn for the ring and scaled about the center without one.
    let scaled = |shape: &dyn Fn(f32, f32) -> f32| {
        coverage(size, |x, y| shape((x / s - 0.5) / k + 0.5, (y / s - 0.5) / k + 0.5))
    };
    let speaker = || if plain { glyph(size, SPEAKER, 1.1, 0.27) } else { glyph(size, SPEAKER, 0.78, 0.17) };
    let level = look.level.clamp(0.0, 1.0);
    match look.middle {
        TrayMiddle::Speaker | TrayMiddle::AlbumArt => speaker(),
        TrayMiddle::PlayState => match look.playing {
            Some(true) => Some(scaled(&|x, y| if triangle(x, y) { 1.0 } else { 0.0 })),
            Some(false) => Some(scaled(&|x, y| {
                let bars = (0.37..=0.45).contains(&x) || (0.55..=0.63).contains(&x);
                if bars && (0.34..=0.66).contains(&y) { 1.0 } else { 0.0 }
            })),
            None => speaker(),
        },
        TrayMiddle::Number => {
            let value = (level * 100.0).round() as u32;
            let scale = match (plain, value >= 100) {
                (true, false) => 0.95,
                (true, true) => 0.78,
                (false, false) => 0.5,
                (false, true) => 0.4,
            };
            number(size, value, scale)
        }
        TrayMiddle::Pulse => {
            let t = clock_seconds(look.clock);
            Some(scaled(&|x, y| pulse(look.pulse, x, y, &look.spectrum, t)))
        }
    }
}

/// How many waves Windows' volume icon shows: none at 0, then one per third.
fn steps(level: f32) -> usize {
    match (level * 100.0).round() as u32 {
        0 => 0,
        1..=33 => 1,
        34..=66 => 2,
        _ => 3,
    }
}

/// Opacity of an unlit wave.
const WAVE_TRACK: f32 = 0.25;

/// The speaker and its three waves, cut from Windows' own glyphs so they match the
/// native icon. `wave(i)` gives how far wave `i` spreads from its middle toward its
/// tips (0 to 1), the lit part's opacity, and the rest's.
fn waves(size: i32, wave: impl Fn(usize) -> (f32, f32, f32)) -> Option<Vec<f32>> {
    let n = size as usize;
    let mut layers = vec![glyph(size, SPEAKER, 0.9, 0.0)?];
    for w in WAVES {
        layers.push(glyph(size, w, 0.9, 0.0)?);
    }
    let mut out = layers[0].clone();
    for i in 0..3 {
        let mask: Vec<f32> = layers[i + 1].iter().zip(&layers[i]).map(|(a, b)| (a - b).max(0.0)).collect();
        let (spread, lit, track) = wave(i);
        // The wave's middle row and half its height, from the pixels it covers.
        let rows: Vec<usize> = (0..n).filter(|y| mask[y * n..(y + 1) * n].iter().any(|m| *m > 0.25)).collect();
        let (Some(top), Some(bottom)) = (rows.first(), rows.last()) else { continue };
        let middle = (top + bottom + 1) as f32 / 2.0;
        let half = (bottom + 1 - top) as f32 / 2.0;
        for (p, m) in mask.iter().enumerate().filter(|(_, m)| **m > 0.0) {
            let off = ((p / n) as f32 + 0.5 - middle).abs();
            let on = (spread * half - off + 0.5).clamp(0.0, 1.0);
            out[p] = out[p].max(m * (on * lit + (1.0 - on) * track));
        }
    }
    Some(out)
}

/// Averages a 4x4 grid of samples per pixel for antialiasing.
fn coverage(size: i32, shape: impl Fn(f32, f32) -> f32) -> Vec<f32> {
    const N: usize = 4;
    let mut out = Vec::with_capacity((size * size) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut sum = 0.0;
            for sy in 0..N {
                for sx in 0..N {
                    let x = px as f32 + (sx as f32 + 0.5) / N as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / N as f32;
                    sum += shape(x, y);
                }
            }
            out.push(sum / (N * N) as f32);
        }
    }
    out
}

// The shapes below take coordinates in the unit square and return opacity there.

const RING_OUTER: f32 = 0.48;
const RING_WIDTH: f32 = 0.12;
const TRACK_OPACITY: f32 = 0.32;
/// The dial runs clockwise from lower left to lower right.
const DIAL_START: f32 = TAU * 0.625;
const DIAL_SWEEP: f32 = TAU * 0.75;

/// A band around the center that starts `start` clockwise from 12 o'clock and runs
/// `sweep`, filled for `level` of it.
fn arc_band(x: f32, y: f32, start: f32, sweep: f32, level: f32) -> f32 {
    let (dx, dy) = (x - 0.5, y - 0.5);
    let r = (dx * dx + dy * dy).sqrt();
    let mid = RING_OUTER - RING_WIDTH / 2.0;
    if (r - mid).abs() > RING_WIDTH / 2.0 {
        return 0.0;
    }
    let along = (dx.atan2(-dy) - start).rem_euclid(TAU);
    let fill = level.clamp(0.0, 1.0) * sweep;
    if along <= fill || cap(dx, dy, mid, start, level) || cap(dx, dy, mid, start + fill, level) {
        1.0
    } else if along <= sweep {
        TRACK_OPACITY
    } else {
        0.0
    }
}

/// Four rising bars, each filling from the bottom once the one before is full.
fn signal_steps(x: f32, y: f32, level: f32) -> f32 {
    for i in 0..4 {
        let left = 0.06 + i as f32 * 0.235;
        let height = 0.25 + i as f32 * 0.2;
        if x < left || x > left + 0.17 || y < 0.92 - height || y > 0.92 {
            continue;
        }
        let fill = (level * 4.0 - i as f32).clamp(0.0, 1.0);
        return if y >= 0.92 - height * fill { 1.0 } else { TRACK_OPACITY };
    }
    0.0
}

/// A solid speaker, lit from the left up to the volume.
fn filling_speaker(x: f32, y: f32, level: f32) -> f32 {
    let body = (0.1..=0.34).contains(&x) && (0.36..=0.64).contains(&y);
    let cone = (0.34..=0.84).contains(&x) && (y - 0.5).abs() <= 0.14 + (x - 0.34) / 0.5 * 0.26;
    if !(body || cone) {
        return 0.0;
    }
    if x <= 0.1 + level * 0.74 { 1.0 } else { TRACK_OPACITY }
}

/// Heights of the equalizer's four bars, from low to high frequencies, scaled by
/// the volume.
fn equalizer(level: f32, spectrum: &Spectrum) -> [f32; 4] {
    std::array::from_fn(|i| 0.16 + 0.7 * band_at(spectrum, i as f32 / 3.0) * level.sqrt())
}

/// Equalizer bar `i`, `height` tall with round ends, centered vertically.
fn bar(x: f32, y: f32, i: usize, height: f32) -> f32 {
    let (cx, r) = (0.175 + i as f32 * 0.215, 0.075);
    let reach = (height / 2.0 - r).max(0.0);
    let dy = ((y - 0.5).abs() - reach).max(0.0);
    if (x - cx).powi(2) + dy * dy <= r * r { 1.0 } else { 0.0 }
}

/// Seconds on a clock that wraps every ten minutes, for motion.
fn clock_seconds(clock: u64) -> f32 {
    (clock % 600_000) as f32 / 1000.0
}

/// The spectrum at `f`, from 0 (bass) to 1 (treble), between its bands.
fn band_at(spectrum: &Spectrum, f: f32) -> f32 {
    let at = f.clamp(0.0, 1.0) * (BANDS - 1) as f32;
    let (low, part) = (at.floor() as usize, at.fract());
    let high = (low + 1).min(BANDS - 1);
    spectrum.bands[low] * (1.0 - part) + spectrum.bands[high] * part
}

/// Where an angle falls on a spectrum mirrored around the circle: bass at 12 o'clock,
/// treble at 6, the same on both sides.
fn mirrored(dx: f32, dy: f32) -> f32 {
    dx.atan2(-dy).abs() / std::f32::consts::PI
}

/// The pulse middle at a point, inside the ring's opening (radius about 0.34).
fn pulse(style: PulseStyle, x: f32, y: f32, s: &Spectrum, t: f32) -> f32 {
    let (dx, dy) = (x - 0.5, y - 0.5);
    let r = (dx * dx + dy * dy).sqrt();
    match style {
        PulseStyle::Dot => {
            // Swells with the bass, its edge bulging with the spectrum around it.
            let edge = 0.08 + 0.09 * s.bass + 0.045 * band_at(s, mirrored(dx, dy));
            if r <= edge { 1.0 } else { 0.0 }
        }
        PulseStyle::Rays => {
            // Six spokes, each stretching with its own band.
            const SPOKES: usize = 6;
            let width = 0.045;
            let mut best = disc(x, y, 0.05);
            for i in 0..SPOKES {
                let a = i as f32 / SPOKES as f32 * TAU - TAU / 4.0;
                let (ux, uy) = (a.cos(), a.sin());
                let len = 0.02 + 0.2 * band_at(s, mirrored(ux, uy));
                let along = (dx * ux + dy * uy).clamp(0.12, 0.12 + len);
                let (px, py) = (dx - ux * along, dy - uy * along);
                if px * px + py * py <= width * width {
                    best = 1.0;
                }
            }
            best
        }
        PulseStyle::Ripples => {
            // Rings spreading from a center that swells with the bass, faster and
            // brighter when loud.
            let mut best = disc(x, y, 0.05 + 0.07 * s.bass);
            for j in 0..2 {
                let p = (t * (0.5 + 1.0 * s.level) + j as f32 * 0.5).fract();
                let radius = 0.1 + 0.22 * p;
                if (r - radius).abs() <= 0.035 {
                    best = best.max(s.level.max(0.15) * (1.0 - p));
                }
            }
            best
        }
        PulseStyle::Orbit => {
            // Six dots on a slowly turning circle, each swelling with its own band.
            const DOTS: usize = 6;
            let mut best = 0.0f32;
            for i in 0..DOTS {
                let a = i as f32 / DOTS as f32 * TAU + t * 0.6;
                let (cx, cy) = (0.2 * a.cos(), 0.2 * a.sin());
                let size = 0.04 + 0.06 * band_at(s, i as f32 / (DOTS - 1) as f32);
                if (dx - cx).powi(2) + (dy - cy).powi(2) <= size * size {
                    best = 1.0;
                }
            }
            best
        }
        PulseStyle::Spectrum => {
            // Bars all around a circle, mirrored left and right like a music video's
            // visualizer.
            const BARS: f32 = 16.0;
            let slot = TAU / BARS;
            let a = dx.atan2(-dy).rem_euclid(TAU);
            let center = ((a / slot).round() * slot) % TAU;
            let (ux, uy) = (center.sin(), -center.cos());
            let len = 0.03 + 0.18 * band_at(s, mirrored(ux, uy));
            let along = (dx * ux + dy * uy).clamp(0.11, 0.11 + len);
            let (px, py) = (dx - ux * along, dy - uy * along);
            if px * px + py * py <= 0.028f32.powi(2) { 1.0 } else { 0.0 }
        }
    }
}

fn disc(x: f32, y: f32, r: f32) -> f32 {
    if (x - 0.5).powi(2) + (y - 0.5).powi(2) <= r * r { 1.0 } else { 0.0 }
}

/// A play triangle, nudged right of center so it looks centered.
fn triangle(x: f32, y: f32) -> bool {
    (0.41..=0.67).contains(&x) && (y - 0.5).abs() <= (0.67 - x) / 0.26 * 0.17
}

/// The cover of what's playing in color, inside the ring or dial if there is one. A
/// paused cover dims.
fn album_art(look: &Look, art: &AppIcon, size: i32, fg: [u8; 3]) -> Option<Vec<u8>> {
    let cover = decode_square(art, size)?;
    let s = size as f32;
    let ring = frame(look, size).unwrap_or_else(|| vec![0.0; (size * size) as usize]);
    let radius = if look.style == TrayStyle::Plain { 0.47 } else { 0.33 };
    let inside = coverage(size, |x, y| disc(x / s, y / s, radius));
    let shade = if look.playing == Some(false) { 0.55 } else { 1.0 };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for p in 0..(size * size) as usize {
        if inside[p] >= ring[p] {
            out.extend(cover[p * 4..p * 4 + 3].iter().map(|c| (*c as f32 * shade) as u8));
            out.push((inside[p] * 255.0).round() as u8);
        } else {
            out.extend([fg[2], fg[1], fg[0], (ring[p] * 255.0).round() as u8]);
        }
    }
    Some(out)
}

/// A decoded cover: its source data, its size, and its BGRA pixels.
type Cover = (Arc<[u8]>, i32, Vec<u8>);

thread_local! {
    /// The last decoded cover.
    static COVER: RefCell<Option<Cover>> = const { RefCell::new(None) };
}

/// The image cropped to its middle square and scaled to `size`, as BGRA.
fn decode_square(image: &AppIcon, size: i32) -> Option<Vec<u8>> {
    let cached = COVER.with_borrow(|c| {
        c.as_ref().filter(|(data, s, _)| Arc::ptr_eq(data, &image.data) && *s == size).map(|c| c.2.clone())
    });
    if cached.is_some() {
        return cached;
    }
    let pixels = unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = wic.CreateStream().ok()?;
        stream.InitializeFromMemory(&image.data).ok()?;
        let decoder = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h).ok()?;
        let side = w.min(h);
        let square =
            WICRect { X: ((w - side) / 2) as i32, Y: ((h - side) / 2) as i32, Width: side as i32, Height: side as i32 };
        let clipper = wic.CreateBitmapClipper().ok()?;
        clipper.Initialize(&frame, &square).ok()?;
        let scaler = wic.CreateBitmapScaler().ok()?;
        scaler.Initialize(&clipper, size as u32, size as u32, WICBitmapInterpolationModeFant).ok()?;
        let converter = wic.CreateFormatConverter().ok()?;
        converter
            .Initialize(
                &scaler,
                &GUID_WICPixelFormat32bppBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )
            .ok()?;
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        converter.CopyPixels(std::ptr::null(), size as u32 * 4, &mut pixels).ok()?;
        pixels
    };
    COVER.with_borrow_mut(|c| *c = Some((image.data.clone(), size, pixels.clone())));
    Some(pixels)
}

/// Rounded ends of the filled arc.
fn cap(dx: f32, dy: f32, mid: f32, angle: f32, level: f32) -> bool {
    if level <= 0.0 {
        return false;
    }
    let (cx, cy) = (mid * angle.sin(), -mid * angle.cos());
    (dx - cx).powi(2) + (dy - cy).powi(2) <= (RING_WIDTH / 2.0).powi(2)
}

/// An icon-font glyph centered in the icon. `shift` moves it right by a fraction of the
/// size, because the speaker glyphs sit left in their box to leave room for sound waves.
fn glyph(size: i32, glyph: &str, scale: f32, shift: f32) -> Option<Vec<f32>> {
    let height = (size as f32 * scale).round() as i32;
    let offset = (size as f32 * shift).round() as i32;
    text(size, glyph, w!("Segoe MDL2 Assets"), height, 0, offset)
}

/// The volume as digits. Bahnschrift Condensed keeps "100" legible at 16 px.
fn number(size: i32, value: u32, scale: f32) -> Option<Vec<f32>> {
    let height = (size as f32 * scale).round() as i32;
    text(size, &value.to_string(), w!("Bahnschrift SemiBold Condensed"), height, FW_SEMIBOLD.0 as i32, 0)
}

/// Draws `text` centered with GDI and returns each pixel's coverage.
fn text(size: i32, text: &str, face: PCWSTR, height: i32, weight: i32, offset: i32) -> Option<Vec<f32>> {
    let mut text: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let (dc, bitmap, bits) = dib(size)?;
        let old_bitmap = SelectObject(dc, bitmap.into());
        let font = CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            FF_DONTCARE.0 as u32,
            face,
        );
        let old_font = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let mut rect = RECT { left: offset - size, top: 0, right: offset + 2 * size, bottom: size };
        DrawTextW(dc, &mut text, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP);

        // White text on black: any channel is the coverage.
        let pixels = std::slice::from_raw_parts(bits as *const u8, (size * size * 4) as usize);
        let alpha = pixels.as_chunks::<4>().0.iter().map(|px| px[0].max(px[1]).max(px[2]) as f32 / 255.0).collect();

        SelectObject(dc, old_font);
        SelectObject(dc, old_bitmap);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        Some(alpha)
    }
}

/// A top-down 32-bit DIB section, zero-filled, with its memory DC.
unsafe fn dib(size: i32) -> Option<(HDC, HBITMAP, *mut std::ffi::c_void)> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        let dc = CreateCompatibleDC(None);
        let mut bits = std::ptr::null_mut();
        match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => Some((dc, bitmap, bits)),
            Err(_) => {
                let _ = DeleteDC(dc);
                None
            }
        }
    }
}

unsafe fn icon_from_bgra(size: i32, pixels: &[u8]) -> Option<HICON> {
    unsafe {
        let (dc, color, bits) = dib(size)?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
        let _ = DeleteDC(dc);
        // An all-zero mask lets the color bitmap's alpha decide visibility.
        let mask_bits = vec![0u8; ((size + 15) / 16 * 2 * size) as usize];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr() as _));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(mask.into());
        let _ = DeleteObject(color.into());
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wide cover keeps its middle square: red on the left half, blue on the right.
    #[test]
    fn crops_and_scales_a_cover() {
        crate::util::com_init();
        let (w, h) = (40u32, 20u32);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|p| match p % w {
                x if x < 20 => [255, 0, 0, 255],
                _ => [0, 0, 255, 255],
            })
            .collect();
        let art = AppIcon { data: crate::icon::encode_png(w, h, &rgba).into(), kind: crate::ImageKind::Png, art: true };
        let bgra = decode_square(&art, 16).expect("decode");
        let at = |x: usize, y: usize| &bgra[(y * 16 + x) * 4..(y * 16 + x) * 4 + 3];
        assert_eq!(at(2, 8), [0, 0, 255], "left half is red");
        assert_eq!(at(13, 8), [255, 0, 0], "right half is blue");
    }
}
