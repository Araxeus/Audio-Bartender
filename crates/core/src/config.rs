use std::path::PathBuf;

use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

use crate::{Settings, TrayStyle};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("MediaKeysSelector");

/// The remembered target and settings, stored in `%APPDATA%\MediaKeysSelector\config.ini`.
/// Autostart lives in the registry instead, so `settings.autostart` is ignored here.
#[derive(Default)]
pub struct Config {
    pub target: Option<String>,
    /// Display name of `target`, shown while its app is closed.
    pub target_name: Option<String>,
    pub settings: Settings,
}

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("MediaKeysSelector").join("config.ini"))
}

fn flag(value: &str) -> bool {
    value == "true"
}

impl Config {
    pub fn load() -> Config {
        let mut cfg = Config::default();
        let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return cfg };
        let s = &mut cfg.settings;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match key {
                "target" if !value.is_empty() => cfg.target = Some(value.to_owned()),
                "target_name" if !value.is_empty() => cfg.target_name = Some(value.to_owned()),
                "theme" => s.dark = matches!(value, "dark" | "light").then(|| value == "dark"),
                "album_art" => s.album_art = flag(value),
                "tray_style" => s.tray_style = if value == "number" { TrayStyle::Number } else { TrayStyle::Ring },
                _ => {}
            }
        }
        cfg
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let s = &self.settings;
        let theme = match s.dark {
            Some(true) => "dark",
            Some(false) => "light",
            None => "",
        };
        let text = format!(
            "target={}\ntarget_name={}\ntheme={theme}\nalbum_art={}\ntray_style={}\n",
            self.target.as_deref().unwrap_or(""),
            self.target_name.as_deref().unwrap_or(""),
            s.album_art,
            if s.tray_style == TrayStyle::Number { "number" } else { "ring" },
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
