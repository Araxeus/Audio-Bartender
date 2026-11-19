//! Per-app volume through the Windows audio session API (the same sessions the
//! Volume Mixer shows).

use std::collections::HashMap;

use windows::Win32::Media::Audio::{
    AudioSessionStateExpired, DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::Interface;

use crate::util::Process;

pub struct Session {
    /// Lowercased exe name of the process that owns the session.
    pub exe: String,
    volume: ISimpleAudioVolume,
}

impl Session {
    pub fn get(&self) -> f32 {
        unsafe { self.volume.GetMasterVolume() }.unwrap_or(0.0)
    }

    pub fn set(&self, level: f32) {
        unsafe {
            let _ = self.volume.SetMasterVolume(level.clamp(0.0, 1.0), std::ptr::null());
        }
    }
}

/// Live audio sessions on every active output device.
pub fn sessions(procs: &[Process]) -> Vec<Session> {
    let exe_of: HashMap<u32, &str> = procs.iter().map(|p| (p.pid, p.exe.as_str())).collect();
    let mut out = Vec::new();
    unsafe {
        let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
            return out;
        };
        let Ok(devices) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
        for d in 0..devices.GetCount().unwrap_or(0) {
            let Ok(device) = devices.Item(d) else { continue };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let Ok(list) = manager.GetSessionEnumerator() else { continue };
            for i in 0..list.GetCount().unwrap_or(0) {
                let Ok(control) = list.GetSession(i) else { continue };
                if control.GetState().is_ok_and(|s| s == AudioSessionStateExpired) {
                    continue;
                }
                let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
                let Some(exe) = control2.GetProcessId().ok().and_then(|pid| exe_of.get(&pid)) else { continue };
                let Ok(volume) = control.cast::<ISimpleAudioVolume>() else { continue };
                out.push(Session { exe: (*exe).to_owned(), volume });
            }
        }
    }
    out
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

/// The target's volume, from its first audio session.
pub fn level(sessions: &[Session], exes: &[String]) -> Option<f32> {
    let level = sessions.iter().find(|s| exes.contains(&s.exe))?.get();
    Some((level * 100.0).round() / 100.0)
}

/// Moves every session of the target to the first session's level plus `delta`.
pub fn adjust(sessions: &[Session], exes: &[String], delta: f32) {
    let mut matching = sessions.iter().filter(|s| exes.contains(&s.exe));
    let Some(first) = matching.next() else { return };
    let level = (first.get() + delta).clamp(0.0, 1.0);
    first.set(level);
    matching.for_each(|s| s.set(level));
}
