//! Sending an app to another output device. Windows keeps a per-app default device
//! through a private WinRT factory, the one behind its "App volume and device
//! preferences" page. EarTrumpet uses the same one, and its interop is the reference
//! for the layout below. Windows can forget a choice after a restart or a driver
//! update, so the worker applies its saved routes again as apps start.

#![allow(non_snake_case)]

use std::ffi::c_void;

use windows::Win32::Media::Audio::{EDataFlow, ERole, eConsole, eMultimedia, eRender};
use windows::Win32::System::WinRT::RoGetActivationFactory;
use windows_core::{HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, interface};

const CLASS: &str = "Windows.Media.Internal.AudioPolicyConfig";
/// An endpoint id becomes a device interface path: this prefix, the id, then the
/// render interface class.
const PREFIX: &str = r"\\?\SWD#MMDEVAPI#";
const RENDER: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";

/// Windows 10 21H2 and later: an IInspectable whose first nineteen methods we don't
/// use, then the three we do. Declared on IUnknown with IInspectable's methods spelled
/// out, which keeps the same layout.
#[interface("ab3d4648-e242-459f-b02f-541c70306324")]
unsafe trait IAudioPolicyConfigFactory: IUnknown {
    // IInspectable's three methods.
    fn GetIids(&self) -> HRESULT;
    fn GetRuntimeClassName(&self) -> HRESULT;
    fn GetTrustLevel(&self) -> HRESULT;
    fn Unused0(&self) -> HRESULT;
    fn Unused1(&self) -> HRESULT;
    fn Unused2(&self) -> HRESULT;
    fn Unused3(&self) -> HRESULT;
    fn Unused4(&self) -> HRESULT;
    fn Unused5(&self) -> HRESULT;
    fn Unused6(&self) -> HRESULT;
    fn Unused7(&self) -> HRESULT;
    fn Unused8(&self) -> HRESULT;
    fn Unused9(&self) -> HRESULT;
    fn Unused10(&self) -> HRESULT;
    fn Unused11(&self) -> HRESULT;
    fn Unused12(&self) -> HRESULT;
    fn Unused13(&self) -> HRESULT;
    fn Unused14(&self) -> HRESULT;
    fn Unused15(&self) -> HRESULT;
    fn Unused16(&self) -> HRESULT;
    fn Unused17(&self) -> HRESULT;
    fn Unused18(&self) -> HRESULT;
    fn SetPersistedDefaultAudioEndpoint(
        &self,
        process: u32,
        flow: EDataFlow,
        role: ERole,
        device: *mut c_void,
    ) -> HRESULT;
    fn GetPersistedDefaultAudioEndpoint(
        &self,
        process: u32,
        flow: EDataFlow,
        role: ERole,
        device: *mut *mut c_void,
    ) -> HRESULT;
    fn ClearAllPersistedApplicationDefaultEndpoints(&self) -> HRESULT;
}

/// Earlier Windows 10: the same layout under another id.
#[interface("2a59116d-6c4f-45e0-a74f-707e3fef9258")]
unsafe trait IAudioPolicyConfigFactoryDownlevel: IUnknown {
    // IInspectable's three methods.
    fn GetIids(&self) -> HRESULT;
    fn GetRuntimeClassName(&self) -> HRESULT;
    fn GetTrustLevel(&self) -> HRESULT;
    fn Unused0(&self) -> HRESULT;
    fn Unused1(&self) -> HRESULT;
    fn Unused2(&self) -> HRESULT;
    fn Unused3(&self) -> HRESULT;
    fn Unused4(&self) -> HRESULT;
    fn Unused5(&self) -> HRESULT;
    fn Unused6(&self) -> HRESULT;
    fn Unused7(&self) -> HRESULT;
    fn Unused8(&self) -> HRESULT;
    fn Unused9(&self) -> HRESULT;
    fn Unused10(&self) -> HRESULT;
    fn Unused11(&self) -> HRESULT;
    fn Unused12(&self) -> HRESULT;
    fn Unused13(&self) -> HRESULT;
    fn Unused14(&self) -> HRESULT;
    fn Unused15(&self) -> HRESULT;
    fn Unused16(&self) -> HRESULT;
    fn Unused17(&self) -> HRESULT;
    fn Unused18(&self) -> HRESULT;
    fn SetPersistedDefaultAudioEndpoint(
        &self,
        process: u32,
        flow: EDataFlow,
        role: ERole,
        device: *mut c_void,
    ) -> HRESULT;
    fn GetPersistedDefaultAudioEndpoint(
        &self,
        process: u32,
        flow: EDataFlow,
        role: ERole,
        device: *mut *mut c_void,
    ) -> HRESULT;
    fn ClearAllPersistedApplicationDefaultEndpoints(&self) -> HRESULT;
}

/// The factory under whichever id this Windows knows. Both have the same layout, so
/// the older one is used through the newer type.
fn factory() -> Option<IAudioPolicyConfigFactory> {
    let class = HSTRING::from(CLASS);
    unsafe {
        if let Ok(factory) = RoGetActivationFactory::<IAudioPolicyConfigFactory>(&class) {
            return Some(factory);
        }
        let old = RoGetActivationFactory::<IAudioPolicyConfigFactoryDownlevel>(&class).ok()?;
        Some(std::mem::transmute::<IAudioPolicyConfigFactoryDownlevel, IAudioPolicyConfigFactory>(old))
    }
}

/// Sends `process` to the output device with endpoint id `device`, or back to the
/// default device with `None`. Sets both roles, as EarTrumpet does. Returns false when
/// Windows refused or lacks the interface.
pub fn set(process: u32, device: Option<&str>) -> bool {
    // Debug builds can show routes without moving any real app's audio.
    if cfg!(debug_assertions) && std::env::var_os("AUDIO_BARTENDER_ROUTE_DRY_RUN").is_some() {
        crate::util::log(format!("route {process} -> {device:?}: dry run"));
        return true;
    }
    let Some(factory) = factory() else { return false };
    let path = device.map(|id| HSTRING::from(format!("{PREFIX}{id}{RENDER}"))).unwrap_or_default();
    // An HSTRING is a handle; an empty one is null and clears the choice.
    let handle: *mut c_void = unsafe { std::mem::transmute_copy(&path) };
    let ok = [eMultimedia, eConsole]
        .into_iter()
        .all(|role| unsafe { factory.SetPersistedDefaultAudioEndpoint(process, eRender, role, handle) }.is_ok());
    crate::util::log(format!("route {process} -> {device:?}: {ok}"));
    ok
}

#[cfg(test)]
mod tests {
    use windows::Win32::Media::Audio::{ERole, eConsole, eMultimedia, eRender};

    /// Reads, without changing, the saved device for the process in ROUTE_TEST_PID.
    #[test]
    fn reads_a_route() {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let Some(pid) = std::env::var("ROUTE_TEST_PID").ok().and_then(|p| p.parse::<u32>().ok()) else { return };
        let factory = super::factory().expect("AudioPolicyConfig factory");
        for role in [eConsole, eMultimedia, ERole(2)] {
            let mut out: *mut std::ffi::c_void = std::ptr::null_mut();
            let result = unsafe { factory.GetPersistedDefaultAudioEndpoint(pid, eRender, role, &mut out) };
            let text = if out.is_null() {
                String::new()
            } else {
                unsafe { std::mem::transmute::<*mut std::ffi::c_void, windows_core::HSTRING>(out) }.to_string()
            };
            eprintln!("role {}: {result:?} {text:?}", role.0);
        }
    }
}
