//! The tray icon's own tooltip. The shell's tooltip keeps its text while it shows,
//! so a wheel step over the icon would leave a stale volume on screen. With
//! `NOTIFYICON_VERSION_4` and no `NIF_SHOWTIP`, the shell instead says when to show
//! and hide a tooltip, and this one updates live. It is either a plain tooltip
//! control or a card drawn by [`crate::card`].

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering::Relaxed};

use windows::Win32::Foundation::POINT;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Controls::{
    TOOLTIPS_CLASSW, TTF_ABSOLUTE, TTF_TRACK, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH, TTM_TRACKACTIVATE, TTM_TRACKPOSITION,
    TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetCursorPos, GetWindowRect, SendMessageW, WINDOW_STYLE, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::PWSTR;

use crate::card;
use crate::util::wide;

/// What the tooltip shows.
pub enum Tip {
    Text(String),
    Card(card::Content),
}

static TOOLTIP: AtomicIsize = AtomicIsize::new(0);
static OWNER: AtomicIsize = AtomicIsize::new(0);
static VISIBLE: AtomicBool = AtomicBool::new(false);
/// The icon the tooltip belongs to: left, top, right, bottom.
static ICON: [AtomicI32; 4] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];

/// Gap between the icon and the tooltip.
const GAP: i32 = 8;

fn tool(owner: HWND, text: &mut [u16]) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: owner,
        uId: 1,
        lpszText: PWSTR(text.as_mut_ptr()),
        ..Default::default()
    }
}

/// Creates the tooltip control. Call once on the tray thread.
pub fn create(owner: HWND) {
    unsafe {
        let Ok(tooltip) = CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            None,
            WS_POPUP | WINDOW_STYLE(TTS_NOPREFIX | TTS_ALWAYSTIP),
            0,
            0,
            0,
            0,
            Some(owner),
            None,
            None,
            None,
        ) else {
            return;
        };
        let mut text = wide("");
        let info = tool(owner, &mut text);
        SendMessageW(tooltip, TTM_ADDTOOLW, None, Some(LPARAM(&info as *const _ as isize)));
        // A max width lets "\n" break lines.
        SendMessageW(tooltip, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(600)));
        TOOLTIP.store(tooltip.0 as isize, Relaxed);
        OWNER.store(owner.0 as isize, Relaxed);
    }
}

fn handles() -> Option<(HWND, HWND)> {
    let tooltip = HWND(TOOLTIP.load(Relaxed) as _);
    let owner = HWND(OWNER.load(Relaxed) as _);
    (!tooltip.is_invalid()).then_some((tooltip, owner))
}

/// Shows `tip` by `icon`, or updates it if it already shows.
pub fn show(tip: &Tip, icon: RECT) {
    for (slot, value) in ICON.iter().zip([icon.left, icon.top, icon.right, icon.bottom]) {
        slot.store(value, Relaxed);
    }
    VISIBLE.store(true, Relaxed);
    match tip {
        Tip::Text(text) => {
            card::hide();
            show_text(text);
        }
        Tip::Card(content) => {
            hide_text();
            card::show(content, crate::input::anchor_for(icon));
        }
    }
}

fn icon() -> RECT {
    let [left, top, right, bottom] = ICON.each_ref().map(|v| v.load(Relaxed));
    RECT { left, top, right, bottom }
}

fn show_text(text: &str) {
    let Some((tooltip, owner)) = handles() else { return };
    unsafe {
        set_text(tooltip, owner, text);
        let mut empty = wide("");
        let info = tool(owner, &mut empty);
        SendMessageW(tooltip, TTM_TRACKACTIVATE, Some(WPARAM(1)), Some(LPARAM(&info as *const _ as isize)));
        place(tooltip);
    }
    crate::util::raise_topmost(tooltip);
}

/// Centers the tooltip above the icon. Runs after showing, when its size is known.
unsafe fn place(tooltip: HWND) {
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(tooltip, &mut rect) }.is_err() {
        return;
    }
    let [left, top, right, _] = ICON.each_ref().map(|v| v.load(Relaxed));
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    let x = (left + right) / 2 - width / 2;
    let y = top - height - GAP;
    let point = ((y as u16 as u32) << 16 | x as u16 as u32) as isize;
    unsafe {
        SendMessageW(tooltip, TTM_TRACKPOSITION, None, Some(LPARAM(point)));
    }
}

/// True while the pointer is on the icon the tooltip belongs to.
pub fn pointer_on_icon() -> bool {
    let mut at = POINT::default();
    if unsafe { GetCursorPos(&mut at) }.is_err() {
        return false;
    }
    let [l, t, r, b] = ICON.each_ref().map(|v| v.load(Relaxed));
    at.x >= l && at.x < r && at.y >= t && at.y < b
}

pub fn visible() -> bool {
    VISIBLE.load(Relaxed)
}

/// Redraws the tooltip if it is showing.
pub fn update(tip: &Tip) {
    if VISIBLE.load(Relaxed) {
        show(tip, icon());
    }
}

pub fn hide() {
    VISIBLE.store(false, Relaxed);
    hide_text();
    card::hide();
}

fn hide_text() {
    let Some((tooltip, owner)) = handles() else { return };
    let mut empty = wide("");
    let info = tool(owner, &mut empty);
    unsafe {
        SendMessageW(tooltip, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(LPARAM(&info as *const _ as isize)));
    }
}

unsafe fn set_text(tooltip: HWND, owner: HWND, text: &str) {
    let mut text = wide(text);
    let info = tool(owner, &mut text);
    unsafe {
        SendMessageW(tooltip, TTM_UPDATETIPTEXTW, None, Some(LPARAM(&info as *const _ as isize)));
    }
}
