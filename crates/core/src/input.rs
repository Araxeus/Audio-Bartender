//! Two threads with their own message loops:
//! - the hook thread runs the low-level keyboard and mouse hooks and nothing else,
//!   because Windows silently removes a hook whose thread is too slow to answer,
//! - the tray thread owns the hidden window, the tray icon and its menu.

mod speaker_slot;

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::InvalidateRect;
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
    NIF_GUID, NIF_ICON, NIF_MESSAGE, NIF_STATE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIN_POPUPCLOSE, NIN_POPUPOPEN, NIS_HIDDEN, NOTIFY_ICON_MESSAGE, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, GUID, PCSTR, PCWSTR, PWSTR, w};

use crate::tooltip;
use crate::tooltip::Tip;
use crate::tray_icon::{self, Look};
use crate::util::{fill_wide, now_ms};
use crate::{
    Accent, Anchor, Cmd, Event, MediaButtons, MediaKey, Page, Playback, Rect, Shared, State, TooltipStyle, TrayMiddle,
    TrayStyle, audio, card, volume,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const CLASS: PCWSTR = w!("AudioBartender.Tray");
const WM_TRAY: u32 = WM_APP + 1;
const WM_SHOW: u32 = WM_APP + 2;
const WM_REDRAW: u32 = WM_APP + 3;
const WM_WHEEL: u32 = WM_APP + 4;
const WM_MENU_WHEEL: u32 = WM_APP + 5;
const WM_PREVIEW: u32 = WM_APP + 6;
const WM_SPEAKER_SLOT: u32 = WM_APP + 7;
const WM_DISMISS: u32 = WM_APP + 8;
const WM_MENU_MUTE: u32 = WM_APP + 9;
const WM_MENU_SELECT: u32 = WM_APP + 10;

/// Polls the pointer while our tooltip outlives the shell's hover.
const TIMER_TOOLTIP: usize = 1;
const TIMER_RESTORE_ICON: usize = 2;
/// Redraws a live tray icon with the sound playing.
const TIMER_LIVE: usize = 3;
const LIVE_FRAME_MS: u32 = 66;

/// The sound the live tray icon last drew, as `f32` bits.
static SOUND: AtomicU32 = AtomicU32::new(0);
/// The loudest recent sound, as `f32` bits. It falls by `PEAK_FALL` each frame.
static PEAK: AtomicU32 = AtomicU32::new(0);
const PEAK_FALL: f32 = 0.97;
/// Quiet audio below this still reads as quiet.
const MIN_PEAK: f32 = 0.12;
/// Whether `TIMER_LIVE` runs.
static LIVE: AtomicBool = AtomicBool::new(false);

/// How long a card with buttons stays while the pointer is on neither it nor the
/// icon, so the pointer can cross the taskbar to reach it.
const CARD_GRACE_MS: u64 = 300;

const ID_OPEN: usize = 1;
const ID_AUTOMATIC: usize = 2;
const ID_SOUND_SETTINGS: usize = 3;
const ID_QUIT: usize = 4;
const ID_SETTINGS: usize = 5;
const ID_MASTER: usize = 6;
const ID_VOLUME_MIXER: usize = 7;
const ID_SOUNDS: usize = 8;
const ID_TROUBLESHOOT: usize = 9;
const ID_TARGET: usize = 100;
const ID_MIXER: usize = 500;

/// Master volume change per wheel notch over the tray icon, like the volume keys.
const WHEEL_STEP: f32 = 0.02;

/// Volume change per wheel notch over a menu entry, matching the flyout.
const MENU_STEP: f32 = 0.02;

/// Volume id of the master volume in the menu.
const MASTER: &str = "master";

/// The menu entry a left click started on, while the click is held back from the
/// menu so it stays open. Zero when none.
static MENU_PRESS: AtomicU32 = AtomicU32::new(0);

/// The held click picks a media key app rather than muting.
static MENU_PRESS_PICKS: AtomicBool = AtomicBool::new(false);

/// Menu entries a left click picks for media keys: Automatic and each app.
fn picks_on_click(command: u32) -> bool {
    let command = command as usize;
    command == ID_AUTOMATIC || (ID_TARGET..ID_MIXER).contains(&command)
}

/// Menu entries a left click mutes: the master volume and other apps.
fn mutes_on_click(command: u32) -> bool {
    command as usize == ID_MASTER || command as usize >= ID_MIXER
}

/// How long after the pointer last hovered the icon its cached position is trusted.
const HOVER_TRUST_MS: u64 = 30_000;

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
static TRAY: AtomicIsize = AtomicIsize::new(0);
pub(crate) static FLYOUT_WINDOW: AtomicIsize = AtomicIsize::new(0);
static DISMISS_CLICK: AtomicBool = AtomicBool::new(false);
// Created, drawn and replaced only on the tray thread.
static CURRENT_ICON: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// The open context menu, for the mouse hook. Zero when no menu is open.
static MENU: AtomicIsize = AtomicIsize::new(0);
static MENU_WHEEL: Mutex<Vec<WheelItem>> = Mutex::new(Vec::new());
/// The open menu's media key apps by position after `ID_TARGET`, for picking one.
static MENU_TARGETS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static ROUTING: AtomicBool = AtomicBool::new(false);

/// What the tray shows. The worker writes it; the tray thread draws it.
static TRAY_STATE: Mutex<TrayState> = Mutex::new(TrayState {
    level: 0.0,
    muted: false,
    style: TrayStyle::Ring,
    middle: TrayMiddle::Speaker,
    tooltip: TooltipStyle::NowPlaying,
    buttons: MediaButtons::Hover,
    accent: Accent::Cyan,
    dark: None,
    device: None,
    media: card::Media { app: None, playback: Playback::Unknown, running: false, track: None, icon: None },
});

struct TrayState {
    level: f32,
    muted: bool,
    style: TrayStyle,
    middle: TrayMiddle,
    tooltip: TooltipStyle,
    buttons: MediaButtons,
    accent: Accent,
    dark: Option<bool>,
    device: Option<String>,
    media: card::Media,
}

impl TrayState {
    /// The plain tooltip text, which also goes to the shell for accessibility.
    fn text(&self) -> String {
        let volume = if self.muted { "Muted".to_owned() } else { format!("{}%", (self.level * 100.0).round()) };
        let device = self.device.as_deref().unwrap_or("Volume");
        let keys = self.media.app.as_deref().unwrap_or("Automatic");
        let mut text = format!("{device}: {volume}\nMedia keys: {keys}");
        if self.media.running
            && let Some(track) = &self.media.track
        {
            let state = match self.media.playback {
                Playback::Playing => "Playing",
                Playback::Paused => "Paused",
                Playback::Stopped => "Stopped",
                Playback::Unknown => "Idle",
            };
            text.push_str(&format!("\n{state} · {track}"));
        }
        text
    }

    /// Built on every redraw, so a wheel step shows its new volume right away.
    fn tip(&self) -> Tip {
        match self.tooltip {
            TooltipStyle::Text => Tip::Text(self.text()),
            style => {
                let dark = self.dark.unwrap_or_else(|| !card::apps_light());
                Tip::Card(card::Content {
                    style,
                    buttons: self.buttons,
                    dark,
                    accent: self.accent.colors(dark).accent,
                    device: self.device.clone(),
                    level: self.level,
                    muted: self.muted,
                    media: self.media.clone(),
                })
            }
        }
    }
}

/// When the pointer left both the icon and the card, or zero while it is on one.
static AWAY_MS: AtomicU64 = AtomicU64::new(0);
/// A previewed tooltip stays until this time (ms since the epoch).
static PREVIEW_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
/// How long a tooltip preview shows.
const PREVIEW_MS: u64 = 4000;

/// Shows the tooltip by the tray icon for a few seconds, so a changed tooltip
/// setting can be seen without hovering the icon.
pub(crate) fn preview_tooltip() {
    post(WM_PREVIEW, 0);
}

/// Set while the flyout shows its apps without the settings.
static FLYOUT_QUIET: AtomicBool = AtomicBool::new(false);

/// The page the flyout shows, or `None` once it closes.
pub(crate) fn set_flyout_page(page: Option<Page>) {
    FLYOUT_QUIET.store(page == Some(Page::Apps), Relaxed);
}

/// Whether `tip`, shown by `icon`, would cover the flyout's apps. A pinned flyout
/// dragged away from the icon leaves room for it.
fn tooltip_covers_flyout(tip: &Tip, icon: RECT) -> bool {
    use windows::Win32::Graphics::Gdi::GetWindowRgnBox;
    if !FLYOUT_QUIET.load(Relaxed) {
        return false;
    }
    let flyout = HWND(FLYOUT_WINDOW.load(Relaxed) as _);
    let (mut window, mut shown) = (RECT::default(), RECT::default());
    let Some(tip) = tooltip::screen_rect(tip, icon) else { return true };
    if unsafe { GetWindowRect(flyout, &mut window) }.is_err() {
        return true;
    }
    // The region holds only the visible cards, in window coordinates.
    let shown = if unsafe { GetWindowRgnBox(flyout, &mut shown) }.0 > 1 {
        RECT {
            left: window.left + shown.left,
            top: window.top + shown.top,
            right: window.left + shown.right,
            bottom: window.top + shown.bottom,
        }
    } else {
        window
    };
    tip.left < shown.right && tip.right > shown.left && tip.top < shown.bottom && tip.bottom > shown.top
}

/// Resolves the same localized accessibility label as Explorer's system tray.
fn system_tray_label(resource: &str, fallback: &str) -> String {
    use crate::util::{from_wide, wide};
    use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
    use windows::Win32::UI::Shell::SHLoadIndirectString;

    let mut dir = [0; 260];
    let len = unsafe { GetWindowsDirectoryW(Some(&mut dir)) } as usize;
    let mut label = [0; 128];
    if len > 0 && len < dir.len() {
        let source = wide(&format!(
            "@{{{}\\SystemApps\\MicrosoftWindows.Client.Core_cw5n1h2txyewy\\resources.pri?ms-resource://MicrosoftWindows.Client.Core/SystemTray/Resources/{resource}}}",
            from_wide(&dir)
        ));
        if unsafe { SHLoadIndirectString(PCWSTR(source.as_ptr()), &mut label, None) }.is_ok() && label[0] != 0 {
            return from_wide(&label);
        }
    }
    fallback.into()
}

fn visible_xaml_overflow() -> Option<HWND> {
    unsafe {
        FindWindowW(w!("TopLevelWindowForOverflowXamlIsland"), None).ok().filter(|&w| IsWindowVisible(w).as_bool())
    }
}

/// Invoke Explorer's own caret before showing the flyout. Looking it up on the
/// windowless COM worker keeps the tray callback free to answer Explorer.
pub(crate) fn close_xaml_overflow() -> windows::core::Result<()> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationInvokePattern, PropertyConditionFlags,
        PropertyConditionFlags_IgnoreCase, PropertyConditionFlags_MatchSubstring, TreeScope_Descendants,
        UIA_AutomationIdPropertyId, UIA_InvokePatternId, UIA_NamePropertyId,
    };

    let Some(overflow) = visible_xaml_overflow() else { return Ok(()) };
    unsafe {
        let ui: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let bar = ui.ElementFromHandle(FindWindowW(w!("Shell_TrayWnd"), None)?)?;
        let icon = ui.CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from("SystemTrayIcon"))?;
        // The expanded button's accessible name can include the localized "Hide" suffix.
        let label = system_tray_label("AutomationName_ChevronButton", "Show hidden icons");
        let name = ui.CreatePropertyConditionEx(
            UIA_NamePropertyId,
            &VARIANT::from(label.as_str()),
            PropertyConditionFlags(PropertyConditionFlags_IgnoreCase.0 | PropertyConditionFlags_MatchSubstring.0),
        )?;
        let caret = bar.FindFirst(TreeScope_Descendants, &ui.CreateAndCondition(&icon, &name)?)?;
        let invoke: IUIAutomationInvokePattern = caret.GetCurrentPatternAs(UIA_InvokePatternId)?;
        if IsWindowVisible(overflow).as_bool() {
            invoke.Invoke()?;
        }
    }
    Ok(())
}

/// Closes the classic Windows 10 tray overflow.
fn close_overflow() {
    use windows::Win32::UI::WindowsAndMessaging::FindWindowExW;
    unsafe {
        if let Ok(overflow) = FindWindowW(w!("NotifyIconOverflowWindow"), None)
            && IsWindowVisible(overflow).as_bool()
        {
            let button = FindWindowW(w!("Shell_TrayWnd"), None)
                .and_then(|bar| FindWindowExW(Some(bar), None, w!("TrayNotifyWnd"), None))
                .and_then(|area| FindWindowExW(Some(area), None, w!("Button"), None));
            match button {
                Ok(button) => _ = PostMessageW(Some(button), BM_CLICK, WPARAM(0), LPARAM(0)),
                Err(_) => _ = ShowWindow(overflow, SW_HIDE),
            }
        }
    }
}

/// True while the tooltip should stay: the pointer is on the icon, on a card with
/// buttons, or has been off both for less than the grace period.
fn keep_tooltip() -> bool {
    if now_ms() < PREVIEW_UNTIL_MS.load(Relaxed) {
        return true;
    }
    if tooltip::pointer_on_icon() {
        AWAY_MS.store(0, Relaxed);
        return true;
    }
    if !card::interactive() {
        return false;
    }
    if card::pointer_on_card() {
        AWAY_MS.store(0, Relaxed);
        return true;
    }
    let now = now_ms();
    let away = match AWAY_MS.load(Relaxed) {
        0 => {
            AWAY_MS.store(now, Relaxed);
            now
        }
        at => at,
    };
    now - away < CARD_GRACE_MS
}

/// Sets the master volume from a card's bar and redraws the tray at once, rather
/// than waiting for the worker's round trip.
pub(crate) fn set_master_from_card(level: f32) {
    if let Some((level, muted)) = audio::set_master_level(level) {
        {
            let mut tray = TRAY_STATE.lock().unwrap();
            tray.level = level;
            tray.muted = muted;
        }
        post(WM_REDRAW, 0);
    }
}

/// Sends a media key from a card button.
pub(crate) fn press(key: MediaKey) {
    shared().send(Cmd::Press(key));
}

/// The icon's screen rectangle as last reported by the shell, for the mouse hook.
static ICON_RECT: [AtomicI32; 4] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];
static LAST_HOVER_MS: AtomicU64 = AtomicU64::new(0);

/// Bit per media key that is currently held, to ignore auto-repeat.
static HELD: AtomicU32 = AtomicU32::new(0);

/// Returns false when another instance runs, after asking it to show its window.
/// Both front ends share the lock, so only one of them runs. `--dev` turns it off
/// for running them side by side.
pub fn claim_single_instance() -> bool {
    if crate::autostart::dev() {
        return true;
    }
    // After a portable update the old copy is still on its way out, so wait for it.
    let tries = if std::env::args().any(|a| a == crate::update::UPDATED_ARG) { 50 } else { 1 };
    unsafe {
        for attempt in 0..tries {
            let mutex = CreateMutexW(None, true, w!("Local\\AudioBartender.Instance"));
            if mutex.is_ok() && GetLastError() != ERROR_ALREADY_EXISTS {
                return true;
            }
            if let Ok(mutex) = mutex {
                let _ = CloseHandle(mutex);
            }
            if attempt + 1 < tries {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
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

pub fn spawn(shared: Arc<Shared>, speaker_position: bool) {
    let _ = SHARED.set(shared);
    speaker_slot::enable(speaker_position && crate::util::windows_11());
    std::thread::Builder::new().name("bartender-hook".into()).spawn(run_hook).expect("spawn hook thread");
    std::thread::Builder::new().name("bartender-tray".into()).spawn(run_tray).expect("spawn tray thread");
}

fn shared() -> &'static Shared {
    SHARED.get().expect("core started")
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
    ROUTING.store(state.routed().is_some(), Relaxed);
    speaker_slot::enable(state.settings.speaker_position && crate::util::windows_11());

    let audio = &state.audio;
    {
        let mut tray = TRAY_STATE.lock().unwrap();
        tray.level = audio.volume;
        tray.muted = audio.muted;
        tray.style = state.settings.tray_style;
        tray.middle = state.settings.tray_middle;
        tray.tooltip = state.settings.tooltip;
        tray.buttons = state.settings.media_buttons;
        tray.accent = state.settings.accent;
        tray.dark = state.settings.dark;
        tray.device = audio.device().map(|d| d.name.clone());
        tray.media = match state.now_playing() {
            Some(t) => card::Media {
                app: Some(t.name.clone()),
                playback: t.playback,
                running: t.running,
                track: t.track.clone(),
                icon: t.icon.clone(),
            },
            None => card::Media::default(),
        };
    }
    post(WM_REDRAW, 0);
}

pub fn remove_tray() {
    let hwnd = HWND(TRAY.load(Relaxed) as _);
    if !hwnd.is_invalid() {
        delete_icon(hwnd);
    }
}

fn delete_icon(hwnd: HWND) -> bool {
    let mut data = NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, ..Default::default() };
    identify(&mut data);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &data).as_bool() }
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
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Relaxed);

        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("Audio Bartender"),
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
        // An elevated window gets messages from Explorer and from an unelevated second
        // launch only when it allows them.
        for msg in [WM_TRAY, WM_SHOW, TASKBAR_CREATED.load(Relaxed)] {
            let _ = ChangeWindowMessageFilterEx(hwnd, msg, MSGFLT_ALLOW, None);
        }
        tooltip::create(hwnd);
        card::create();
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
    // A click on a volume entry of the open menu mutes it: a left click on the master
    // volume and other apps, a right click on any volume, media key apps included. The
    // menu never sees the click, so it stays open.
    let button = wparam.0 as u32;
    let menu = HMENU(MENU.load(Relaxed) as _);
    let clicks = [WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP];
    if code == HC_ACTION as i32 && clicks.contains(&button) && !menu.is_invalid() {
        let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let position = unsafe { MenuItemFromPoint(None, menu, event.pt) };
        let command = if position >= 0 { unsafe { GetMenuItemID(menu, position) } } else { 0 };
        let has_volume = MENU_WHEEL.lock().unwrap().iter().any(|w| w.command == command);
        let mutes = match button {
            WM_LBUTTONDOWN => has_volume && mutes_on_click(command),
            WM_RBUTTONDOWN => has_volume,
            _ => false,
        };
        let picks = button == WM_LBUTTONDOWN && picks_on_click(command);
        if mutes || picks {
            MENU_PRESS.store(command, Relaxed);
            MENU_PRESS_PICKS.store(picks, Relaxed);
            return LRESULT(1);
        }
        if button == WM_LBUTTONUP || button == WM_RBUTTONUP {
            let pressed = MENU_PRESS.swap(0, Relaxed);
            if pressed != 0 {
                if pressed == command {
                    let message = if MENU_PRESS_PICKS.load(Relaxed) { WM_MENU_SELECT } else { WM_MENU_MUTE };
                    post_with(message, 0, command as isize);
                }
                return LRESULT(1);
            }
        }
    }
    if code == HC_ACTION as i32 {
        // When Explorer is above the overlay it receives the down event. Pair
        // it with its own up even if the shell changes stacking mid-click.
        static SPEAKER_BUTTONS: AtomicU32 = AtomicU32::new(0);
        let message = wparam.0 as u32;
        let button = match message {
            WM_LBUTTONDOWN | WM_LBUTTONUP => 1,
            WM_RBUTTONDOWN | WM_RBUTTONUP => 2,
            WM_MBUTTONDOWN | WM_MBUTTONUP => 4,
            _ => 0,
        };
        if button != 0 {
            let point = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt };
            let down = matches!(message, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN);
            if down && speaker_slot::native_speaker_at(point) {
                SPEAKER_BUTTONS.fetch_or(button, Relaxed);
                return LRESULT(1);
            }
            if !down && SPEAKER_BUTTONS.fetch_and(!button, Relaxed) & button != 0 {
                if speaker_slot::native_speaker_at(point)
                    || speaker_slot::visible_window().is_some_and(|w| unsafe { WindowFromPoint(point) == w })
                {
                    let event = if message == WM_RBUTTONUP { WM_CONTEXTMENU } else { message };
                    post_with(WM_TRAY, 0, event as isize);
                }
                return LRESULT(1);
            }
        }
        match wparam.0 as u32 {
            WM_LBUTTONDOWN => {
                let flyout = HWND(FLYOUT_WINDOW.load(Relaxed) as _);
                let point = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt };
                let mut pid = 0;
                if !flyout.is_invalid()
                    && unsafe { IsWindowVisible(flyout).as_bool() }
                    && unsafe { GetWindowThreadProcessId(flyout, Some(&mut pid)) } != 0
                    && pid == std::process::id()
                    && speaker_slot::on_neighbor(point)
                {
                    DISMISS_CLICK.store(true, Relaxed);
                    post(WM_DISMISS, 0);
                    return LRESULT(1);
                }
            }
            WM_LBUTTONUP if DISMISS_CLICK.swap(false, Relaxed) => return LRESULT(1),
            _ => {}
        }
    }
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
        let on_icon = speaker_slot::visible_window().is_some_and(|slot| unsafe { WindowFromPoint(event.pt) == slot })
            || speaker_slot::native_speaker_at(event.pt)
            || (fresh && x >= l && x < r && y >= t && y < b);
        if on_icon || card::contains(event.pt) {
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
                let icon = icon_rect(hwnd).unwrap_or_else(cursor_rect);
                let tip = TRAY_STATE.lock().unwrap().tip();
                if !tooltip_covers_flyout(&tip, icon) {
                    tooltip::show(&tip, icon);
                }
            }
            // Redrawing the icon also makes the shell close the tooltip, so while the
            // pointer stays on the icon, or heads for a card with buttons, keep it and
            // watch for the pointer to leave.
            NIN_POPUPCLOSE => {
                if tooltip::pointer_on_icon() || card::interactive() {
                    AWAY_MS.store(0, Relaxed);
                    unsafe {
                        SetTimer(Some(hwnd), TIMER_TOOLTIP, 100, None);
                    }
                } else {
                    tooltip::hide();
                }
            }
            WM_LBUTTONUP => {
                tooltip::hide();
                if visible_xaml_overflow().is_some() {
                    shared().send(Cmd::ToggleOverflowFlyout(anchor(hwnd)));
                } else {
                    close_overflow();
                    shared().emit(Event::Flyout { anchor: anchor(hwnd), page: Page::Apps, toggle: true });
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
        WM_MENU_MUTE => unsafe { menu_mute(lparam.0 as u32) },
        WM_MENU_SELECT => unsafe { menu_select(lparam.0 as u32) },
        WM_TIMER if wparam.0 == TIMER_TOOLTIP => {
            if tooltip::visible() && keep_tooltip() {
                return LRESULT(0);
            }
            tooltip::hide();
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_TOOLTIP);
            }
        }
        WM_PREVIEW => {
            let tip = TRAY_STATE.lock().unwrap().tip();
            let icon = shown_icon_rect(hwnd).unwrap_or_else(|| {
                let r = default_anchor().icon;
                RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
            });
            tooltip::show(&tip, icon);
            PREVIEW_UNTIL_MS.store(now_ms() + PREVIEW_MS, Relaxed);
            AWAY_MS.store(0, Relaxed);
            unsafe {
                SetTimer(Some(hwnd), TIMER_TOOLTIP, 100, None);
            }
        }
        WM_TIMER if wparam.0 == TIMER_RESTORE_ICON => unsafe { set_icon(hwnd, NIM_MODIFY) },
        WM_TIMER if wparam.0 == TIMER_LIVE => {
            // Against the recent peak, so quiet and loud audio both move the icon.
            let raw = crate::meters::loudest();
            let peak = raw.max(f32::from_bits(PEAK.load(Relaxed)) * PEAK_FALL);
            PEAK.store(peak.to_bits(), Relaxed);
            let sound = if raw > 0.005 { (raw / peak.max(MIN_PEAK)).min(1.0) } else { 0.0 };
            let drawn = f32::from_bits(SOUND.load(Relaxed));
            let moving = TRAY_STATE.lock().unwrap().style == TrayStyle::Equalizer && sound > 0.01;
            if moving || (sound - drawn).abs() > 0.015 || (sound == 0.0 && drawn != 0.0) {
                SOUND.store(sound.to_bits(), Relaxed);
                unsafe { set_icon(hwnd, NIM_MODIFY) };
            }
        }
        WM_SHOW => show(Page::Apps),
        WM_DISMISS => shared().emit(Event::DismissFlyout),
        WM_SPEAKER_SLOT => {
            if speaker_slot::place() {
                LAST_HOVER_MS.store(0, Relaxed);
                tooltip::hide();
                unsafe { set_icon(hwnd, NIM_MODIFY) };
                shared().send(Cmd::RefreshAudio { devices: false });
            }
        }
        WM_REDRAW => unsafe { set_icon(hwnd, NIM_MODIFY) },
        WM_POWERBROADCAST if matches!(wparam.0 as u32, PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC) => {
            speaker_slot::reset();
            unsafe { set_icon(hwnd, NIM_MODIFY) };
            return LRESULT(1);
        }
        WM_DISPLAYCHANGE if speaker_slot::enabled() => {
            speaker_slot::reset();
            unsafe { set_icon(hwnd, NIM_MODIFY) };
        }
        WM_SETTINGCHANGE => unsafe { set_icon(hwnd, NIM_MODIFY) },
        _ if msg == TASKBAR_CREATED.load(Relaxed) => {
            speaker_slot::reset();
            LAST_HOVER_MS.store(0, Relaxed);
            unsafe { set_icon(hwnd, NIM_ADD) };
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

pub(crate) fn speaker_position_active() -> bool {
    speaker_slot::reachable()
}

/// Whether this app's tray icon shows on the taskbar rather than in the overflow.
/// Windows 11 keeps that in the registry. On Windows 10 the icon is on the taskbar
/// when its rectangle lies inside the taskbar's.
pub(crate) fn icon_on_taskbar() -> Option<bool> {
    if speaker_slot::enabled() {
        return None;
    }
    if crate::util::windows_11() {
        return crate::util::tray_icon_promoted();
    }
    let hwnd = HWND(TRAY.load(Relaxed) as _);
    if hwnd.is_invalid() {
        return None;
    }
    unsafe {
        let icon = Shell_NotifyIconGetRect(&identifier(hwnd)).ok()?;
        let taskbar = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
        let mut bar = RECT::default();
        windows::Win32::UI::WindowsAndMessaging::GetWindowRect(taskbar, &mut bar).ok()?;
        Some(icon.left >= bar.left && icon.right <= bar.right && icon.top >= bar.top && icon.bottom <= bar.bottom)
    }
}

fn icon_rect(hwnd: HWND) -> Option<RECT> {
    let mut at = POINT::default();
    unsafe { GetCursorPos(&mut at).ok()? };
    let inside = |r: &RECT| at.x >= r.left && at.x < r.right && at.y >= r.top && at.y < r.bottom;
    speaker_slot::rect()
        .filter(inside)
        .or_else(|| unsafe { Shell_NotifyIconGetRect(&identifier(hwnd)).ok().filter(inside) })
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
    if let Some(anchor) = speaker_slot::anchor() {
        return anchor;
    }
    let hwnd = HWND(TRAY.load(Relaxed) as _);
    let icon = unsafe { Shell_NotifyIconGetRect(&identifier(hwnd)) }.unwrap_or_else(|_| {
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

/// Where the icon shows on the taskbar, wherever the pointer is: over the Windows
/// speaker, among the tray icons, or else the overflow button that hides it.
fn shown_icon_rect(hwnd: HWND) -> Option<RECT> {
    if let Some(r) = speaker_slot::rect() {
        return Some(r);
    }
    let bar = taskbar_rect()?;
    let on_bar = |r: &RECT| {
        r.right > r.left && r.left >= bar.left && r.right <= bar.right && r.top >= bar.top && r.bottom <= bar.bottom
    };
    unsafe { Shell_NotifyIconGetRect(&identifier(hwnd)) }
        .ok()
        .filter(on_bar)
        .or_else(|| overflow_button().filter(on_bar))
}

fn taskbar_rect() -> Option<RECT> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut rect = RECT::default();
    unsafe { GetWindowRect(FindWindowW(w!("Shell_TrayWnd"), None).ok()?, &mut rect).ok()? };
    Some(rect)
}

/// A 1-pixel rectangle at the pointer.
fn cursor_rect() -> RECT {
    let mut at = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut at);
    }
    RECT { left: at.x, top: at.y, right: at.x + 1, bottom: at.y + 1 }
}

/// The button that opens the tray overflow. Windows 10 gives it a window. On
/// Windows 11 it leads the notification area, so take that area's first square.
fn overflow_button() -> Option<RECT> {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowRect};
    unsafe {
        let taskbar = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
        let area = FindWindowExW(Some(taskbar), None, w!("TrayNotifyWnd"), None).ok()?;
        let mut rect = RECT::default();
        if let Ok(button) = FindWindowExW(Some(area), None, w!("Button"), None)
            && GetWindowRect(button, &mut rect).is_ok()
            && rect.right > rect.left
        {
            return Some(rect);
        }
        GetWindowRect(area, &mut rect).ok()?;
        let side = (rect.right - rect.left).min(rect.bottom - rect.top);
        Some(RECT { right: rect.left + side, bottom: rect.top + side, ..rect })
    }
}

fn anchor(hwnd: HWND) -> Anchor {
    remember_icon_rect(hwnd, true);
    // In the overflow area, anchor on the pointer instead.
    anchor_for(icon_rect(hwnd).unwrap_or_else(cursor_rect))
}

/// The anchor for an icon rectangle: its monitor's work area and scale.
pub(crate) fn anchor_for(icon: RECT) -> Anchor {
    unsafe {
        let monitor = MonitorFromRect(&icon, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let (mut dpi, mut unused) = (96u32, 0u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
        let rect = |r: RECT| Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
        Anchor { icon: rect(icon), work: rect(info.rcWork), screen: rect(info.rcMonitor), scale: dpi as f32 / 96.0 }
    }
}

/// Cleared when Explorer requires window-and-number identification for the current icon.
static USE_GUID: AtomicBool = AtomicBool::new(true);

/// The tray icon's fixed identity, which lets Windows remember where the user put it
/// across restarts. It comes from the exe's path, because Windows binds a GUID to one
/// path: each copy, and each front end, gets its own.
pub(crate) fn ordinary_icon_guid() -> GUID {
    static ID: OnceLock<GUID> = OnceLock::new();
    *ID.get_or_init(|| {
        let name = std::env::current_exe().map(|p| p.to_string_lossy().to_lowercase()).unwrap_or_default();
        // FNV-1a, so the same path always gives the same GUID.
        let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
        GUID::from_u128(0x5f3a_9c1e_7b42_4d08_0000_0000_0000_0000 | hash as u128)
    })
}

/// Marks `data` with the fixed identity while the shell accepts it.
fn identify(data: &mut NOTIFYICONDATAW) {
    if USE_GUID.load(Relaxed) {
        data.uFlags |= NIF_GUID;
        data.guidItem = ordinary_icon_guid();
    }
}

fn identifier(hwnd: HWND) -> NOTIFYICONIDENTIFIER {
    let guid = if USE_GUID.load(Relaxed) { ordinary_icon_guid() } else { GUID::zeroed() };
    NOTIFYICONIDENTIFIER { cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: hwnd, guidItem: guid, uID: 0 }
}

/// Draws the current volume and pushes it to the shell with `action`.
unsafe fn set_icon(hwnd: HWND, action: NOTIFY_ICON_MESSAGE) {
    let (look, tip) = {
        let tray = TRAY_STATE.lock().unwrap();
        let media = &tray.media;
        let playing = match media.playback {
            _ if !media.running => None,
            Playback::Playing => Some(true),
            Playback::Paused | Playback::Stopped => Some(false),
            Playback::Unknown => None,
        };
        let look = Look {
            level: tray.level,
            muted: tray.muted,
            style: tray.style,
            middle: tray.middle,
            light_taskbar: tray_icon::light_taskbar(),
            sound: f32::from_bits(SOUND.load(Relaxed)),
            playing,
            art: media.icon.clone().filter(|icon| icon.art),
            clock: now_ms(),
        };
        (look, (tray.text(), tray.tip()))
    };
    // Live styles keep a timer that redraws them with the sound.
    let live = crate::tray_icon_live(look.style, look.middle);
    if live != LIVE.swap(live, Relaxed) {
        unsafe {
            if live {
                SetTimer(Some(hwnd), TIMER_LIVE, LIVE_FRAME_MS, None);
            } else {
                let _ = KillTimer(Some(hwnd), TIMER_LIVE);
                SOUND.store(0, Relaxed);
            }
        }
    }
    let (text, tip) = tip;
    let slot = speaker_slot::visible_window();
    let size = slot.map_or_else(|| unsafe { GetSystemMetrics(SM_CXSMICON) }, speaker_slot::icon_size);
    let Some(icon) = tray_icon::render(&look, size) else {
        unsafe { SetTimer(Some(hwnd), TIMER_RESTORE_ICON, 1000, None) };
        return;
    };

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
    fill_wide(&mut data.szTip, &text);
    // Keep the registration warm so fallback retains the saved pin.
    data.uFlags |= NIF_STATE;
    data.dwStateMask = NIS_HIDDEN;
    if speaker_slot::reachable() && slot.is_some() {
        data.dwState = NIS_HIDDEN;
    }
    if let Some(slot) = slot {
        unsafe {
            let _ = InvalidateRect(Some(slot), None, false);
        }
    }
    identify(&mut data);
    tooltip::update(&tip);
    unsafe {
        // Clears an icon a crashed copy left behind, which would block the GUID. Only the
        // copy holding the single-instance lock may do this: with `--dev` another live copy
        // of this exe can own the GUID, and deleting would take its icon away.
        if action == NIM_ADD && USE_GUID.load(Relaxed) && !crate::autostart::dev() {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
        let mut registered = Shell_NotifyIconW(action, &data).as_bool();
        let adding = action == NIM_ADD || !registered;
        if !registered && action == NIM_MODIFY {
            registered = Shell_NotifyIconW(NIM_ADD, &data).as_bool();
        }
        if action == NIM_ADD && !registered && USE_GUID.swap(false, Relaxed) {
            crate::util::log("tray GUID refused, using the window identity");
            data.uFlags &= !NIF_GUID;
            data.guidItem = GUID::zeroed();
            registered = Shell_NotifyIconW(NIM_ADD, &data).as_bool();
        }
        if registered {
            let _ = KillTimer(Some(hwnd), TIMER_RESTORE_ICON);
        } else {
            SetTimer(Some(hwnd), TIMER_RESTORE_ICON, 1000, None);
        }
        if registered && adding {
            // Version 4 sends the hover events that drive our own tooltip.
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
        // The shell keeps its own copy, so the previous icon can go.
        let old = CURRENT_ICON.swap(icon.0 as isize, Relaxed);
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

/// Themes context menus: `dark` forces dark or light, `None` follows Windows.
unsafe fn theme_menus(dark: Option<bool>) {
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        let ordinal = |n: usize| GetProcAddress(uxtheme, PCSTR(n as *const u8));
        // SetPreferredAppMode: 1 follows Windows, 2 forces dark, 3 forces light.
        let mode = match dark {
            None => 1,
            Some(true) => 2,
            Some(false) => 3,
        };
        if let Some(f) = ordinal(135) {
            let set_preferred_app_mode: extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            set_preferred_app_mode(mode);
        }
        if let Some(f) = ordinal(136) {
            let flush_menu_themes: extern "system" fn() = std::mem::transmute(f);
            flush_menu_themes();
        }
    }
}

/// Opens `file` with `args` through the shell. Returns false if it couldn't start.
fn shell_open(file: &str, args: Option<&str>) -> bool {
    let (file, args) = (crate::util::wide(file), args.map(crate::util::wide));
    let args = args.as_ref().map_or(PCWSTR::null(), |a| PCWSTR(a.as_ptr()));
    let result = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(file.as_ptr()), args, None, SW_SHOWNORMAL) };
    // ShellExecute returns a value above 32 on success.
    result.0 as isize > 32
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

        theme_menus(state.settings.dark);
        // The bold default item opens the flyout. It names the version and front end,
        // so a bug report can say which one is running.
        add(MF_STRING, ID_OPEN, &crate::update::app_label());
        // From the top: other apps, media key apps, then the master volume right above
        // the Windows links.
        if !state.mixer.is_empty() {
            add(MF_SEPARATOR, 0, "");
            add(MF_STRING | MF_GRAYED, 0, "Other apps (click mutes)");
            for (i, app) in state.mixer.iter().enumerate() {
                wheel.push(WheelItem { command: (ID_MIXER + i) as u32, id: app.id.clone(), name: app.name.clone() });
                add(MF_STRING, ID_MIXER + i, &volume_label(&app.name, app.volume, app.muted));
            }
        }
        add(MF_SEPARATOR, 0, "");
        add(MF_STRING | MF_GRAYED, 0, "Send media keys to (right click mutes)");
        add(MF_STRING | check(state.selected.is_none()), ID_AUTOMATIC, "Automatic (Windows decides)");
        for (i, target) in state.targets.iter().enumerate() {
            let selected = state.selected.as_deref() == Some(target.id.as_str());
            let label = match target.volume {
                _ if !target.running => format!("{} (not running)", target.name),
                Some(level) => {
                    wheel.push(WheelItem {
                        command: (ID_TARGET + i) as u32,
                        id: target.id.clone(),
                        name: target.name.clone(),
                    });
                    volume_label(&target.name, level, target.muted)
                }
                None => target.name.clone(),
            };
            add(MF_STRING | check(selected), ID_TARGET + i, &label);
        }
        add(MF_SEPARATOR, 0, "");
        let audio = &state.audio;
        add(MF_STRING, ID_MASTER, &volume_label("Volume", audio.volume, audio.muted));
        wheel.push(WheelItem { command: ID_MASTER as u32, id: MASTER.into(), name: "Volume".into() });
        add(MF_STRING | MF_GRAYED, 0, "Scroll over a volume to change it");
        // The same Windows links as the Windows volume icon's menu.
        add(MF_SEPARATOR, 0, "");
        add(MF_STRING, ID_SOUND_SETTINGS, "Open Sound settings");
        add(MF_STRING, ID_VOLUME_MIXER, "Open Volume mixer");
        add(MF_STRING, ID_SOUNDS, "Sounds");
        add(MF_STRING, ID_TROUBLESHOOT, "Troubleshoot sound problems");
        add(MF_SEPARATOR, 0, "");
        add(MF_STRING, ID_SETTINGS, "Settings…");
        add(MF_STRING, ID_QUIT, "Quit");
        let _ = SetMenuDefaultItem(menu, ID_OPEN as u32, 0);

        *MENU_WHEEL.lock().unwrap() = wheel;
        *MENU_TARGETS.lock().unwrap() = state.targets.iter().map(|t| t.id.clone()).collect();
        MENU.store(menu.0 as isize, Relaxed);

        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // The menu only closes on outside clicks while its owner is foreground.
        let _ = SetForegroundWindow(hwnd);
        let mut flags = TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY;
        let mut monitor = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST), &mut monitor).as_bool() {
            // Explorer can cover the bottom menu item when Quick Settings has just
            // closed. Keep the whole menu, including Quit, in the work area.
            if at.y >= monitor.rcWork.bottom {
                flags |= TPM_BOTTOMALIGN;
                at.y = monitor.rcWork.bottom;
            } else {
                at.y = at.y.max(monitor.rcWork.top);
            }
        }
        let picked = TrackPopupMenu(menu, flags, at.x, at.y, None, hwnd, None);
        MENU.store(0, Relaxed);
        MENU_WHEEL.lock().unwrap().clear();
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        match picked.0 as usize {
            ID_OPEN => show(Page::Apps),
            ID_SETTINGS => show(Page::Settings),
            ID_MASTER => audio::toggle_master_mute(),
            ID_AUTOMATIC => shared().send(Cmd::Select(None)),
            ID_SOUND_SETTINGS => _ = shell_open("ms-settings:sound", None),
            // `-m` takes a point packed like an LPARAM and opens the mixer just above it.
            ID_VOLUME_MIXER => _ = shell_open("sndvol.exe", Some(&format!("-m {}", (at.y << 16) | (at.x & 0xFFFF)))),
            ID_SOUNDS => _ = shell_open("rundll32.exe", Some("shell32.dll,Control_RunDLL mmsys.cpl,,2")),
            ID_TROUBLESHOOT => {
                // Newer Windows builds retired msdt; the troubleshooter list replaces it.
                if !shell_open("msdt.exe", Some("-id AudioPlaybackDiagnostic")) {
                    shell_open("ms-settings:troubleshoot", None);
                }
            }
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
    if let Some((level, muted)) = level {
        unsafe { relabel_menu_item(command, &name, level, muted) };
    }
}

/// Mutes or unmutes a menu entry and rewrites its label while the menu stays open.
unsafe fn menu_mute(command: u32) {
    let item = MENU_WHEEL.lock().unwrap().iter().find(|w| w.command == command).map(|w| (w.id.clone(), w.name.clone()));
    let Some((id, name)) = item else { return };
    let level = if id == MASTER {
        audio::toggle_master_mute();
        audio::adjust_master(0.0)
    } else {
        volume::toggle_mute(&id);
        let sessions = volume::sessions(&crate::util::processes());
        volume::level(&sessions, &volume::exes_for(&id))
    };
    shared().send(Cmd::RefreshAudio { devices: false });
    if let Some((level, muted)) = level {
        unsafe { relabel_menu_item(command, &name, level, muted) };
    }
}

/// Picks where media keys go and moves the check mark while the menu stays open.
unsafe fn menu_select(command: u32) {
    let targets = MENU_TARGETS.lock().unwrap().clone();
    let picked = (command as usize).checked_sub(ID_TARGET).and_then(|i| targets.get(i).cloned());
    shared().send(Cmd::Select(picked));
    let menu = HMENU(MENU.load(Relaxed) as _);
    if menu.is_invalid() {
        return;
    }
    let check = |id: usize| if id == command as usize { MF_CHECKED.0 } else { MF_UNCHECKED.0 };
    unsafe {
        let _ = CheckMenuItem(menu, ID_AUTOMATIC as u32, check(ID_AUTOMATIC));
        for i in 0..targets.len() {
            let _ = CheckMenuItem(menu, (ID_TARGET + i) as u32, check(ID_TARGET + i));
        }
        if let Ok(popup) = FindWindowW(w!("#32768"), None) {
            let _ = InvalidateRect(Some(popup), None, true);
        }
    }
}

/// Shows a menu entry's new volume while the menu is open.
unsafe fn relabel_menu_item(command: u32, name: &str, level: f32, muted: bool) {
    let menu = HMENU(MENU.load(Relaxed) as _);
    if menu.is_invalid() {
        return;
    }
    let mut text = crate::util::wide(&volume_label(name, level, muted));
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
