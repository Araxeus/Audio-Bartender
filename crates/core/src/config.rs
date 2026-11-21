use std::collections::BTreeMap;
use std::path::PathBuf;

use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_BINARY, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

use crate::{
    Accent, DeviceChip, LevelMeters, MediaButtons, MediaControls, Settings, TooltipStyle, TrayStyle, VolumeBars,
};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
/// Where Task Manager keeps its Startup tab switch for each Run entry.
const APPROVED_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");

/// The app's name before it became Audio Bartender, for moving what it kept.
const OLD_NAME: &str = "MediaKeysSelector";
const OLD_RUN_VALUE: PCWSTR = w!("MediaKeysSelector");

/// The remembered target and settings, stored in `%APPDATA%\AudioBartender\config.ini`.
/// Autostart lives in the registry instead, so `settings.autostart` is ignored here.
#[derive(Default)]
pub struct Config {
    pub target: Option<String>,
    /// Display name of `target`, shown while its app is closed.
    pub target_name: Option<String>,
    /// When the last update check finished, in ms since the epoch.
    pub last_update_check: u64,
    pub settings: Settings,
    /// Output device id by app id, for apps sent away from the default device.
    pub routes: BTreeMap<String, String>,
}

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("AudioBartender").join("config.ini"))
}

fn flag(value: &str) -> bool {
    value == "true"
}

impl Config {
    pub fn load() -> Config {
        migrate();
        let mut cfg = Config::default();
        cfg.settings.auto_update = crate::autostart::installed();
        let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return cfg };
        let s = &mut cfg.settings;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            if let Some(app) = key.strip_prefix("route.") {
                if !app.is_empty() && !value.is_empty() {
                    cfg.routes.insert(app.to_owned(), value.to_owned());
                }
                continue;
            }
            match key {
                "target" if !value.is_empty() => cfg.target = Some(value.to_owned()),
                "target_name" if !value.is_empty() => cfg.target_name = Some(value.to_owned()),
                "theme" => s.dark = matches!(value, "dark" | "light").then(|| value == "dark"),
                "album_art" => s.album_art = flag(value),
                "speaker_position" => s.speaker_position = flag(value),
                "tray_style" => s.tray_style = if value == "number" { TrayStyle::Number } else { TrayStyle::Ring },
                "level_meters" => s.level_meters = LevelMeters::from_name(value).unwrap_or_default(),
                "device_chip" => s.device_chip = DeviceChip::from_name(value).unwrap_or_default(),
                "media_controls" => s.media_controls = MediaControls::from_name(value).unwrap_or_default(),
                "media_buttons" => s.media_buttons = MediaButtons::from_name(value).unwrap_or_default(),
                "auto_update" => s.auto_update = flag(value),
                "last_update_check" => cfg.last_update_check = value.parse().unwrap_or(0),
                "accent" => s.accent = Accent::from_name(value).unwrap_or_default(),
                "tooltip" => {
                    s.tooltip = match value {
                        "compact" => TooltipStyle::Compact,
                        "text" => TooltipStyle::Text,
                        _ => TooltipStyle::NowPlaying,
                    }
                }
                "volume_bars" => {
                    s.volume_bars = match value {
                        "none" => VolumeBars::None,
                        "always" => VolumeBars::Always,
                        "hover" | "click" => VolumeBars::OnHover,
                        _ => VolumeBars::Compact,
                    }
                }
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
            "target={}\ntarget_name={}\ntheme={theme}\nalbum_art={}\nspeaker_position={}\ntray_style={}\nvolume_bars={}\ntooltip={}\nmedia_buttons={}\nmedia_controls={}\nlevel_meters={}\ndevice_chip={}\nauto_update={}\nlast_update_check={}\naccent={}\n",
            self.target.as_deref().unwrap_or(""),
            self.target_name.as_deref().unwrap_or(""),
            s.album_art,
            s.speaker_position,
            if s.tray_style == TrayStyle::Number { "number" } else { "ring" },
            match s.volume_bars {
                VolumeBars::None => "none",
                VolumeBars::Compact => "compact",
                VolumeBars::Always => "always",
                VolumeBars::OnHover => "hover",
            },
            match s.tooltip {
                TooltipStyle::NowPlaying => "now_playing",
                TooltipStyle::Compact => "compact",
                TooltipStyle::Text => "text",
            },
            s.media_buttons.name(),
            s.media_controls.name(),
            s.level_meters.name(),
            s.device_chip.name(),
            s.auto_update,
            self.last_update_check,
            s.accent.name(),
        );
        let routes: String = self.routes.iter().map(|(app, device)| format!("route.{app}={device}\n")).collect();
        let _ = std::fs::write(path, text + &routes);
    }
}

/// Moves the settings folder and the Run entry from the old name. The old Run entry
/// pointed at an exe that the rename replaced, so the new one points at this exe.
/// Task Manager's switch for it carries over, so a disabled entry stays disabled.
fn migrate() {
    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        let (old, new) = (appdata.join(OLD_NAME), appdata.join("AudioBartender"));
        if old.exists() && !new.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
    unsafe {
        let mut buf = [0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let found = RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            OLD_RUN_VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        );
        if found.is_err() {
            return;
        }
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, OLD_RUN_VALUE);
        crate::autostart::set(true);
        let mut approved = [0u8; 16];
        let mut size = approved.len() as u32;
        let found = RegGetValueW(
            HKEY_CURRENT_USER,
            APPROVED_KEY,
            OLD_RUN_VALUE,
            RRF_RT_REG_BINARY,
            None,
            Some(approved.as_mut_ptr().cast()),
            Some(&mut size),
        );
        if found.is_ok() {
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                APPROVED_KEY,
                crate::autostart::RUN_VALUE,
                REG_BINARY.0,
                Some(approved.as_ptr().cast()),
                size,
            );
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED_KEY, OLD_RUN_VALUE);
        }
    }
}
