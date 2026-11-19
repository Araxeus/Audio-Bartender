//! Two threads with their own message loops:
//! - the hook thread runs the low-level keyboard hook and nothing else, because Windows
//!   silently removes a hook whose thread is too slow to answer,
//! - the tray thread owns the hidden window, the tray icon and its menu.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HMODULE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::System::LibraryLoader::{
    EnumResourceNamesW, GetModuleHandleW, GetProcAddress, LoadLibraryW,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCSTR, PCWSTR, w};

use crate::util::fill_wide;
use crate::{Cmd, Event, MediaKey, Shared};

const CLASS: PCWSTR = w!("MediaKeysSelector.Tray");
const WM_TRAY: u32 = WM_APP + 1;
const WM_SHOW: u32 = WM_APP + 2;
const WM_TOOLTIP: u32 = WM_APP + 3;

const ID_OPEN: usize = 1;
const ID_AUTOMATIC: usize = 2;
const ID_AUTOSTART: usize = 3;
const ID_QUIT: usize = 4;
const ID_TARGET: usize = 100;

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
static TRAY: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static ROUTING: AtomicBool = AtomicBool::new(false);
static TOOLTIP: Mutex<String> = Mutex::new(String::new());

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
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(instance.into()), 0);
        crate::util::log(format!("hook installed: {}", hook.is_ok()));
    }
    pump_messages();
}

fn shared() -> &'static Shared {
    SHARED.get().expect("core started")
}

/// Turns key capture on or off and updates the tray tooltip. Called by the media worker.
pub fn set_routing(target: Option<String>) {
    crate::util::log(format!("routing -> {target:?}"));
    ROUTING.store(target.is_some(), Relaxed);
    *TOOLTIP.lock().unwrap() = match target {
        Some(name) => format!("Media keys → {name}"),
        None => "Media keys → Automatic".to_owned(),
    };
    post(WM_TOOLTIP);
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

fn post(msg: u32) {
    let hwnd = HWND(TRAY.load(Relaxed) as _);
    if !hwnd.is_invalid() {
        unsafe {
            let _ = PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(0));
        }
    }
}

fn run_tray() {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: CLASS, ..Default::default() };
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
        allow_dark_menus(hwnd);
        tray_icon(hwnd, NIM_ADD);
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

use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP,
};

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

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => match lparam.0 as u32 {
            WM_LBUTTONUP => shared().emit(Event::Show),
            WM_RBUTTONUP | WM_CONTEXTMENU => unsafe { show_menu(hwnd) },
            _ => {}
        },
        WM_SHOW => shared().emit(Event::Show),
        WM_TOOLTIP => unsafe { tray_icon(hwnd, NIM_MODIFY) },
        _ if msg == TASKBAR_CREATED.load(Relaxed) => unsafe { tray_icon(hwnd, NIM_ADD) },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

unsafe fn tray_icon(hwnd: HWND, action: windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE) {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: unsafe { app_icon() },
        ..Default::default()
    };
    let tip = TOOLTIP.lock().unwrap().clone();
    fill_wide(&mut data.szTip, if tip.is_empty() { "Media Keys Selector" } else { &tip });
    unsafe {
        let _ = Shell_NotifyIconW(action, &data);
    }
}

/// The exe's first icon resource at small-icon size, or the stock application icon.
unsafe fn app_icon() -> HICON {
    static ICON: AtomicIsize = AtomicIsize::new(0);
    let cached = ICON.load(Relaxed);
    if cached != 0 {
        return HICON(cached as _);
    }

    unsafe extern "system" fn first(module: HMODULE, _: PCWSTR, name: PCWSTR, out: isize) -> BOOL {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
        if let Ok(icon) = unsafe { LoadImageW(Some(module.into()), name, IMAGE_ICON, size, size, LR_DEFAULTCOLOR) } {
            unsafe { *(out as *mut isize) = icon.0 as isize };
        }
        false.into()
    }

    let mut found: isize = 0;
    unsafe {
        let module = GetModuleHandleW(None).unwrap_or_default();
        let _ = EnumResourceNamesW(Some(module), RT_GROUP_ICON, Some(first), &mut found as *mut isize as isize);
        if found == 0 {
            found = LoadIconW(None, IDI_APPLICATION).map(|i| i.0 as isize).unwrap_or(0);
        }
    }
    ICON.store(found, Relaxed);
    HICON(found as _)
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

unsafe fn show_menu(hwnd: HWND) {
    let state = shared().state();
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let add = |flags: MENU_ITEM_FLAGS, id: usize, text: &str| {
            let text = crate::util::wide(text);
            let _ = AppendMenuW(menu, flags, id, PCWSTR(text.as_ptr()));
        };
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };

        add(MF_STRING | MF_GRAYED, 0, "Send media keys to");
        add(MF_STRING | check(state.selected.is_none()), ID_AUTOMATIC, "Automatic (Windows decides)");
        for (i, target) in state.targets.iter().enumerate() {
            let selected = state.selected.as_deref() == Some(target.id.as_str());
            let label = if target.running { target.name.clone() } else { format!("{} (not running)", target.name) };
            add(MF_STRING | check(selected), ID_TARGET + i, &label);
        }
        add(MF_SEPARATOR, 0, "");
        add(MF_STRING | check(state.autostart), ID_AUTOSTART, "Start with Windows");
        add(MF_STRING, ID_OPEN, "Open");
        add(MF_STRING, ID_QUIT, "Quit");
        let _ = SetMenuDefaultItem(menu, ID_OPEN as u32, 0);

        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // The menu only closes on outside clicks while its owner is foreground.
        let _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, at.x, at.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        match picked.0 as usize {
            ID_OPEN => shared().emit(Event::Show),
            ID_AUTOMATIC => shared().send(Cmd::Select(None)),
            ID_AUTOSTART => shared().send(Cmd::SetAutostart(!state.autostart)),
            ID_QUIT => {
                remove_tray();
                shared().emit(Event::Quit);
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
