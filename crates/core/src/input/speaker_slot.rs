//! Places an Audio Bartender window over the individual Quick Settings speaker.
//! UI Automation reads Explorer on a windowless MTA thread. The tray thread
//! paints and positions our child window and owns all of its input handling.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BeginPaint, COLOR_HIGHLIGHT, CreateSolidBrush, DIB_RGB_COLORS, DeleteObject,
    EndPaint, FillRect, GetMonitorInfoW, GetSysColor, HDC, InvalidateRect, MONITOR_DEFAULTTONULL, MONITORINFO,
    MonitorFromWindow, PAINTSTRUCT, SetDIBitsToDevice,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, HCF_HIGHCONTRASTON, HIGHCONTRASTW, IUIAutomation, IUIAutomationCacheRequest, IUIAutomationElement,
    TreeScope_Descendants, UIA_AutomationIdPropertyId, UIA_BoundingRectanglePropertyId, UIA_ButtonControlTypeId,
    UIA_ClassNamePropertyId, UIA_ControlTypePropertyId, UIA_IsOffscreenPropertyId, UIA_NamePropertyId,
    UIA_ProcessIdPropertyId,
};
use windows::Win32::UI::Controls::{HOVER_DEFAULT, WM_MOUSEHOVER, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_HOVER, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{
    ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA, NIN_POPUPCLOSE, NIN_POPUPOPEN, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use super::{CURRENT_ICON, WM_SPEAKER_SLOT, WM_TRAY, WM_WHEEL, post, post_with};
use crate::util;

const CLASS: PCWSTR = w!("AudioBartender.SpeakerSlot");
const PLACEMENT_LIFETIME: Duration = Duration::from_secs(4);
const TIMER_PLACEMENT: usize = 1;
const TIMER_HOVER_TINT: usize = 2;
const WM_TRACK_TINT: u32 = WM_APP + 1;
const HOVER_FADE_MS: u64 = 250;
static ENABLED: AtomicBool = AtomicBool::new(false);
static WAKE: OnceLock<Sender<()>> = OnceLock::new();
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static BACKGROUND: AtomicU32 = AtomicU32::new(0x303030);
static HOVERED: AtomicBool = AtomicBool::new(false);
static TINT_UNTIL: AtomicU64 = AtomicU64::new(0);
static PAINTED: AtomicBool = AtomicBool::new(false);
static REACHABLE: AtomicBool = AtomicBool::new(false);
static FOUND: Mutex<Option<Placement>> = Mutex::new(None);

#[derive(Clone, Copy)]
struct Placement {
    taskbar: isize,
    bar: RECT,
    button: RECT,
    neighbors: [RECT; 2],
    tile: RECT,
    observed: Instant,
}

pub(super) fn enable(on: bool) {
    if ENABLED.swap(on, Relaxed) == on {
        return;
    }
    let wake = WAKE.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("bartender-speaker-position".into())
            .spawn(move || {
                util::com_init();
                unsafe {
                    SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
                }
                let mut automation = None;
                let label = super::system_tray_label("AutomationName_VolumeStateButton", "Volume");
                loop {
                    if ENABLED.load(Relaxed) {
                        if automation.is_none() {
                            automation = automation_client();
                        }
                        let found = automation.as_ref().and_then(|(ui, cache)| locate(ui, cache, &label));
                        *FOUND.lock().unwrap() = found;
                        post(WM_SPEAKER_SLOT, 0);
                        if rx.recv_timeout(Duration::from_secs(1)) == Err(mpsc::RecvTimeoutError::Disconnected) {
                            break;
                        }
                    } else {
                        automation = None;
                        if rx.recv().is_err() {
                            break;
                        }
                    }
                }
            })
            .expect("spawn speaker position worker");
        tx
    });
    *FOUND.lock().unwrap() = None;
    let _ = wake.send(());
    post(WM_SPEAKER_SLOT, 0);
}

fn named_speaker(name: &str, label: &str) -> bool {
    !label.is_empty()
        && name
            .strip_prefix(label)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with(':'))
}

// One provider request per refresh, even though we check several properties.
fn automation_client() -> Option<(IUIAutomation, IUIAutomationCacheRequest)> {
    unsafe {
        let ui: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let cache = ui.CreateCacheRequest().ok()?;
        for id in [
            UIA_ProcessIdPropertyId,
            UIA_ControlTypePropertyId,
            UIA_AutomationIdPropertyId,
            UIA_IsOffscreenPropertyId,
            UIA_ClassNamePropertyId,
            UIA_NamePropertyId,
            UIA_BoundingRectanglePropertyId,
        ] {
            cache.AddProperty(id).ok()?;
        }
        Some((ui, cache))
    }
}

fn button_rect(element: &IUIAutomationElement, pid: u32, label: &str) -> Option<RECT> {
    unsafe {
        let class = element.CachedClassName().ok()?.to_string();
        if element.CachedProcessId().ok()? as u32 != pid
            || element.CachedControlType().ok()? != UIA_ButtonControlTypeId
            || element.CachedIsOffscreen().ok()?.as_bool()
            || element.CachedAutomationId().ok()? != "SystemTrayIcon"
            || !(class.starts_with("SystemTray.OmniButton") || class == "SystemTray.AccentButton")
            || !named_speaker(&element.CachedName().ok()?.to_string(), label)
        {
            return None;
        }
        element.CachedBoundingRectangle().ok()
    }
}

fn contains(outer: RECT, inner: RECT) -> bool {
    inner.left >= outer.left
        && inner.top >= outer.top
        && inner.right <= outer.right
        && inner.bottom <= outer.bottom
        && inner.left < inner.right
        && inner.top < inner.bottom
}

fn locate(ui: &IUIAutomation, cache: &IUIAutomationCacheRequest, label: &str) -> Option<Placement> {
    unsafe {
        let taskbar = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
        let mut pid = 0;
        let mut shell_pid = 0;
        GetWindowThreadProcessId(taskbar, Some(&mut pid));
        GetWindowThreadProcessId(GetShellWindow(), Some(&mut shell_pid));
        if pid == 0 || pid != shell_pid || !IsWindowVisible(taskbar).as_bool() {
            return None;
        }
        // A separate window cannot follow the shell's auto-hide animation atomically.
        let mut appbar = APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, ..Default::default() };
        if SHAppBarMessage(ABM_GETSTATE, &mut appbar) & ABS_AUTOHIDE as usize != 0 {
            return None;
        }
        let mut bar = RECT::default();
        GetWindowRect(taskbar, &mut bar).ok()?;
        let root = ui.ElementFromHandle(taskbar).ok()?;
        let condition =
            ui.CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from("SystemTrayIcon")).ok()?;
        let items = root.FindAllBuildCache(TreeScope_Descendants, &condition, cache).ok()?;
        let mut matched = None;
        let mut accents = Vec::new();
        for index in 0..items.Length().ok()? {
            let item = items.GetElement(index).ok()?;
            if let Some(button) = button_rect(&item, pid, label).filter(|r| contains(bar, *r)) {
                if matched.replace(button).is_some() {
                    return None;
                }
            } else if item.CachedClassName().ok()? == "SystemTray.AccentButton"
                && !item.CachedIsOffscreen().ok()?.as_bool()
            {
                accents.push(item.CachedBoundingRectangle().ok()?);
            }
        }
        let button = matched?;
        let gap = (button.right - button.left) / 2;
        let mut neighbors = [RECT::default(); 2];
        for r in accents {
            if contains(bar, r) && r.top == button.top && r.bottom == button.bottom {
                if (0..=gap).contains(&(button.left - r.right)) {
                    neighbors[0] = r;
                }
                if (0..=gap).contains(&(r.left - button.right)) {
                    neighbors[1] = r;
                }
            }
        }
        Some(Placement {
            taskbar: taskbar.0 as isize,
            bar,
            button,
            neighbors,
            tile: tile(button, neighbors, GetDpiForWindow(taskbar))?,
            observed: Instant::now(),
        })
    }
}

pub(super) fn visible_window() -> Option<HWND> {
    let hwnd = HWND(WINDOW.load(Relaxed) as _);
    (!hwnd.is_invalid() && unsafe { IsWindowVisible(hwnd).as_bool() }).then_some(hwnd)
}

fn inside(p: POINT, r: RECT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

/// Explorer animates the group when the pointer crosses Wi-Fi, battery or our
/// child window. Wake the tray thread for the fade, including the last move out.
pub(super) fn pointer_moved(point: POINT) {
    static WAS_NEAR: AtomicBool = AtomicBool::new(false);
    if !enabled() {
        return;
    }
    let near = {
        let Ok(found) = FOUND.try_lock() else { return };
        found.is_some_and(|p| {
            let span = 2 * (p.tile.right - p.tile.left);
            inside(point, p.bar)
                && (p.tile.left - span..p.tile.right + span).contains(&point.x)
                && (p.tile.top - span..p.tile.bottom + span).contains(&point.y)
        })
    };
    if !WAS_NEAR.swap(near, Relaxed) && !near {
        return;
    }
    let hwnd = HWND(WINDOW.load(Relaxed) as _);
    let now = unsafe { GetTickCount64() };
    if !hwnd.is_invalid() && TINT_UNTIL.swap(now + HOVER_FADE_MS, Relaxed) <= now {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_TRACK_TINT, WPARAM(0), LPARAM(0));
        }
    }
}

/// Only handle clicks on a recently measured part of this exact taskbar.
/// Low-level mouse coordinates are physical, even for a DPI-virtualized hook.
fn shell_hit(point: POINT, matches: impl FnOnce(Placement) -> bool) -> bool {
    if !ENABLED.load(Relaxed) {
        return false;
    }
    let Some(p) = FOUND.try_lock().ok().and_then(|p| *p) else { return false };
    if p.observed.elapsed() >= PLACEMENT_LIFETIME || !matches(p) {
        return false;
    }
    physical(|| unsafe {
        let taskbar = HWND(p.taskbar as _);
        let mut bar = RECT::default();
        let hit = WindowFromPoint(point);
        hit != HWND(WINDOW.load(Relaxed) as _)
            && GetAncestor(hit, GA_ROOT) == taskbar
            && GetWindowRect(taskbar, &mut bar).is_ok()
            && bar == p.bar
    })
}

pub(super) fn native_speaker_at(point: POINT) -> bool {
    shell_hit(point, |p| inside(point, p.tile))
}

pub(super) fn enabled() -> bool {
    ENABLED.load(Relaxed)
}

pub(super) fn reachable() -> bool {
    REACHABLE.load(Relaxed)
}

pub(super) fn on_neighbor(point: POINT) -> bool {
    shell_hit(point, |p| p.neighbors.into_iter().any(|r| inside(point, r)))
}

pub(super) fn rect() -> Option<RECT> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(visible_window()?, &mut r) }.ok()?;
    Some(r)
}

// The front ends and headless callers can have different default DPI contexts.
fn physical<T>(f: impl FnOnce() -> T) -> T {
    let previous = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let result = f();
    unsafe {
        SetThreadDpiAwarenessContext(previous);
    }
    result
}

pub(super) fn anchor() -> Option<crate::Anchor> {
    let hwnd = visible_window()?;
    physical(|| {
        let mut anchor = super::anchor_for(rect()?);
        anchor.scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.;
        Some(anchor)
    })
}

pub(super) fn reset() {
    PAINTED.store(false, Relaxed);
    REACHABLE.store(false, Relaxed);
    *FOUND.lock().unwrap() = None;
    if let Some(hwnd) = visible_window() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    if let Some(wake) = WAKE.get() {
        let _ = wake.send(());
    }
}

fn window(taskbar: HWND) -> Option<HWND> {
    let hwnd = HWND(WINDOW.load(Relaxed) as _);
    if !hwnd.is_invalid() {
        if unsafe { GetParent(hwnd).ok() } == Some(taskbar) {
            return Some(hwnd);
        }
        unsafe { DestroyWindow(hwnd) }.ok()?;
    }
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOPARENTNOTIFY | WS_EX_NOACTIVATE,
            CLASS,
            w!("Audio Bartender volume"),
            WS_CHILD | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            Some(taskbar),
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        // A plain GDI child can receive input while Explorer's composited XAML
        // still paints over it. Give the tile its own opaque composition surface.
        if SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA).is_err() {
            let _ = DestroyWindow(hwnd);
            return None;
        }
        WINDOW.store(hwnd.0 as isize, Relaxed);
        Some(hwnd)
    }
}

/// Cover the glyph with a small opaque tile, leaving both adjacent controls and
/// the rest of the taskbar alone. Reject group-sized rectangles and stale geometry.
fn tile(button: RECT, neighbors: [RECT; 2], dpi: u32) -> Option<RECT> {
    let dip = |v: i32| (v * dpi as i32 + 48) / 96;
    let (w, h) = (button.right - button.left, button.bottom - button.top);
    if !(dip(16)..=dip(48)).contains(&w) || !(dip(16)..=dip(100)).contains(&h) {
        return None;
    }
    // Leave a little space around the speaker. Borrow only measured empty
    // space on both sides so the glyph stays centered over the Windows speaker.
    let [left_neighbor, right_neighbor] = neighbors;
    let gutter = if left_neighbor != RECT::default() && right_neighbor != RECT::default() {
        (button.left - left_neighbor.right).min(right_neighbor.left - button.right).max(0)
    } else {
        0
    };
    let width = dip(30).min(w + 2 * gutter);
    // The standard 48-DIP taskbar leaves 4 DIPs above and below the highlight.
    let height = dip(40).min((h - 2 * dip(4)).max(dip(16)));
    let left = button.left + (w - width).div_euclid(2);
    let top = button.top + (h - height) / 2;
    Some(RECT { left, top, right: left + width, bottom: top + height })
}

fn match_background(hwnd: HWND, p: Placement) {
    use windows::Win32::Graphics::Gdi::{GetDC, GetPixel, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow, ReleaseDC};
    let tile = p.tile;
    let edge = if HOVERED.load(Relaxed) && tile.top > p.button.top {
        // Sample outside the shell's group highlight before applying our own.
        POINT { x: (tile.left + tile.right) / 2, y: tile.top - 1 }
    } else {
        POINT { x: tile.left - 1, y: (tile.top + tile.bottom) / 2 }
    };
    unsafe {
        let hit = WindowFromPoint(edge);
        // A tooltip or Quick Settings can cover the sample without moving the speaker.
        if hit == hwnd || GetAncestor(hit, GA_ROOT) != HWND(p.taskbar as _) {
            return;
        }
        let dc = GetDC(None);
        let color = GetPixel(dc, edge.x, edge.y);
        ReleaseDC(None, dc);
        if color.0 != u32::MAX && BACKGROUND.swap(color.0, Relaxed) != color.0 {
            let _ = RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_UPDATENOW);
        }
    }
}

fn show(found: Placement) -> Option<()> {
    unsafe {
        let taskbar = HWND(found.taskbar as _);
        if FindWindowW(w!("Shell_TrayWnd"), None).ok()? != taskbar || !IsWindowVisible(taskbar).as_bool() {
            return None;
        }
        let mut bar = RECT::default();
        GetWindowRect(taskbar, &mut bar).ok()?;
        if bar != found.bar {
            return None;
        }
        let mut monitor = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(taskbar, MONITOR_DEFAULTTONULL), &mut monitor).as_bool()
            || !contains(monitor.rcMonitor, found.button)
        {
            return None;
        }
        let tile = found.tile;
        let hwnd = window(taskbar)?;
        match_background(hwnd, found);
        let old = rect();
        // A real child moves with Explorer between the desktop and Quick Settings
        // bands. Child coordinates are in the taskbar client area, including RTL.
        if old != Some(tile) || GetTopWindow(Some(taskbar)).ok() != Some(hwnd) {
            let mut corners = [POINT { x: tile.left, y: tile.top }, POINT { x: tile.right, y: tile.bottom }];
            windows::Win32::Graphics::Gdi::MapWindowPoints(None, Some(taskbar), &mut corners);
            SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                corners[0].x.min(corners[1].x),
                corners[0].y,
                tile.right - tile.left,
                tile.bottom - tile.top,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            )
            .ok()?;
        }
        // If Explorer stops answering UI Automation, retire the stale hit target.
        SetTimer(Some(hwnd), TIMER_PLACEMENT, PLACEMENT_LIFETIME.as_millis() as u32, None);
        Some(())
    }
}

/// Report transitions from a clickable overlay to the shell fallback and back.
pub(super) fn place() -> bool {
    let before = visible_window();
    let found = *FOUND.lock().unwrap();
    if !(ENABLED.load(Relaxed) && found.and_then(|p| physical(|| show(p))).is_some())
        && let Some(hwnd) = before
    {
        PAINTED.store(false, Relaxed);
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    let reachable = PAINTED.load(Relaxed)
        && visible_window().is_some_and(|hwnd| {
            physical(|| {
                let Some(tile) = rect() else { return false };
                let center = POINT { x: (tile.left + tile.right) / 2, y: (tile.top + tile.bottom) / 2 };
                unsafe {
                    let hit = WindowFromPoint(center);
                    if hit == hwnd {
                        return true;
                    }
                    let Some(p) = found else { return false };
                    let taskbar = HWND(p.taskbar as _);
                    if !reachable() || before != Some(hwnd) || hit.is_invalid() || GetAncestor(hit, GA_ROOT) == taskbar
                    {
                        return false;
                    }
                    // A top-level window can legitimately cover the taskbar (e.g.
                    // the input switcher's click-away surface). Keep the existing
                    // placement only if the tile still wins the taskbar's own hit test.
                    let mut point = [center];
                    windows::Win32::Graphics::Gdi::MapWindowPoints(None, Some(taskbar), &mut point);
                    ChildWindowFromPointEx(
                        taskbar,
                        point[0],
                        CWP_SKIPINVISIBLE | CWP_SKIPDISABLED | CWP_SKIPTRANSPARENT,
                    ) == hwnd
                }
            })
        });
    REACHABLE.swap(reachable, Relaxed) != reachable
}

pub(super) fn icon_size(hwnd: HWND) -> i32 {
    (unsafe { GetDpiForWindow(hwnd) } * 16 / 96).max(1) as i32
}

// Blend two 0xRRGGBB colors in sRGB, as Explorer does on the taskbar backdrop.
fn blend(base: u32, ink: u32, opacity: f32) -> u32 {
    let channel = |shift: u32| {
        let a = ((base >> shift) & 255) as f32;
        let b = ((ink >> shift) & 255) as f32;
        (a + (b - a) * opacity).round() as u32
    };
    channel(16) << 16 | channel(8) << 8 | channel(0)
}

fn paint_hover(dc: HDC, r: RECT, scale: f32, background: COLORREF) {
    let (width, height) = (r.right, r.bottom);
    if width <= 0 || height <= 0 {
        return;
    }
    let base = background.0.swap_bytes() >> 8; // COLORREF to a 32-bit DIB pixel.
    let mut contrast = HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    let high_contrast = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            Some((&raw mut contrast).cast()),
            Default::default(),
        )
        .is_ok()
            && contrast.dwFlags.contains(HCF_HIGHCONTRASTON)
    };
    // SystemTrayResources.xbf: OmniButtonBackgroundPointerOver uses the secondary
    // taskbar fill, with a one-DIP secondary gradient stroke over it.
    let (fill, stroke_top, stroke_bottom) = if high_contrast {
        let highlight = unsafe { GetSysColor(COLOR_HIGHLIGHT) }.swap_bytes() >> 8;
        (base, highlight, highlight)
    } else {
        let light = crate::tray_icon::light_taskbar();
        let fill = blend(base, 0xffffff, if light { 128. } else { 15. } / 255.);
        let ink = if light { 0 } else { 0xffffff };
        let (top, bottom) = if light { (5., 15.) } else { (15., 26.) };
        (fill, blend(fill, ink, top / 255.), blend(fill, ink, bottom / 255.))
    };
    let radius = (4. * scale).min(width.min(height) as f32 / 2.);
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let edge_y = (y.min(height - 1 - y) as f32) + 0.5;
        let stroke = blend(stroke_bottom, stroke_top, ((height as f32 - y as f32 - 0.5) / (3. * scale)).clamp(0., 1.));
        for x in 0..width {
            let edge_x = (x.min(width - 1 - x) as f32) + 0.5;
            // Distance inside the rounded rectangle, measured at the pixel center.
            let (dx, dy) = (radius - edge_x, radius - edge_y);
            let depth = radius - dx.max(0.).hypot(dy.max(0.)) - dx.max(dy).min(0.);
            let edge = blend(base, stroke, (depth + 0.5).clamp(0., 1.));
            pixels.push(blend(edge, fill, (depth - scale + 0.5).clamp(0., 1.)));
        }
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        SetDIBitsToDevice(
            dc,
            0,
            0,
            width as u32,
            height as u32,
            0,
            0,
            0,
            height as u32,
            pixels.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
        );
    }
}

fn set_hover(hwnd: HWND, hovered: bool) {
    if HOVERED.swap(hovered, Relaxed) != hovered {
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    static CLICKED: AtomicBool = AtomicBool::new(false);
    let event = match msg {
        WM_NCDESTROY => {
            HOVERED.store(false, Relaxed);
            TINT_UNTIL.store(0, Relaxed);
            if WINDOW.compare_exchange(hwnd.0 as isize, 0, Relaxed, Relaxed).is_ok() {
                post(WM_SPEAKER_SLOT, 0);
            }
            PAINTED.store(false, Relaxed);
            CLICKED.store(false, Relaxed);
            return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
        }
        WM_SHOWWINDOW if wp.0 == 0 => {
            set_hover(hwnd, false);
            return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
        }
        WM_TRACK_TINT => {
            unsafe { SetTimer(Some(hwnd), TIMER_HOVER_TINT, USER_TIMER_MINIMUM, None) };
            return LRESULT(0);
        }
        WM_TIMER if wp.0 == TIMER_HOVER_TINT => {
            let found = *FOUND.lock().unwrap();
            if let Some(p) = found
                && unsafe { IsWindowVisible(hwnd).as_bool() }
            {
                physical(|| match_background(hwnd, p));
            }
            if unsafe { GetTickCount64() } >= TINT_UNTIL.load(Relaxed) {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_TINT);
                }
            }
            return LRESULT(0);
        }
        WM_TIMER if wp.0 == TIMER_PLACEMENT => {
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_PLACEMENT);
            }
            *FOUND.lock().unwrap() = None;
            post(WM_SPEAKER_SLOT, 0);
            return LRESULT(0);
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe {
                let dc = BeginPaint(hwnd, &mut ps);
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                let background = COLORREF(BACKGROUND.load(Relaxed));
                if HOVERED.load(Relaxed) {
                    paint_hover(dc, r, GetDpiForWindow(hwnd) as f32 / 96., background);
                } else {
                    let brush = CreateSolidBrush(background);
                    FillRect(dc, &r, brush);
                    let _ = DeleteObject(brush.into());
                }
                let icon = HICON(CURRENT_ICON.load(Relaxed) as _);
                let mut painted = false;
                if !icon.is_invalid() {
                    let size = icon_size(hwnd).min(r.right).min(r.bottom);
                    painted = DrawIconEx(
                        dc,
                        (r.right - size) / 2,
                        (r.bottom - size) / 2,
                        icon,
                        size,
                        size,
                        0,
                        None,
                        DI_NORMAL,
                    )
                    .is_ok();
                }
                let finished = EndPaint(hwnd, &ps).as_bool() && painted;
                if PAINTED.swap(finished, Relaxed) != finished {
                    post(WM_SPEAKER_SLOT, 0);
                }
            }
            return LRESULT(0);
        }
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            set_hover(hwnd, true);
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: if CLICKED.load(Relaxed) { TME_LEAVE } else { TME_HOVER | TME_LEAVE },
                hwndTrack: hwnd,
                dwHoverTime: HOVER_DEFAULT,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut track);
            }
            WM_MOUSEMOVE
        }
        WM_MOUSEHOVER if CLICKED.load(Relaxed) => return LRESULT(0),
        WM_MOUSEHOVER => NIN_POPUPOPEN,
        WM_MOUSELEAVE => {
            set_hover(hwnd, false);
            // Showing the flyout can cause a synthetic leave under a stationary mouse.
            let mut pt = POINT::default();
            if unsafe { GetCursorPos(&mut pt) }.is_ok()
                && !rect().is_some_and(|r| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom)
            {
                CLICKED.store(false, Relaxed);
            }
            NIN_POPUPCLOSE
        }
        WM_LBUTTONUP => {
            CLICKED.store(true, Relaxed);
            msg
        }
        WM_MBUTTONUP => msg,
        WM_RBUTTONUP => WM_CONTEXTMENU,
        WM_MOUSEWHEEL => {
            post(WM_WHEEL, (wp.0 >> 16) & 0xffff);
            return LRESULT(0);
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    };
    post_with(WM_TRAY, 0, event as isize);
    LRESULT(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_hit_stops_at_the_speaker_edges() {
        let button = RECT { left: -225, top: 1128, right: -189, bottom: 1200 };
        let neighbors = [RECT { left: -267, right: -231, ..button }, RECT { left: -183, right: -141, ..button }];
        let r = tile(button, neighbors, 144).unwrap();
        assert_eq!(r, RECT { left: -230, top: 1134, right: -185, bottom: 1194 });
        assert!(inside(POINT { x: -207, y: 1164 }, r));
        for (x, y) in [(-231, 1164), (-185, 1164), (-207, 1133), (-207, 1194)] {
            assert!(!inside(POINT { x, y }, r));
        }
    }

    #[test]
    fn exact_localized_role_and_individual_bounds() {
        assert!(named_speaker("Volume Speakers: Muted", "Volume"));
        assert!(named_speaker("Lautstärke Lautsprecher", "Lautstärke"));
        assert!(!named_speaker("Network Volume", "Volume"));
        assert!(!named_speaker("VolumeMixer", "Volume"));
        let button = RECT { left: -225, top: 1128, right: -189, bottom: 1200 };
        let unknown = [RECT::default(); 2];
        assert_eq!(tile(button, unknown, 144), Some(RECT { left: -225, top: 1134, right: -189, bottom: 1194 }));
        assert_eq!(tile(RECT { right: -60, ..button }, unknown, 144), None);
    }
}
