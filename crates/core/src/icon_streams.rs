//! Moves tray icons by editing Explorer's `IconStreams`, the per-user store behind
//! the Notification Area Icons page. Windows 10 offers apps no API for this:
//! `ITrayNotify::SetPreference` refuses every item.
//!
//! Layout: a 20-byte header, then 1640-byte records. A record holds the exe path as
//! ROT13 UTF-16 in bytes 0..528, the preference at byte 528 (0 hide, 2 show), and
//! the icon GUID at byte 1068. Explorer keeps the value in memory and writes it on
//! exit, so editing means stopping Explorer, writing, and starting it again.

use std::path::Path;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_BINARY, RRF_RT_REG_BINARY, RegCloseKey, RegGetValueW,
    RegOpenKeyExW, RegSetValueExW,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_USER};
use windows::core::{GUID, w};

const KEY: windows::core::PCWSTR =
    w!("Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\TrayNotify");
const VALUE: windows::core::PCWSTR = w!("IconStreams");

const HEADER: usize = 20;
const RECORD: usize = 1640;
const PATH_BYTES: usize = 528;
const PREFERENCE_AT: usize = 528;
const GUID_AT: usize = 1068;

const HIDE: u8 = 0;
const SHOW: u8 = 2;

/// Windows' own volume icon.
const VOLUME_GUID: GUID = GUID::from_u128(0x7820ae73_23e3_4229_82c1_e41cb67d5b9c);

fn read() -> Option<Vec<u8>> {
    let mut size = 0u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, KEY, VALUE, RRF_RT_REG_BINARY, None, None, Some(&mut size)).ok().ok()?;
        let mut data = vec![0u8; size as usize];
        RegGetValueW(HKEY_CURRENT_USER, KEY, VALUE, RRF_RT_REG_BINARY, None, Some(data.as_mut_ptr().cast()), Some(&mut size))
            .ok()
            .ok()?;
        data.truncate(size as usize);
        Some(data)
    }
}

fn write(data: &[u8]) -> bool {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, KEY, None, KEY_SET_VALUE | KEY_QUERY_VALUE, &mut key).is_err() {
            return false;
        }
        let ok = RegSetValueExW(key, VALUE, None, REG_BINARY, Some(data)).is_ok();
        let _ = RegCloseKey(key);
        ok
    }
}

fn rot13(c: u16) -> u16 {
    match c {
        0x41..=0x5A => (c - 0x41 + 13) % 26 + 0x41,
        0x61..=0x7A => (c - 0x61 + 13) % 26 + 0x61,
        _ => c,
    }
}

struct Record<'a> {
    bytes: &'a [u8],
}

impl Record<'_> {
    fn path(&self) -> String {
        let units: Vec<u16> = self.bytes[..PATH_BYTES]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .take_while(|&c| c != 0)
            .map(rot13)
            .collect();
        String::from_utf16_lossy(&units)
    }

    fn guid(&self) -> GUID {
        let b = &self.bytes[GUID_AT..GUID_AT + 16];
        GUID::from_values(
            u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            u16::from_le_bytes([b[4], b[5]]),
            u16::from_le_bytes([b[6], b[7]]),
            [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
        )
    }
}

fn records(data: &[u8]) -> impl Iterator<Item = (usize, Record<'_>)> {
    let count = data.len().saturating_sub(HEADER) / RECORD;
    (0..count).map(move |i| {
        let at = HEADER + i * RECORD;
        (at, Record { bytes: &data[at..at + RECORD] })
    })
}

fn is_ours(record: &Record, exe: &Path) -> bool {
    record.path().eq_ignore_ascii_case(&exe.to_string_lossy())
}

/// True when the stored preferences show our icon and hide Windows' volume icon.
pub fn replaced() -> bool {
    let (Some(data), Ok(exe)) = (read(), std::env::current_exe()) else { return false };
    let mut ours = false;
    let mut volume_shown = false;
    for (at, record) in records(&data) {
        if record.guid() == VOLUME_GUID {
            volume_shown |= data[at + PREFERENCE_AT] == SHOW;
        } else if is_ours(&record, &exe) {
            ours |= data[at + PREFERENCE_AT] == SHOW;
        }
    }
    ours && !volume_shown
}

/// Applies the preferences to a copy of the stored bytes. Returns the new bytes and
/// a line per changed record.
pub fn plan(data: &[u8], exe: &Path, replace: bool) -> (Vec<u8>, Vec<String>) {
    let mut out = data.to_vec();
    let mut changes = Vec::new();
    for (at, record) in records(data) {
        let wanted = if record.guid() == VOLUME_GUID {
            if replace { HIDE } else { SHOW }
        } else if replace && is_ours(&record, exe) {
            SHOW
        } else {
            continue;
        };
        let current = data[at + PREFERENCE_AT];
        if current != wanted {
            out[at + PREFERENCE_AT] = wanted;
            changes.push(format!("{} : {current} -> {wanted}", record.path()));
        }
    }
    (out, changes)
}

/// Lists the changes `replace` would make, without touching anything.
pub fn dry_run(replace: bool) -> Vec<String> {
    let (Some(data), Ok(exe)) = (read(), std::env::current_exe()) else { return vec!["IconStreams unreadable".into()] };
    let (_, changes) = plan(&data, &exe, replace);
    let ours = records(&data).filter(|(_, r)| is_ours(r, &exe)).count();
    let mut lines = vec![format!("records for this exe: {ours}")];
    lines.extend(changes);
    lines
}

fn explorer_running() -> bool {
    crate::util::processes().iter().any(|p| p.exe == "explorer.exe")
}

fn wait(until: impl Fn() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if until() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    until()
}

/// Shows our icon on the taskbar and hides Windows' volume icon, or brings it back.
/// Restarts Explorer, which closes open File Explorer windows.
pub fn apply(replace: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    // A clean exit makes Explorer save the icons it holds in memory, including ours.
    unsafe {
        let taskbar = FindWindowW(w!("Shell_TrayWnd"), None).map_err(|_| "taskbar not found")?;
        PostMessageW(Some(taskbar), WM_USER + 436, WPARAM(0), LPARAM(0)).map_err(|e| e.to_string())?;
    }
    let stopped = wait(|| !explorer_running(), Duration::from_secs(8));

    let result = (|| {
        if !stopped {
            return Err("Explorer did not exit".to_owned());
        }
        let data = read().ok_or("IconStreams unreadable")?;
        let (updated, changes) = plan(&data, &exe, replace);
        crate::util::log(format!("icon streams: {changes:?}"));
        if replace && !records(&data).any(|(_, r)| is_ours(&r, &exe)) {
            return Err("Explorer has no record of this icon yet".to_owned());
        }
        if !write(&updated) {
            return Err("could not write IconStreams".to_owned());
        }
        Ok(())
    })();

    // Always bring Explorer back, whatever happened above.
    if !explorer_running() {
        let _ = std::process::Command::new("explorer.exe").spawn();
    }
    result
}
