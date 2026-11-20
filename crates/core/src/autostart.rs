//! Start with Windows, and running elevated when installed.
//!
//! A portable copy only adds or removes a Run entry for its own exe. An installed copy
//! (setup put its uninstaller beside the exe) does the same, but an all-users install
//! has also registered a scheduled task that runs it elevated, so the keyboard hook
//! sees keys typed while an admin window has focus. An unelevated launch of an
//! installed copy hands off to that task. The app never installs anything itself.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_DWORD, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW,
    RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{PCWSTR, w};

use crate::util::{from_wide, wide};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
/// Where Task Manager keeps its Startup tab switch for each Run entry.
const APPROVED_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
pub(crate) const RUN_VALUE: PCWSTR = w!("AudioBartender");
/// Holds whether an elevated launch should open the flyout.
const APP_KEY: PCWSTR = w!("Software\\AudioBartender");
const OPEN_VALUE: PCWSTR = w!("OpenFlyout");

/// The scheduled task setup registers for all-users installs.
const TASK: &str = "AudioBartender";
/// Marks a launch by that task.
const TASK_ARG: &str = "--task";
/// Lets setup turn starting with Windows on or off as the signed-in user.
const SET_ARG: &str = "--set-autostart=";
/// Allows several copies at once and never hands off, for comparing front ends.
const DEV_ARG: &str = "--dev";

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn has_arg(arg: &str) -> bool {
    std::env::args().any(|a| a == arg)
}

/// True when launched with `--dev`.
pub fn dev() -> bool {
    has_arg(DEV_ARG)
}

/// True when setup installed this copy: Inno Setup puts its uninstaller beside the exe.
pub fn installed() -> bool {
    std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("unins000.exe"))).is_some_and(|u| u.exists())
}

/// True when the Run entry exists and Task Manager hasn't disabled it.
pub fn enabled() -> bool {
    read_string(RUN_KEY, RUN_VALUE).is_some() && !disabled_in_task_manager()
}

/// Adds or removes the Run entry for this exe. Turning it on also clears Task
/// Manager's disable mark, which would otherwise keep the entry from running.
pub fn set(on: bool) {
    if on {
        let Ok(exe) = std::env::current_exe() else { return };
        write_string(RUN_KEY, RUN_VALUE, &format!("\"{}\" --hidden", exe.display()));
    } else {
        delete_value(RUN_KEY, RUN_VALUE);
    }
    delete_value(APPROVED_KEY, RUN_VALUE);
}

/// Runs `--set-autostart=on|off` for setup, then exits. Call before anything else starts.
pub fn handle_helper_args() {
    let Some(value) = std::env::args().find_map(|a| a.strip_prefix(SET_ARG).map(str::to_owned)) else { return };
    set(value == "on");
    std::process::exit(0);
}

/// Hands an unelevated launch of an installed copy to the elevated task. Returns
/// true when the task took over and this process should quit.
pub fn hand_off() -> bool {
    if dev() || has_arg(TASK_ARG) || !installed() || is_elevated() {
        return false;
    }
    // The task can't pass arguments, so a launch from the Start menu leaves a note to
    // open the flyout.
    write_dword(APP_KEY, OPEN_VALUE, u32::from(!has_arg("--hidden")));
    let started = Command::new("schtasks.exe")
        .args(["/run", "/tn", TASK])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success());
    if !started {
        delete_value(APP_KEY, OPEN_VALUE);
    }
    crate::util::log(format!("hand off to elevated task: {started}"));
    started
}

/// For a launch by the task: whether the launch it replaced asked for the flyout.
/// Reads the note once and removes it.
pub fn take_open_request() -> bool {
    let open = read_dword(APP_KEY, OPEN_VALUE) == Some(1);
    delete_value(APP_KEY, OPEN_VALUE);
    open
}

/// True when the task started this copy.
pub fn launched_by_task() -> bool {
    has_arg(TASK_ARG)
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Task Manager marks a disabled entry with an odd first byte.
fn disabled_in_task_manager() -> bool {
    let mut data = [0u8; 16];
    let mut size = data.len() as u32;
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            APPROVED_KEY,
            RUN_VALUE,
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    found.is_ok() && size > 0 && data[0] & 1 == 1
}

fn read_string(key: PCWSTR, value: PCWSTR) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let found = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key, value, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
    };
    found.is_ok().then(|| from_wide(&buf))
}

fn write_string(key: PCWSTR, value: PCWSTR, text: &str) {
    let data = wide(text);
    unsafe {
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        );
    }
}

fn read_dword(key: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = 4u32;
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as _),
            Some(&mut size),
        )
    };
    found.is_ok().then_some(data)
}

fn write_dword(key: PCWSTR, value: PCWSTR, data: u32) {
    unsafe {
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, key, value, REG_DWORD.0, Some(&data as *const u32 as _), 4);
    }
}

pub(crate) fn delete_value(key: PCWSTR, value: PCWSTR) {
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, key, value);
    }
}
