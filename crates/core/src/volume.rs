//! Per-app volume through the Windows audio session API (the same sessions the
//! Volume Mixer shows).

use std::collections::HashMap;

use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, DEVICE_STATE_ACTIVE, IAudioSessionControl2,
    IAudioSessionManager2, ISimpleAudioVolume, eRender,
};
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::core::Interface;

use crate::util::{Process, processes};

pub struct Session {
    /// Lowercased exe name of the process that owns the session.
    pub exe: String,
    pub pid: u32,
    /// The session's stream is running. Players stop it when paused.
    pub active: bool,
    volume: ISimpleAudioVolume,
    meter: Option<IAudioMeterInformation>,
}

impl Session {
    pub fn level(&self) -> f32 {
        unsafe { self.volume.GetMasterVolume() }.unwrap_or(0.0)
    }

    /// The loudest sample since the last call, 0 to 1.
    pub fn peak(&self) -> f32 {
        self.meter.as_ref().and_then(|m| unsafe { m.GetPeakValue() }.ok()).unwrap_or(0.0)
    }

    pub fn muted(&self) -> bool {
        unsafe { self.volume.GetMute() }.map(|m| m.as_bool()).unwrap_or(false)
    }

    fn set(&self, level: f32) {
        unsafe {
            let _ = self.volume.SetMasterVolume(level.clamp(0.0, 1.0), std::ptr::null());
        }
    }

    fn set_mute(&self, mute: bool) {
        unsafe {
            let _ = self.volume.SetMute(mute, std::ptr::null());
        }
    }
}

/// Live audio sessions on every active output device.
pub fn sessions(procs: &[Process]) -> Vec<Session> {
    let exe_of: HashMap<u32, &str> = procs.iter().map(|p| (p.pid, p.exe.as_str())).collect();
    let own = std::process::id();
    let mut out = Vec::new();
    let Some(enumerator) = crate::audio::enumerator() else { return out };
    unsafe {
        let Ok(devices) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
        for d in 0..devices.GetCount().unwrap_or(0) {
            let Ok(device) = devices.Item(d) else { continue };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let Ok(list) = manager.GetSessionEnumerator() else { continue };
            for i in 0..list.GetCount().unwrap_or(0) {
                let Ok(control) = list.GetSession(i) else { continue };
                let Ok(state) = control.GetState() else { continue };
                if state == AudioSessionStateExpired {
                    continue;
                }
                let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
                let Ok(pid) = control2.GetProcessId() else { continue };
                if pid == own {
                    continue;
                }
                let Some(exe) = exe_of.get(&pid) else { continue };
                let Ok(volume) = control.cast::<ISimpleAudioVolume>() else { continue };
                let meter = control.cast::<IAudioMeterInformation>().ok();
                let active = state == AudioSessionStateActive;
                out.push(Session { exe: (*exe).to_owned(), pid, active, volume, meter });
            }
        }
    }
    out
}

/// Whether any audio session of `exes` is playing, or `None` when they have none.
pub fn playing(sessions: &[Session], exes: &[String]) -> Option<bool> {
    let mut mine = sessions.iter().filter(|s| exes.contains(&s.exe)).peekable();
    mine.peek()?;
    Some(mine.any(|s| s.active))
}

/// Exe names whose audio sessions belong to a target.
pub fn exes_for(id: &str) -> Vec<String> {
    if let Some(exe) = id.strip_prefix("exe:") {
        return vec![exe.to_owned()];
    }
    let aumid = id.strip_prefix("smtc:").unwrap_or(id);
    let lower = aumid.to_lowercase();
    if lower.ends_with(".exe") {
        return vec![lower];
    }
    let known = match aumid {
        "308046B0AF4A39CB" | "6F193CCC56814779" | "E7CF176E110C211B" => "firefox.exe",
        "MSEdge" => "msedge.exe",
        "Chrome" => "chrome.exe",
        _ => "",
    };
    if !known.is_empty() {
        return vec![known.to_owned()];
    }

    // Packaged apps (`Publisher.App_hash!App`) usually run `App.exe`.
    let app = aumid.rsplit('!').next().unwrap_or(aumid);
    let app = app.rsplit('.').next().unwrap_or(app);
    vec![format!("{}.exe", app.to_lowercase())]
}

/// Volume and mute of the first matching session.
pub fn level(sessions: &[Session], exes: &[String]) -> Option<(f32, bool)> {
    let session = sessions.iter().find(|s| exes.contains(&s.exe))?;
    Some(((session.level() * 100.0).round() / 100.0, session.muted()))
}

/// Moves every session of `id` to `change(first session's level)`.
/// Raising the volume also unmutes, like the Volume Mixer.
pub fn adjust(id: &str, change: impl Fn(f32) -> f32) {
    let exes = exes_for(id);
    let sessions = sessions(&processes());
    let mut matching = sessions.iter().filter(|s| exes.contains(&s.exe)).peekable();
    let Some(first) = matching.peek() else { return };
    let before = first.level();
    let level = (change(before) * 100.0).round().clamp(0.0, 100.0) / 100.0;
    for session in matching {
        session.set(level);
        if level > before {
            session.set_mute(false);
        }
    }
}

pub fn toggle_mute(id: &str) {
    let exes = exes_for(id);
    let sessions = sessions(&processes());
    let matching: Vec<&Session> = sessions.iter().filter(|s| exes.contains(&s.exe)).collect();
    let Some(first) = matching.first() else { return };
    let mute = !first.muted();
    matching.iter().for_each(|s| s.set_mute(mute));
}
