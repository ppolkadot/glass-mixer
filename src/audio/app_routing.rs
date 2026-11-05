//! Per-app output and input devices.
//!
//! Windows has no public API for this. The Settings app ("App volume and device
//! preferences") uses the undocumented WinRT activation factory
//! `Windows.Media.Internal.AudioPolicyConfig`. Its interface has one ID before
//! Windows 10 21H2 and another from then on, with the same method layout: after
//! `IInspectable` come 19 methods this module never calls, then the three below.
//! The layout matches EarTrumpet's long-standing binding
//! (`IAudioPolicyConfigFactoryVariantFor21H2.cs`). Both IDs are tried at
//! runtime; if neither is available, per-app routing is reported as unsupported.
//!
//! A preference is set for a process ID but Windows stores it for the app, so
//! it also applies to the app's later launches. A null device ID resets that one
//! app to the system default.
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use super::model::Flow;
use std::ffi::c_void;
use windows::Win32::Media::Audio::{EDataFlow, ERole, eCapture, eConsole, eMultimedia, eRender};
use windows::Win32::System::WinRT::RoGetActivationFactory;
use windows::core::{HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, Interface, interface};

const CLASS: &str = "Windows.Media.Internal.AudioPolicyConfig";
const MMDEVAPI_PREFIX: &str = r"\\?\SWD#MMDEVAPI#";
const RENDER_INTERFACE: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";
const CAPTURE_INTERFACE: &str = "#{2eef81be-33fa-4800-9670-1cd474972c3f}";

/// The factory interface (Windows 10 21H2 and later). It derives from
/// `IInspectable`, whose three methods are declared here as placeholders (the
/// `interface` macro can't derive from it); the next 19 are placeholders too.
/// Placeholders are never called; they keep the used methods in their slots.
#[interface("ab3d4648-e242-459f-b02f-541c70306324")]
unsafe trait IAudioPolicyConfigFactory: IUnknown {
    fn _GetIids(&self) -> HRESULT;
    fn _GetRuntimeClassName(&self) -> HRESULT;
    fn _GetTrustLevel(&self) -> HRESULT;
    fn _AddCtxVolumeChange(&self) -> HRESULT;
    fn _RemoveCtxVolumeChanged(&self) -> HRESULT;
    fn _AddRingerVibrateStateChanged(&self) -> HRESULT;
    fn _RemoveRingerVibrateStateChange(&self) -> HRESULT;
    fn _SetVolumeGroupGainForId(&self) -> HRESULT;
    fn _GetVolumeGroupGainForId(&self) -> HRESULT;
    fn _GetActiveVolumeGroupForEndpointId(&self) -> HRESULT;
    fn _GetVolumeGroupsForEndpoint(&self) -> HRESULT;
    fn _GetCurrentVolumeContext(&self) -> HRESULT;
    fn _SetVolumeGroupMuteForId(&self) -> HRESULT;
    fn _GetVolumeGroupMuteForId(&self) -> HRESULT;
    fn _SetRingerVibrateState(&self) -> HRESULT;
    fn _GetRingerVibrateState(&self) -> HRESULT;
    fn _SetPreferredChatApplication(&self) -> HRESULT;
    fn _ResetPreferredChatApplication(&self) -> HRESULT;
    fn _GetPreferredChatApplication(&self) -> HRESULT;
    fn _GetCurrentChatApplications(&self) -> HRESULT;
    fn _AddChatContextChanged(&self) -> HRESULT;
    fn _RemoveChatContextChanged(&self) -> HRESULT;
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

/// Only its ID is used: the same object, requested with the pre-21H2 ID.
#[interface("2a59116d-6c4f-45e0-a74f-707e3fef9258")]
unsafe trait IAudioPolicyConfigFactoryDownlevel: IUnknown {}

pub struct AppRouting(IAudioPolicyConfigFactory);

impl AppRouting {
    /// The factory, or the reason per-app routing is unavailable.
    pub fn create() -> windows::core::Result<Self> {
        let class = HSTRING::from(CLASS);
        match unsafe { RoGetActivationFactory::<IAudioPolicyConfigFactory>(&class) } {
            Ok(factory) => Ok(Self(factory)),
            Err(current) => {
                let older = unsafe { RoGetActivationFactory::<IAudioPolicyConfigFactoryDownlevel>(&class) }
                    .map_err(|_| current)?;
                // Same object and method layout; only the interface ID differs.
                Ok(Self(unsafe { IAudioPolicyConfigFactory::from_raw(older.into_raw()) }))
            }
        }
    }

    /// Sets (or, with `None`, resets) the device the app owning `pid` uses.
    pub fn set(&self, pid: u32, flow: Flow, device_id: Option<&str>) -> windows::core::Result<()> {
        let device = device_id.map(|id| HSTRING::from(pack_device_id(id, flow)));
        // An HSTRING is passed as its handle; a null handle means "the default".
        let raw = device.as_ref().map_or(std::ptr::null_mut(), |h| unsafe { std::mem::transmute_copy(h) });
        for role in [eMultimedia, eConsole] {
            unsafe { self.0.SetPersistedDefaultAudioEndpoint(pid, data_flow(flow), role, raw).ok()? };
        }
        Ok(())
    }

    /// The device the app owning `pid` is set to use, or `None` for the default.
    pub fn get(&self, pid: u32, flow: Flow) -> windows::core::Result<Option<String>> {
        let mut raw: *mut c_void = std::ptr::null_mut();
        unsafe { self.0.GetPersistedDefaultAudioEndpoint(pid, data_flow(flow), eMultimedia, &mut raw).ok()? };
        if raw.is_null() {
            return Ok(None);
        }
        // Takes ownership of the returned handle, which is released on drop.
        let value: HSTRING = unsafe { std::mem::transmute(raw) };
        Ok(unpack_device_id(&value.to_string_lossy()))
    }

    /// Resets every app to the system default. This affects all apps, not only
    /// those set in Glass Mixer.
    pub fn clear_all(&self) -> windows::core::Result<()> {
        unsafe { self.0.ClearAllPersistedApplicationDefaultEndpoints().ok() }
    }
}

fn data_flow(flow: Flow) -> EDataFlow {
    match flow {
        Flow::Output => eRender,
        Flow::Input => eCapture,
    }
}

/// The device-interface path the factory expects for an endpoint ID.
fn pack_device_id(id: &str, flow: Flow) -> String {
    let interface = if flow == Flow::Output { RENDER_INTERFACE } else { CAPTURE_INTERFACE };
    format!("{MMDEVAPI_PREFIX}{id}{interface}")
}

/// The endpoint ID inside a device-interface path; `None` for an empty value.
fn unpack_device_id(path: &str) -> Option<String> {
    let id = path.strip_prefix(MMDEVAPI_PREFIX).unwrap_or(path);
    let id = id.strip_suffix(RENDER_INTERFACE).or_else(|| id.strip_suffix(CAPTURE_INTERFACE)).unwrap_or(id);
    (!id.trim().is_empty()).then(|| id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "{0.0.0.00000000}.{6f1c2b7e-8d4a-4c59-9e3b-5a7d21c0f1a3}";

    #[test]
    fn endpoint_ids_round_trip_through_the_device_interface_path() {
        let output = pack_device_id(ID, Flow::Output);
        assert_eq!(
            output,
            r"\\?\SWD#MMDEVAPI#{0.0.0.00000000}.{6f1c2b7e-8d4a-4c59-9e3b-5a7d21c0f1a3}#{e6327cad-dcec-4949-ae8a-991e976a79d2}"
        );
        assert_eq!(unpack_device_id(&output).as_deref(), Some(ID));
        assert_eq!(unpack_device_id(&pack_device_id(ID, Flow::Input)).as_deref(), Some(ID));
        assert_eq!(unpack_device_id(ID).as_deref(), Some(ID), "a bare ID is accepted as is");
        assert_eq!(unpack_device_id(""), None);
    }

    #[test]
    fn the_vtable_places_the_used_methods_after_inspectable_and_19_others() {
        // IUnknown (3) + IInspectable (3) + 19 placeholders = slot 25.
        let size = std::mem::size_of::<usize>();
        assert_eq!(std::mem::size_of::<IUnknown_Vtbl>(), 3 * size);
        let set = std::mem::offset_of!(IAudioPolicyConfigFactory_Vtbl, SetPersistedDefaultAudioEndpoint);
        let get = std::mem::offset_of!(IAudioPolicyConfigFactory_Vtbl, GetPersistedDefaultAudioEndpoint);
        let clear = std::mem::offset_of!(IAudioPolicyConfigFactory_Vtbl, ClearAllPersistedApplicationDefaultEndpoints);
        assert_eq!((set / size, get / size, clear / size), (25, 26, 27));
    }
}
