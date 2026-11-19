use std::path::PathBuf;

use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("MediaKeysSelector");

/// The remembered target, stored in `%APPDATA%\MediaKeysSelector\config.ini`.
pub struct Config {
    pub target: Option<String>,
    /// Display name of `target`, shown while its app is closed.
    pub target_name: Option<String>,
    pub dark: Option<bool>,
    pub custom_title_bar: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config { target: None, target_name: None, dark: None, custom_title_bar: true }
    }
}

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("MediaKeysSelector").join("config.ini"))
}

impl Config {
    pub fn load() -> Config {
        let mut cfg = Config::default();
        let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return cfg };
        for line in text.lines() {
            match line.split_once('=') {
                Some(("target", v)) if !v.is_empty() => cfg.target = Some(v.to_owned()),
                Some(("target_name", v)) if !v.is_empty() => cfg.target_name = Some(v.to_owned()),
                Some(("theme", "dark")) => cfg.dark = Some(true),
                Some(("theme", "light")) => cfg.dark = Some(false),
                Some(("titlebar", "native")) => cfg.custom_title_bar = false,
                _ => {}
            }
        }
        cfg
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let text = format!(
            "target={}\ntarget_name={}\ntheme={}\ntitlebar={}\n",
            self.target.as_deref().unwrap_or(""),
            self.target_name.as_deref().unwrap_or(""),
            match self.dark {
                Some(true) => "dark",
                Some(false) => "light",
                None => "",
            },
            if self.custom_title_bar { "custom" } else { "native" }
        );
        let _ = std::fs::write(path, text);
    }
}

fn run_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" --hidden", exe.display()))
}

/// True when the Run key launches this exe.
pub fn autostart() -> bool {
    let Some(expected) = run_command() else { return false };
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let found = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
    };
    found.is_ok() && crate::util::from_wide(&buf).eq_ignore_ascii_case(&expected)
}

pub fn set_autostart(on: bool) {
    unsafe {
        if !on {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
            return;
        }
        let Some(cmd) = run_command() else { return };
        let data = crate::util::wide(&cmd);
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        );
    }
}
