//! Two threads with their own message loops:
//! - the hook thread runs the low-level keyboard and mouse hooks and nothing else,
//!   because Windows silently removes a hook whose thread is too slow to answer,
//! - the tray thread owns the hidden window, the tray icon and its menu.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromRect,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIN_POPUPCLOSE, NIN_POPUPOPEN, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICON_VERSION_4, NOTIFY_ICON_MESSAGE, NOTIFYICONDATAW,
    NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::core::{BOOL, PCSTR, PCWSTR, PWSTR, w};

use crate::tray_icon::{self, Look};
use crate::tooltip;
use crate::util::fill_wide;
use crate::{Anchor, Cmd, Event, MediaKey, Page, Rect, Setting, Shared, State, TrayStyle, audio, volume};

const CLASS: PCWSTR = w!("MediaKeysSelector.Tray");
const WM_TRAY: u32 = WM_APP + 1;
const WM_SHOW: u32 = WM_APP + 2;
const WM_REDRAW: u32 = WM_APP + 3;
const WM_WHEEL: u32 = WM_APP + 4;
const WM_MENU_WHEEL: u32 = WM_APP + 5;

/// Polls the pointer while our tooltip outlives the shell's hover.
const TIMER_TOOLTIP: usize = 1;

const ID_OPEN: usize = 1;
const ID_AUTOMATIC: usize = 2;
const ID_AUTOSTART: usize = 3;
const ID_QUIT: usize = 4;
const ID_SETTINGS: usize = 5;
const ID_MASTER: usize = 6;
const ID_TARGET: usize = 100;
const ID_MIXER: usize = 500;

/// Master volume change per wheel notch over the tray icon, like the volume keys.
const WHEEL_STEP: f32 = 0.02;

/// Volume change per wheel notch over a menu entry, matching the flyout.
const MENU_STEP: f32 = 0.02;

/// Volume id of the master volume in the menu.
const MASTER: &str = "master";

/// How long after the pointer last hovered the icon its cached position is trusted.
const HOVER_TRUST_MS: u64 = 30_000;

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
static TRAY: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// The open context menu, for the mouse hook. Zero when no menu is open.
static MENU: AtomicIsize = AtomicIsize::new(0);
static MENU_WHEEL: Mutex<Vec<WheelItem>> = Mutex::new(Vec::new());
static ROUTING: AtomicBool = AtomicBool::new(false);

/// What the tray shows. The worker writes it; the tray thread draws it.
static TRAY_STATE: Mutex<TrayState> = Mutex::new(TrayState {
    level: 0.0,
    muted: false,
    style: TrayStyle::Ring,
    device: String::new(),
    keys: String::new(),
});

struct TrayState {
    level: f32,
    muted: bool,
    style: TrayStyle,
    /// " · Speakers", or empty without a device.
    device: String,
    /// "Media keys → Spotify".
    keys: String,
}

impl TrayState {
    /// Built on every redraw, so a wheel step shows its new volume right away.
    fn tip(&self) -> String {
        let volume = if self.muted { "Muted".to_owned() } else { format!("Volume {}%", (self.level * 100.0).round()) };
        format!("{volume}{}\n{}", self.device, self.keys)
    }
}

/// The icon's screen rectangle as last reported by the shell, for the mouse hook.
static ICON_RECT: [AtomicI32; 4] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];
static LAST_HOVER_MS: AtomicU64 = AtomicU64::new(0);

/// Bit per media key that is currently held, to ignore auto-repeat.
static HELD: AtomicU32 = AtomicU32::new(0);

/// Off while both front ends run side by side for comparison.
const SINGLE_INSTANCE: bool = false;

/// Returns false when another instance runs, after asking it to show its window.
pub fn claim_single_instance() -> bool {
    if !SINGLE_INSTANCE {
        return true;
    }
    unsafe {
        let mutex = CreateMutexW(None, true, w!("Local\\MediaKeysSelector.Instance"));
        if mutex.is_ok() && GetLastError() != ERROR_ALREADY_EXISTS {
            return true;
        }
        if let Ok(hwnd) = FindWindowW(CLASS, None) {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let _ = AllowSetForegroundWindow(pid);
            let _ = PostMessageW(Some(hwnd), WM_SHOW, WPARAM(0), LPARAM(0));
        }
        false
    }
}

pub fn spawn(shared: Arc<Shared>) {
    let _ = SHARED.set(shared);
    std::thread::Builder::new().name("mediakeys-hook".into()).spawn(run_hook).expect("spawn hook thread");
    std::thread::Builder::new().name("mediakeys-tray".into()).spawn(run_tray).expect("spawn tray thread");
}

fn shared() -> &'static Shared {
    SHARED.get().expect("core started")
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn pump_messages() {
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn run_hook() {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(instance.into()), 0);
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(instance.into()), 0);
        crate::util::log(format!("hooks installed: keyboard {} mouse {}", keyboard.is_ok(), mouse.is_ok()));
    }
    pump_messages();
}

/// Called by the worker after each state change: updates key capture, icon and tooltip.
pub fn update_tray(state: &State) {
    let routed = state.routed().map(|t| t.name.clone());
    ROUTING.store(routed.is_some(), Relaxed);

    let audio = &state.audio;
    {
        let mut tray = TRAY_STATE.lock().unwrap();
        tray.level = audio.volume;
        tray.muted = audio.muted;
        tray.style = state.settings.tray_style;
        tray.device = audio.device().map(|d| format!(" · {}", d.name)).unwrap_or_default();
        tray.keys = routed.map(|name| format!("Media keys → {name}")).unwrap_or_else(|| "Media keys → Automatic".into());
    }
    post(WM_REDRAW, 0);
}

pub fn remove_tray() {
    let hwnd = HWND(TRAY.load(Relaxed) as _);
    if !hwnd.is_invalid() {
        let data = NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, ..Default::default() };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
}

fn post(msg: u32, wparam: usize) {
    post_with(msg, wparam, 0);
}

fn post_with(msg: u32, wparam: usize, lparam: isize) {
    let hwnd = HWND(TRAY.load(Relaxed) as _);
    if !hwnd.is_invalid() {
        unsafe {
            let _ = PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(lparam));
        }
    }
}

fn run_tray() {
    crate::util::com_init();
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class =
            WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: CLASS, ..Default::default() };
        RegisterClassW(&class);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Relaxed);

        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("Media Keys Selector"),
            WS_OVERLAPPED,
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
        TRAY.store(hwnd.0 as isize, Relaxed);
        tooltip::create(hwnd);
        allow_dark_menus(hwnd);
        set_icon(hwnd, NIM_ADD);
    }
    pump_messages();
}

fn media_key(vk: u32) -> Option<(MediaKey, u32)> {
    match VIRTUAL_KEY(vk as u16) {
        VK_MEDIA_PLAY_PAUSE => Some((MediaKey::PlayPause, 1)),
        VK_MEDIA_NEXT_TRACK => Some((MediaKey::Next, 2)),
        VK_MEDIA_PREV_TRACK => Some((MediaKey::Previous, 4)),
        VK_MEDIA_STOP => Some((MediaKey::Stop, 8)),
        _ => None,
    }
}

/// Swallows media keys while a target is routed and hands them to the media worker.
/// Windows drops hooks that block, so this only flips bits and sends on a channel.
///
/// Windows skips this hook entirely if the process registers raw keyboard input,
/// so front ends must not (Tauri needs `DeviceEventFilter::Always`).
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if let Some((key, bit)) = media_key(event.vkCode).filter(|_| ROUTING.load(Relaxed)) {
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if down {
                if HELD.fetch_or(bit, Relaxed) & bit == 0 {
                    shared().send(Cmd::Press(key));
                }
            } else {
                HELD.fetch_and(!bit, Relaxed);
            }
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Turns the wheel over the tray icon into master volume. The shell never forwards
/// wheel events to tray icons, so this is the only way to get them.
unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_MOUSEWHEEL {
        let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let delta = (event.mouseData >> 16) as i16;

        // Over a volume entry of the open context menu.
        let menu = HMENU(MENU.load(Relaxed) as _);
        if !menu.is_invalid() {
            let position = unsafe { MenuItemFromPoint(None, menu, event.pt) };
            if position >= 0 {
                let command = unsafe { GetMenuItemID(menu, position) };
                if MENU_WHEEL.lock().unwrap().iter().any(|w| w.command == command) {
                    post_with(WM_MENU_WHEEL, delta as u16 as usize, command as isize);
                    return LRESULT(1);
                }
            }
        }

        let [l, t, r, b] = ICON_RECT.each_ref().map(|v| v.load(Relaxed));
        let (x, y) = (event.pt.x, event.pt.y);
        let fresh = now_ms().saturating_sub(LAST_HOVER_MS.load(Relaxed)) < HOVER_TRUST_MS;
        if fresh && x >= l && x < r && y >= t && y < b {
            post(WM_WHEEL, delta as u16 as usize);
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Version 4 callbacks carry the event in the low word.
        WM_TRAY => match lparam.0 as u32 & 0xFFFF {
            WM_MOUSEMOVE => remember_icon_rect(hwnd, false),
            NIN_POPUPOPEN => {
                let tip = TRAY_STATE.lock().unwrap().tip();
                tooltip::show(&tip, icon_rect(hwnd).unwrap_or_else(|| cursor_rect()));
            }
            // Redrawing the icon also makes the shell close the tooltip, so while the
            // pointer stays on the icon, keep it and watch for the pointer to leave.
            NIN_POPUPCLOSE => {
                if tooltip::pointer_on_icon() {
                    unsafe {
                        SetTimer(Some(hwnd), TIMER_TOOLTIP, 150, None);
                    }
                } else {
                    tooltip::hide();
                }
            }
            WM_LBUTTONUP => {
                tooltip::hide();
                if let Some(anchor) = anchor(hwnd) {
                    shared().emit(Event::Flyout { anchor, page: Page::Apps, toggle: true });
                }
            }
            WM_MBUTTONUP => {
                audio::toggle_master_mute();
            }
            WM_CONTEXTMENU => {
                tooltip::hide();
                unsafe { show_menu(hwnd) }
            }
            _ => {}
        },
        WM_WHEEL => {
            let notches = wparam.0 as u16 as i16 as f32 / WHEEL_DELTA as f32;
            let result = audio::adjust_master(notches * WHEEL_STEP);
            crate::util::log(format!("tray wheel {notches:+} -> {result:?}"));
            if let Some((level, muted)) = result {
                // Redraw now rather than waiting for the worker's round trip.
                {
                    let mut tray = TRAY_STATE.lock().unwrap();
                    tray.level = level;
                    tray.muted = muted;
                }
                unsafe { set_icon(hwnd, NIM_MODIFY) };
            }
        }
        WM_MENU_WHEEL => unsafe { menu_wheel(lparam.0 as u32, wparam.0 as u16 as i16) },
        WM_TIMER if wparam.0 == TIMER_TOOLTIP => {
            if !tooltip::visible() || !tooltip::pointer_on_icon() {
                tooltip::hide();
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_TOOLTIP);
                }
            }
        }
        WM_SHOW => show(Page::Apps),
        WM_REDRAW => unsafe { set_icon(hwnd, NIM_MODIFY) },
        WM_SETTINGCHANGE => unsafe { set_icon(hwnd, NIM_MODIFY) },
        _ if msg == TASKBAR_CREATED.load(Relaxed) => unsafe { set_icon(hwnd, NIM_ADD) },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

/// The icon's rectangle, if the pointer is on it. For an icon hidden in the overflow
/// area the shell reports the overflow button instead, which must not count.
fn icon_rect(hwnd: HWND) -> Option<RECT> {
    let id = NOTIFYICONIDENTIFIER { cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: hwnd, ..Default::default() };
    let rect = unsafe { Shell_NotifyIconGetRect(&id).ok()? };
    let mut at = POINT::default();
    unsafe { GetCursorPos(&mut at).ok()? };
    let inside = at.x >= rect.left && at.x < rect.right && at.y >= rect.top && at.y < rect.bottom;
    inside.then_some(rect)
}

/// Caches the icon position for the mouse hook. Hover callbacks are frequent, so
/// the shell is asked at most twice a second unless `force` is set.
fn remember_icon_rect(hwnd: HWND, force: bool) {
    let now = now_ms();
    let last = LAST_HOVER_MS.swap(now, Relaxed);
    if !force && now.saturating_sub(last) < 500 {
        return;
    }
    // Keep the last good position when the shell's answer doesn't match the pointer.
    if let Some(r) = icon_rect(hwnd) {
        for (slot, value) in ICON_RECT.iter().zip([r.left, r.top, r.right, r.bottom]) {
            slot.store(value, Relaxed);
        }
    }
}

/// Opens the flyout without a tray click.
fn show(page: Page) {
    shared().emit(Event::Flyout { anchor: default_anchor(), page, toggle: false });
}

/// Next to the tray icon (or the overflow button, if it sits there), else the
/// bottom-right corner of the primary screen.
pub fn default_anchor() -> Anchor {
    let hwnd = HWND(TRAY.load(Relaxed) as _);
    let id = NOTIFYICONIDENTIFIER { cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: hwnd, ..Default::default() };
    let icon = unsafe { Shell_NotifyIconGetRect(&id) }.unwrap_or_else(|_| {
        let work = unsafe {
            let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(monitor, &mut info);
            info.rcWork
        };
        RECT { left: work.right - 1, top: work.bottom - 1, right: work.right, bottom: work.bottom }
    });
    anchor_for(icon)
}

/// A 1-pixel rectangle at the pointer.
fn cursor_rect() -> RECT {
    let mut at = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut at);
    }
    RECT { left: at.x, top: at.y, right: at.x + 1, bottom: at.y + 1 }
}

fn anchor(hwnd: HWND) -> Option<Anchor> {
    remember_icon_rect(hwnd, true);
    // In the overflow area, anchor on the pointer instead.
    Some(anchor_for(icon_rect(hwnd).unwrap_or_else(cursor_rect)))
}

/// The anchor for an icon rectangle: its monitor's work area and scale.
fn anchor_for(icon: RECT) -> Anchor {
    unsafe {
        let monitor = MonitorFromRect(&icon, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let (mut dpi, mut unused) = (96u32, 0u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
        let rect = |r: RECT| Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
        Anchor { icon: rect(icon), work: rect(info.rcWork), scale: dpi as f32 / 96.0 }
    }
}

/// Draws the current volume and pushes it to the shell with `action`.
unsafe fn set_icon(hwnd: HWND, action: NOTIFY_ICON_MESSAGE) {
    static CURRENT: AtomicIsize = AtomicIsize::new(0);
    let (look, tip) = {
        let tray = TRAY_STATE.lock().unwrap();
        let look =
            Look { level: tray.level, muted: tray.muted, style: tray.style, light_taskbar: tray_icon::light_taskbar() };
        (look, tray.tip())
    };
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
    let Some(icon) = tray_icon::render(&look, size) else { return };

    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        // Without NIF_SHOWTIP the shell leaves the tooltip to us (see `tooltip`). The
        // text still goes to the shell for accessibility.
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    fill_wide(&mut data.szTip, if tip.is_empty() { "Media Keys Selector" } else { &tip });
    tooltip::update(&tip);
    unsafe {
        let _ = Shell_NotifyIconW(action, &data);
        if action == NIM_ADD {
            // Version 4 sends the hover events that drive our own tooltip.
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
        // The shell keeps its own copy, so the previous icon can go.
        let old = CURRENT.swap(icon.0 as isize, Relaxed);
        if old != 0 {
            let _ = DestroyIcon(HICON(old as _));
        }
    }
    if action == NIM_ADD {
        remember_icon_rect(hwnd, true);
    }
}

/// Opts into dark context menus when Windows uses dark mode. These uxtheme exports
/// are undocumented but stable since Windows 10 1903.
unsafe fn allow_dark_menus(hwnd: HWND) {
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        let ordinal = |n: usize| GetProcAddress(uxtheme, PCSTR(n as *const u8));
        if let Some(f) = ordinal(135) {
            let set_preferred_app_mode: extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            set_preferred_app_mode(1);
        }
        if let Some(f) = ordinal(133) {
            let allow_for_window: extern "system" fn(HWND, BOOL) -> BOOL = std::mem::transmute(f);
            let _ = allow_for_window(hwnd, true.into());
        }
        if let Some(f) = ordinal(136) {
            let flush_menu_themes: extern "system" fn() = std::mem::transmute(f);
            flush_menu_themes();
        }
    }
}

/// A menu entry whose volume the wheel changes: its command id and the volume id.
struct WheelItem {
    command: u32,
    id: String,
    name: String,
}

/// Formats a menu label with the volume right-aligned in the shortcut column.
fn volume_label(name: &str, level: f32, muted: bool) -> String {
    if muted { format!("{name}\tMuted") } else { format!("{name}\t{}%", (level * 100.0).round()) }
}

unsafe fn show_menu(hwnd: HWND) {
    let state = shared().state();
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let add = |flags: MENU_ITEM_FLAGS, id: usize, text: &str| {
            let text = crate::util::wide(text);
            let _ = AppendMenuW(menu, flags, id, PCWSTR(text.as_ptr()));
        };
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let mut wheel = Vec::new();

        add(MF_STRING, ID_OPEN, "Open Media Keys Selector");
        add(MF_STRING, ID_SETTINGS, "Settings…");
        add(MF_SEPARATOR, 0, "");
        let audio = &state.audio;
        add(MF_STRING, ID_MASTER, &volume_label("Volume", audio.volume, audio.muted));
        wheel.push(WheelItem { command: ID_MASTER as u32, id: MASTER.into(), name: "Volume".into() });
        add(MF_SEPARATOR, 0, "");
        add(MF_STRING | MF_GRAYED, 0, "Send media keys to");
        add(MF_STRING | check(state.selected.is_none()), ID_AUTOMATIC, "Automatic (Windows decides)");
        for (i, target) in state.targets.iter().enumerate() {
            let selected = state.selected.as_deref() == Some(target.id.as_str());
            let label = match target.volume {
                _ if !target.running => format!("{} (not running)", target.name),
                Some(level) => {
                    wheel.push(WheelItem { command: (ID_TARGET + i) as u32, id: target.id.clone(), name: target.name.clone() });
                    volume_label(&target.name, level, target.muted)
                }
                None => target.name.clone(),
            };
            add(MF_STRING | check(selected), ID_TARGET + i, &label);
        }
        if !state.mixer.is_empty() {
            add(MF_SEPARATOR, 0, "");
            add(MF_STRING | MF_GRAYED, 0, "Other apps (click mutes)");
            for (i, app) in state.mixer.iter().enumerate() {
                wheel.push(WheelItem { command: (ID_MIXER + i) as u32, id: app.id.clone(), name: app.name.clone() });
                add(MF_STRING, ID_MIXER + i, &volume_label(&app.name, app.volume, app.muted));
            }
        }
        add(MF_SEPARATOR, 0, "");
        add(MF_STRING | MF_GRAYED, 0, "Scroll over a volume to change it");
        add(MF_STRING | check(state.settings.autostart), ID_AUTOSTART, "Start with Windows");
        add(MF_STRING, ID_QUIT, "Quit");
        let _ = SetMenuDefaultItem(menu, ID_OPEN as u32, 0);

        *MENU_WHEEL.lock().unwrap() = wheel;
        MENU.store(menu.0 as isize, Relaxed);

        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // The menu only closes on outside clicks while its owner is foreground.
        let _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, at.x, at.y, None, hwnd, None);
        MENU.store(0, Relaxed);
        MENU_WHEEL.lock().unwrap().clear();
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        match picked.0 as usize {
            ID_OPEN => show(Page::Apps),
            ID_SETTINGS => show(Page::Settings),
            ID_MASTER => audio::toggle_master_mute(),
            ID_AUTOMATIC => shared().send(Cmd::Select(None)),
            ID_AUTOSTART => shared().send(Cmd::Set(Setting::Autostart(!state.settings.autostart))),
            ID_QUIT => {
                remove_tray();
                shared().emit(Event::Quit);
            }
            id if id >= ID_MIXER => {
                if let Some(app) = state.mixer.get(id - ID_MIXER) {
                    volume::toggle_mute(&app.id);
                    shared().send(Cmd::RefreshAudio { devices: false });
                }
            }
            id if id >= ID_TARGET => {
                if let Some(target) = state.targets.get(id - ID_TARGET) {
                    shared().send(Cmd::Select(Some(target.id.clone())));
                }
            }
            _ => {}
        }
    }
}

/// Applies a wheel step to a menu entry and rewrites its label while the menu stays open.
/// The menu's modal loop still dispatches posted messages, so this runs mid-menu.
unsafe fn menu_wheel(command: u32, delta: i16) {
    let item = MENU_WHEEL.lock().unwrap().iter().find(|w| w.command == command).map(|w| (w.id.clone(), w.name.clone()));
    let Some((id, name)) = item else { return };
    crate::util::log(format!("menu wheel {command} {delta}"));
    let step = delta as f32 / WHEEL_DELTA as f32 * MENU_STEP;
    let level = if id == MASTER {
        audio::adjust_master(step)
    } else {
        volume::adjust(&id, |level| level + step);
        let sessions = volume::sessions(&crate::util::processes());
        volume::level(&sessions, &volume::exes_for(&id))
    };
    shared().send(Cmd::RefreshAudio { devices: false });
    let Some((level, muted)) = level else { return };

    let menu = HMENU(MENU.load(Relaxed) as _);
    if menu.is_invalid() {
        return;
    }
    let mut text = crate::util::wide(&volume_label(&name, level, muted));
    let info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STRING,
        dwTypeData: PWSTR(text.as_mut_ptr()),
        ..Default::default()
    };
    unsafe {
        let _ = SetMenuItemInfoW(menu, command, false, &info);
        // Popup menus live in windows of the system class "#32768".
        if let Ok(popup) = FindWindowW(w!("#32768"), None) {
            let _ = InvalidateRect(Some(popup), None, true);
        }
    }
}
