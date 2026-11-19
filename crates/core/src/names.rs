use std::collections::HashMap;
use std::ffi::c_void;

use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::core::PCWSTR;

use crate::util::{Process, process_path, wide};

/// AppUserModelIDs that don't name their app.
const KNOWN: &[(&str, &str)] = &[
    ("308046B0AF4A39CB", "Firefox"),
    ("6F193CCC56814779", "Firefox Nightly"),
    ("E7CF176E110C211B", "Firefox Developer Edition"),
    ("MSEdge", "Microsoft Edge"),
    ("Chrome", "Google Chrome"),
];

/// Resolves friendly app names and caches them for the life of the process.
#[derive(Default)]
pub struct Names {
    cache: HashMap<String, String>,
}

impl Names {
    /// Friendly name for an SMTC AppUserModelID.
    pub fn for_aumid(&mut self, aumid: &str, procs: &[Process]) -> String {
        if let Some(name) = self.cache.get(aumid) {
            return name.clone();
        }
        let name = resolve_aumid(aumid, procs);
        // Exe-based names depend on the process running, so only cache successes.
        if !name.to_lowercase().ends_with(".exe") {
            self.cache.insert(aumid.to_owned(), name.clone());
        }
        name
    }

    /// Friendly name for a running exe, e.g. `vlc.exe` -> "VLC media player".
    pub fn for_exe(&mut self, exe: &str, procs: &[Process]) -> String {
        // Toolhelp's name for PID 0, which owns the system sounds session.
        if exe == "[system process]" {
            return "System sounds".to_owned();
        }
        if let Some(name) = self.cache.get(exe) {
            return name.clone();
        }
        let name = describe_exe(exe, procs).unwrap_or_else(|| exe.trim_end_matches(".exe").to_owned());
        self.cache.insert(exe.to_owned(), name.clone());
        name
    }
}

fn resolve_aumid(aumid: &str, procs: &[Process]) -> String {
    if let Some((_, name)) = KNOWN.iter().find(|(id, _)| id.eq_ignore_ascii_case(aumid)) {
        return (*name).to_owned();
    }
    let lower = aumid.to_lowercase();
    if lower.ends_with(".exe") {
        return describe_exe(&lower, procs).unwrap_or_else(|| aumid[..aumid.len() - 4].to_owned());
    }

    // Packaged apps look like `SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`.
    let app = aumid.rsplit('!').next().unwrap_or(aumid);
    let app = app.rsplit('.').next().unwrap_or(app);
    match app {
        "ZuneMusic" => "Media Player".to_owned(),
        "ZuneVideo" => "Movies & TV".to_owned(),
        _ => app.to_owned(),
    }
}

/// The FileDescription of a running exe's version resource.
fn describe_exe(exe: &str, procs: &[Process]) -> Option<String> {
    let pid = procs.iter().find(|p| p.exe == exe)?.pid;
    let path = wide(&process_path(pid)?);
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(path.as_ptr()), None, size, data.as_mut_ptr().cast()).ok()?;

        let mut ptr: *mut c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let query = wide("\\VarFileInfo\\Translation");
        if !VerQueryValueW(data.as_ptr().cast(), PCWSTR(query.as_ptr()), &mut ptr, &mut len).as_bool() || len < 4 {
            return None;
        }
        let lang = *(ptr as *const u16);
        let codepage = *(ptr as *const u16).add(1);

        let query = wide(&format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\FileDescription"));
        if !VerQueryValueW(data.as_ptr().cast(), PCWSTR(query.as_ptr()), &mut ptr, &mut len).as_bool() || len == 0 {
            return None;
        }
        let text = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        let name = crate::util::from_wide(text).trim().to_owned();
        (!name.is_empty()).then_some(name)
    }
}
