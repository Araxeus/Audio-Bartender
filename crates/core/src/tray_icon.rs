//! Draws the tray icon as a volume indicator.
//!
//! Ring style: a speaker inside a ring that fills clockwise from the top with the volume.
//! Number style: the volume as digits. Muted shows the Windows mute glyph in both.

use std::f32::consts::TAU;

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateBitmap, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER, DT_NOCLIP, DT_SINGLELINE, DT_VCENTER,
    DeleteDC, DeleteObject, DrawTextW, FF_DONTCARE, FW_SEMIBOLD, HBITMAP, HDC, OUT_DEFAULT_PRECIS, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};
use windows::core::{PCWSTR, w};

use crate::TrayStyle;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub level: f32,
    pub muted: bool,
    pub style: TrayStyle,
    pub light_taskbar: bool,
}

/// Segoe MDL2 Assets glyphs, the font Windows draws its own volume icon with.
const SPEAKER: &str = "\u{E992}";
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
    let s = size as f32;
    let alpha: Vec<f32> = if look.muted {
        glyph(size, MUTE, 0.9, 0.0)?
    } else {
        match look.style {
            TrayStyle::Ring => {
                let ring = coverage(size, |x, y| ring(x / s, y / s, look.level));
                let speaker = glyph(size, SPEAKER, 0.78, 0.17)?;
                ring.iter().zip(speaker).map(|(a, b)| a.max(b)).collect()
            }
            TrayStyle::Number => number(size, (look.level * 100.0).round() as u32)?,
        }
    };
    let (r, g, b) = if look.light_taskbar { (28u8, 28u8, 28u8) } else { (255u8, 255u8, 255u8) };
    let pixels = alpha.iter().flat_map(|&a| [b, g, r, (a.clamp(0.0, 1.0) * 255.0).round() as u8]).collect();
    Some(pixels)
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

// The ring takes coordinates in the unit square and returns opacity at that point.

const RING_OUTER: f32 = 0.48;
const RING_WIDTH: f32 = 0.12;
const TRACK_OPACITY: f32 = 0.32;

fn ring(x: f32, y: f32, level: f32) -> f32 {
    let (dx, dy) = (x - 0.5, y - 0.5);
    let r = (dx * dx + dy * dy).sqrt();
    let mid = RING_OUTER - RING_WIDTH / 2.0;
    if (r - mid).abs() > RING_WIDTH / 2.0 {
        return 0.0;
    }
    // Clockwise angle from 12 o'clock.
    let angle = dx.atan2(-dy).rem_euclid(TAU);
    let sweep = level.clamp(0.0, 1.0) * TAU;
    if angle <= sweep || cap(dx, dy, mid, 0.0, level) || cap(dx, dy, mid, sweep, level) {
        1.0
    } else {
        TRACK_OPACITY
    }
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
fn number(size: i32, value: u32) -> Option<Vec<f32>> {
    let scale = if value >= 100 { 0.78 } else { 0.95 };
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
        let alpha = pixels.chunks_exact(4).map(|px| px[0].max(px[1]).max(px[2]) as f32 / 255.0).collect();

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
