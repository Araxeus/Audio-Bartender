use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::{BOOL, PWSTR};

/// Appends a line to `%TEMP%\audio_bartender.log` when `AUDIO_BARTENDER_LOG` is set.
pub fn log(line: impl AsRef<str>) {
    use std::io::Write;
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var_os("AUDIO_BARTENDER_LOG").is_some()) {
        return;
    }
    let path = std::env::temp_dir().join("audio_bartender.log");
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() % 100_000)
            .unwrap_or(0);
        let _ = writeln!(file, "{ms:>5} pid {} {}", std::process::id(), line.as_ref());
    }
}

/// Joins COM on the calling thread if it hasn't yet. A thread that already chose an
/// apartment keeps it, which is fine for the audio APIs.
/// Milliseconds since the Unix epoch.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn com_init() {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

/// Opens the classic Notification Area Icons control panel, or its System Icons page.
pub fn open_icon_page(page: crate::IconPage) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    const PANEL: &str = "shell:::{05d7b0f4-2121-4eff-bf6b-ed3f69b894d9}";
    let target = match page {
        crate::IconPage::ShowOurs => PANEL.to_owned(),
        crate::IconPage::HideWindows => format!("{PANEL}\\SystemIcons"),
    };
    let target = wide(&target);
    unsafe {
        ShellExecuteW(
            None,
            windows::core::w!("open"),
            windows::core::w!("explorer.exe"),
            windows::core::PCWSTR(target.as_ptr()),
            None,
            SW_SHOWNORMAL,
        );
    }
}

/// Moves a window, keeping its size, so its visible frame sits where `anchor` places
/// a flyout of that size, `progress` of the way through its slide. While sliding,
/// the window stays just below the taskbar so it appears to come out from behind it.
/// Where the client area starts inside the window rectangle. A window region
/// doesn't change it, unlike the visible frame DWM reports.
fn client_offset(hwnd: HWND) -> (i32, i32) {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    unsafe {
        let (mut window, mut client) = (RECT::default(), POINT::default());
        if GetWindowRect(hwnd, &mut window).is_err() || !ClientToScreen(hwnd, &mut client).as_bool() {
            return (0, 0);
        }
        (client.x - window.left, client.y - window.top)
    }
}

pub fn place_window(hwnd: isize, anchor: &crate::Anchor, frame: crate::Frame, progress: f32) {
    let hwnd = HWND(hwnd as _);
    position(hwnd, anchor, frame, client_offset(hwnd), progress);
}

pub fn set_window_region(hwnd: isize, parts: &[(i32, i32, i32, i32)]) {
    use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, RGN_OR, SetWindowRgn};
    let hwnd = HWND(hwnd as _);
    let (dx, dy) = client_offset(hwnd);
    unsafe {
        let region = CreateRectRgn(0, 0, 0, 0);
        for &(l, t, r, b) in parts {
            let part = CreateRectRgn(l + dx, t + dy, r + dx, b + dy);
            CombineRgn(Some(region), Some(region), Some(part), RGN_OR);
            let _ = DeleteObject(part.into());
        }
        // The window owns the region from here on.
        SetWindowRgn(hwnd, Some(region), true);
    }
}

/// Puts a window whose client area is `frame`, offset by `border` inside it, where
/// `anchor` places it `progress` of the way through its slide.
fn position(hwnd: HWND, anchor: &crate::Anchor, frame: crate::Frame, border: (i32, i32), progress: f32) {
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    };
    let (x, y) = anchor.place_sliding(frame, progress);
    let (dx, dy) = border;
    unsafe {
        let taskbar = if progress < 1.0 { FindWindowW(windows::core::w!("Shell_TrayWnd"), None).ok() } else { None };
        let mut flags: SET_WINDOW_POS_FLAGS = SWP_NOSIZE | SWP_NOACTIVATE;
        if taskbar.is_none() {
            flags |= SWP_NOZORDER;
        }
        let _ = SetWindowPos(hwnd, taskbar, x - dx, y - dy, 0, 0, flags);
    }
}

/// True on Windows 11, whose build numbers start at 22000.
pub fn windows_11() -> bool {
    use std::sync::OnceLock;
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    static WINDOWS_11: OnceLock<bool> = OnceLock::new();
    *WINDOWS_11.get_or_init(|| {
        let mut buf = [0u16; 32];
        let mut size = (buf.len() * 2) as u32;
        let found = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                windows::core::w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
                windows::core::w!("CurrentBuildNumber"),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        found.is_ok() && from_wide(&buf).parse::<u32>().is_ok_and(|build| build >= 22000)
    })
}

/// Moves this app's tray icon out of the overflow on Windows 11. Explorer keeps each
/// icon's place in a key under `NotifyIconSettings`, named for nothing we know, so this
/// finds ours by its executable and sets `IsPromoted`, as dragging the icon does.
/// Returns false when no key names this executable yet.
pub fn promote_tray_icon() -> bool {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW,
        RegOpenKeyExW, RegSetKeyValueW,
    };
    const ROOT: &str = "Control Panel\\NotifyIconSettings";
    let Some(exe) =
        std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
    else {
        return false;
    };
    let root = wide(ROOT);
    let mut promoted = false;
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, windows::core::PCWSTR(root.as_ptr()), None, KEY_READ, &mut key).is_err() {
            return false;
        }
        for index in 0.. {
            let mut name = [0u16; 64];
            let mut len = name.len() as u32;
            let listed = RegEnumKeyExW(
                key,
                index,
                Some(windows::core::PWSTR(name.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            );
            if listed.is_err() {
                break;
            }
            let sub = wide(&format!("{ROOT}\\{}", from_wide(&name)));
            let mut path = [0u16; 1024];
            let mut size = (path.len() * 2) as u32;
            let found = RegGetValueW(
                HKEY_CURRENT_USER,
                windows::core::PCWSTR(sub.as_ptr()),
                windows::core::w!("ExecutablePath"),
                RRF_RT_REG_SZ,
                None,
                Some(path.as_mut_ptr().cast()),
                Some(&mut size),
            );
            // The path may start with a known folder's GUID, so match the file name.
            let path = from_wide(&path).to_lowercase();
            if found.is_ok() && path.rsplit('\\').next() == Some(exe.as_str()) {
                let one = 1u32;
                let set = RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    windows::core::PCWSTR(sub.as_ptr()),
                    windows::core::w!("IsPromoted"),
                    REG_DWORD.0,
                    Some((&one as *const u32).cast()),
                    4,
                );
                promoted |= set.is_ok();
            }
        }
        let _ = RegCloseKey(key);
    }
    crate::log(format!("tray icon promoted: {promoted}"));
    promoted
}

/// Puts a topmost window above the other topmost windows, such as a flyout that
/// came to the front after it.
pub(crate) fn raise_topmost(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos};
    unsafe {
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Hides or shows a window without changing its visibility state.
pub fn set_cloaked(hwnd: isize, cloaked: bool) {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
    let value = windows::core::BOOL::from(cloaked);
    unsafe {
        let _ =
            DwmSetWindowAttribute(HWND(hwnd as _), DWMWA_CLOAK, &value as *const _ as _, size_of_val(&value) as u32);
    }
}

/// Whether "Show animations in Windows" is on.
pub fn animations_enabled() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut on = windows::core::BOOL(1);
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut _ as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    on.as_bool()
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Copies `s` into a fixed UTF-16 buffer, truncating and NUL-terminating it.
pub fn fill_wide(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for c in s.encode_utf16().take(dst.len() - 1) {
        dst[n] = c;
        n += 1;
    }
    dst[n] = 0;
}

pub struct Process {
    pub pid: u32,
    /// Lowercased file name, e.g. `vlc.exe`.
    pub exe: String,
}

pub fn processes() -> Vec<Process> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            out.push(Process { pid: entry.th32ProcessID, exe: from_wide(&entry.szExeFile).to_lowercase() });
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        ok.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// The first visible, titled, unowned top-level window of `pid`.
pub fn main_window(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        let candidate = pid == search.pid
            && unsafe { IsWindowVisible(hwnd) }.as_bool()
            && unsafe { GetWindowTextLengthW(hwnd) } > 0
            && unsafe { GetWindow(hwnd, GW_OWNER) }.is_err();
        if candidate {
            search.found = Some(hwnd);
            return false.into();
        }
        true.into()
    }

    let mut search = Search { pid, found: None };
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize));
    }
    search.found
}
