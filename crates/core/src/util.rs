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

/// Appends a line to `%TEMP%\mediakeys.log` when `MEDIAKEYS_LOG` is set.
pub fn log(line: impl AsRef<str>) {
    use std::io::Write;
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var_os("MEDIAKEYS_LOG").is_some()) {
        return;
    }
    let path = std::env::temp_dir().join("mediakeys.log");
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() % 100_000).unwrap_or(0);
        let _ = writeln!(file, "{ms:>5} pid {} {}", std::process::id(), line.as_ref());
    }
}

/// Joins COM on the calling thread if it hasn't yet. A thread that already chose an
/// apartment keeps it, which is fine for the audio APIs.
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
        ShellExecuteW(None, windows::core::w!("open"), windows::core::w!("explorer.exe"), windows::core::PCWSTR(target.as_ptr()), None, SW_SHOWNORMAL);
    }
}

/// Moves and resizes a window so its client area is `width` x `height` logical pixels,
/// placed by `anchor`. Keeps the flyout against the taskbar as its content grows.
pub fn place_window(hwnd: isize, anchor: &crate::Anchor, width: f32, height: f32) {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
    };
    let hwnd = HWND(hwnd as _);
    let (w, h) = ((width * anchor.scale).round() as i32, (height * anchor.scale).round() as i32);
    let (x, y) = anchor.place(w, h);
    unsafe {
        // The window rectangle can include invisible resize borders around the client area.
        let (mut window, mut client, mut origin) = (RECT::default(), RECT::default(), POINT::default());
        if GetWindowRect(hwnd, &mut window).is_err() || GetClientRect(hwnd, &mut client).is_err() {
            return;
        }
        let _ = ClientToScreen(hwnd, &mut origin);
        let (dx, dy) = (origin.x - window.left, origin.y - window.top);
        let extra_w = (window.right - window.left) - client.right;
        let extra_h = (window.bottom - window.top) - client.bottom;
        let _ = SetWindowPos(hwnd, None, x - dx, y - dy, w + extra_w, h + extra_h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
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
