//! App icons pulled from exe files and encoded as PNG for the front ends.

use std::sync::Arc;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    SelectObject,
};
use windows::Win32::UI::Shell::SHDefExtractIconW;
use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON};
use windows::core::PCWSTR;

use crate::util::wide;
use crate::{AppIcon, ImageKind};

/// Big enough for a 34 px avatar at 150% scaling.
const SIZE: u32 = 48;

/// The exe's main icon at [`SIZE`], or `None` when it has none.
pub fn extract(exe_path: &str) -> Option<AppIcon> {
    let path = wide(exe_path);
    let mut icon = HICON::default();
    unsafe {
        // S_FALSE (no icon) counts as success, so check the handle too.
        SHDefExtractIconW(PCWSTR(path.as_ptr()), 0, 0, Some(&mut icon), None, SIZE).ok().ok()?;
        if icon.is_invalid() {
            return None;
        }
        let rgba = render(icon);
        let _ = DestroyIcon(icon);
        Some(AppIcon { data: Arc::from(encode_png(SIZE, SIZE, &rgba?)), kind: ImageKind::Png, art: false })
    }
}

/// Draws the icon into a 32-bit DIB and returns its pixels as RGBA.
unsafe fn render(icon: HICON) -> Option<Vec<u8>> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE as i32,
            // Negative height makes the DIB top-down.
            biHeight: -(SIZE as i32),
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
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        let old = SelectObject(dc, bitmap.into());
        let drawn = DrawIconEx(dc, 0, 0, icon, SIZE as i32, SIZE as i32, 0, None, DI_NORMAL).is_ok();

        let len = (SIZE * SIZE * 4) as usize;
        let mut rgba = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        if !drawn {
            return None;
        }

        for px in rgba.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        // Old icons without an alpha channel come out fully transparent.
        if rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 0) {
            rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|px| px[3] = 255);
        }
        Some(rgba)
    }
}

/// A minimal PNG encoder with uncompressed deflate blocks. The icons are small,
/// so skipping compression costs a few KB and saves a dependency.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let row = width as usize * 4;
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for line in rgba.chunks_exact(row) {
        raw.push(0);
        raw.extend_from_slice(line);
    }

    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        zlib.push((i + 1 == blocks.len()) as u8);
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

#[cfg_attr(not(feature = "serde"), allow(dead_code))]
pub fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for group in data.chunks(3) {
        let n = group.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= group.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
