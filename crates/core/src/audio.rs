//! Output devices and the master volume of the default device.

use std::ffi::c_void;

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, DEVICE_STATE_ACTIVE, EDataFlow, ERole, IMMDevice,
    IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator, eCommunications,
    eConsole, eMultimedia, eRender,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::core::{GUID, HRESULT, PCWSTR, implement, interface};

use crate::Device;

pub fn enumerator() -> Option<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok() }
}

fn default_device(enumerator: &IMMDeviceEnumerator) -> Option<IMMDevice> {
    unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok() }
}

fn device_id(device: &IMMDevice) -> Option<String> {
    unsafe {
        let id = device.GetId().ok()?;
        let text = id.to_string().ok();
        CoTaskMemFree(Some(id.0 as *const c_void));
        text
    }
}

fn device_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let text = PropVariantToStringAlloc(&value).ok()?;
        let name = text.to_string().ok();
        CoTaskMemFree(Some(text.0 as *const c_void));
        name
    }
}

/// Active output devices, with the default one marked.
pub fn devices(enumerator: &IMMDeviceEnumerator) -> Vec<Device> {
    let default = default_device(enumerator).and_then(|d| device_id(&d));
    let mut out = Vec::new();
    unsafe {
        let Ok(list) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
        for i in 0..list.GetCount().unwrap_or(0) {
            let Ok(device) = list.Item(i) else { continue };
            let Some(id) = device_id(&device) else { continue };
            let name = device_name(&device).unwrap_or_else(|| "Unknown device".into());
            out.push(Device { default: default.as_deref() == Some(id.as_str()), id, name });
        }
    }
    out
}

/// The default device's volume control.
pub fn master(enumerator: &IMMDeviceEnumerator) -> Option<IAudioEndpointVolume> {
    let device = default_device(enumerator)?;
    unsafe { device.Activate(CLSCTX_ALL, None).ok() }
}

/// Level from 0 to 1 and mute state.
pub fn read(volume: &IAudioEndpointVolume) -> (f32, bool) {
    unsafe {
        let level = volume.GetMasterVolumeLevelScalar().unwrap_or(0.0);
        let muted = volume.GetMute().map(|m| m.as_bool()).unwrap_or(false);
        ((level * 100.0).round() / 100.0, muted)
    }
}

/// Changes the master volume by `delta`. Raising the volume also unmutes, like the Windows flyout.
pub fn adjust_master(delta: f32) -> Option<(f32, bool)> {
    let volume = master(&enumerator()?)?;
    let (level, muted) = read(&volume);
    let level = ((level + delta) * 100.0).round().clamp(0.0, 100.0) / 100.0;
    unsafe {
        let _ = volume.SetMasterVolumeLevelScalar(level, std::ptr::null());
        if muted && delta > 0.0 {
            let _ = volume.SetMute(false, std::ptr::null());
        }
    }
    Some(read(&volume))
}

pub fn set_master(level: f32) {
    let Some(volume) = enumerator().and_then(|e| master(&e)) else { return };
    unsafe {
        let _ = volume.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
    }
}

pub fn toggle_master_mute() {
    let Some(volume) = enumerator().and_then(|e| master(&e)) else { return };
    let (_, muted) = read(&volume);
    unsafe {
        let _ = volume.SetMute(!muted, std::ptr::null());
    }
}

// The interface macro keeps the COM method names.
#[allow(non_snake_case)]
mod policy {
    use super::*;

/// Undocumented, but stable since Windows 7 and the only way to change the default
/// device. EarTrumpet and SoundSwitch use the same interface.
#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: windows::core::IUnknown {
    fn GetMixFormat(&self, id: PCWSTR, format: *mut *mut c_void) -> HRESULT;
    fn GetDeviceFormat(&self, id: PCWSTR, default: i32, format: *mut *mut c_void) -> HRESULT;
    fn ResetDeviceFormat(&self, id: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(&self, id: PCWSTR, endpoint: *mut c_void, mix: *mut c_void) -> HRESULT;
    fn GetProcessingPeriod(&self, id: PCWSTR, default: i32, period: *mut i64, min: *mut i64) -> HRESULT;
    fn SetProcessingPeriod(&self, id: PCWSTR, period: *mut i64) -> HRESULT;
    fn GetShareMode(&self, id: PCWSTR, mode: *mut c_void) -> HRESULT;
    fn SetShareMode(&self, id: PCWSTR, mode: *mut c_void) -> HRESULT;
    fn GetPropertyValue(&self, id: PCWSTR, key: *const PROPERTYKEY, value: *mut c_void) -> HRESULT;
    fn SetPropertyValue(&self, id: PCWSTR, key: *const PROPERTYKEY, value: *mut c_void) -> HRESULT;
    fn SetDefaultEndpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
    fn SetEndpointVisibility(&self, id: PCWSTR, visible: i32) -> HRESULT;
}

const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// Makes `id` the default output for every role.
pub fn set_default_device(id: &str) {
    let id = crate::util::wide(id);
    unsafe {
        let Ok(policy) = CoCreateInstance::<_, IPolicyConfig>(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) else { return };
        for role in [eConsole, eMultimedia, eCommunications] {
            let _ = policy.SetDefaultEndpoint(PCWSTR(id.as_ptr()), role);
        }
    }
}
}
pub use policy::set_default_device;

/// Calls `on_change(devices_changed)` when the master volume, mute or device set changes.
pub struct Watcher {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
    endpoint: Option<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)>,
    on_change: std::sync::Arc<dyn Fn(bool) + Send + Sync>,
}

impl Watcher {
    pub fn new(on_change: impl Fn(bool) + Send + Sync + 'static) -> Option<Watcher> {
        let enumerator = enumerator()?;
        let on_change: std::sync::Arc<dyn Fn(bool) + Send + Sync> = std::sync::Arc::new(on_change);
        let client: IMMNotificationClient = DeviceEvents(on_change.clone()).into();
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client).ok()? };
        let mut watcher = Watcher { enumerator, client, endpoint: None, on_change };
        watcher.follow_default();
        Some(watcher)
    }

    pub fn enumerator(&self) -> &IMMDeviceEnumerator {
        &self.enumerator
    }

    /// Re-attaches the volume callback to the current default device.
    pub fn follow_default(&mut self) {
        self.detach();
        let Some(volume) = master(&self.enumerator) else { return };
        let callback: IAudioEndpointVolumeCallback = VolumeEvents(self.on_change.clone()).into();
        if unsafe { volume.RegisterControlChangeNotify(&callback) }.is_ok() {
            self.endpoint = Some((volume, callback));
        }
    }

    pub fn master(&self) -> Option<&IAudioEndpointVolume> {
        self.endpoint.as_ref().map(|(volume, _)| volume)
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            let _ = self.enumerator.UnregisterEndpointNotificationCallback(&self.client);
        }
        self.detach();
    }
}

impl Watcher {
    fn detach(&mut self) {
        if let Some((volume, callback)) = self.endpoint.take() {
            unsafe {
                let _ = volume.UnregisterControlChangeNotify(&callback);
            }
        }
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeEvents(std::sync::Arc<dyn Fn(bool) + Send + Sync>);

impl IAudioEndpointVolumeCallback_Impl for VolumeEvents_Impl {
    fn OnNotify(&self, _: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        (self.0)(false);
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceEvents(std::sync::Arc<dyn Fn(bool) + Send + Sync>);

impl IMMNotificationClient_Impl for DeviceEvents_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        (self.0)(true);
        Ok(())
    }

    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        (self.0)(true);
        Ok(())
    }

    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        (self.0)(true);
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> windows::core::Result<()> {
        if flow == eRender && role == eConsole {
            (self.0)(true);
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}
