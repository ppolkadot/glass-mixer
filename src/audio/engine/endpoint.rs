//! Endpoint discovery, naming, and Core Audio volume/meter ownership.

use super::WinResult;
use super::error::{ApplyError, Context};
use crate::audio::Msg;
use crate::audio::callbacks::{EndpointVolumeNotifier, OWN_EVENT_CONTEXT};
use crate::audio::com::{is_not_found, take_pwstr};
use crate::audio::model::{DeviceInfo, EndpointState, balance_from_channels, channels_from_balance};
use crossbeam_channel::Sender;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioMeterInformation,
};
use windows::Win32::Media::Audio::{DEVICE_STATE_ACTIVE, EDataFlow, IMMDevice, IMMDeviceEnumerator, eConsole};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CLSCTX_ALL, STGM_READ};
use windows::core::HSTRING;

pub(super) fn list_devices(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> WinResult<Vec<DeviceInfo>> {
    let collection = unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)? };
    let count = unsafe { collection.GetCount()? };
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let device = unsafe { collection.Item(i)? };
        let id = take_pwstr(unsafe { device.GetId()? });
        let name = device_name(&device).unwrap_or_else(|| "Unknown device".into());
        out.push(DeviceInfo { id, name, is_default: false });
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    Ok(out)
}

pub(super) fn default_device_id(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> WinResult<Option<String>> {
    match unsafe { enumerator.GetDefaultAudioEndpoint(flow, eConsole) } {
        Ok(device) => Ok(Some(take_pwstr(unsafe { device.GetId()? }))),
        Err(e) if is_not_found(e.code()) => Ok(None),
        Err(e) => Err(e),
    }
}

fn device_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let s = take_pwstr(PropVariantToStringAlloc(&value).ok()?);
        (!s.is_empty()).then_some(s)
    }
}

pub(super) struct Endpoint {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) device: IMMDevice,
    pub(super) volume: IAudioEndpointVolume,
    callback: IAudioEndpointVolumeCallback,
    pub(super) meter: Option<IAudioMeterInformation>,
}

impl Endpoint {
    pub(super) fn set_balance(&self, balance: f32) -> Result<(), ApplyError> {
        let what = || format!("Couldn't change the balance of {}", self.name);
        unsafe {
            if self.volume.GetChannelCount().context(what)? != 2 {
                return Err(ApplyError::Message(format!("{} doesn't support stereo balance.", self.name)));
            }
            let master = self.volume.GetMasterVolumeLevelScalar().context(what)?;
            let (l, r) = channels_from_balance(balance);
            self.volume.SetChannelVolumeLevelScalar(0, master * l, &OWN_EVENT_CONTEXT).context(what)?;
            self.volume.SetChannelVolumeLevelScalar(1, master * r, &OWN_EVENT_CONTEXT).context(what)?;
        }
        Ok(())
    }

    pub(super) fn open(enumerator: &IMMDeviceEnumerator, id: &str, tx: Sender<Msg>) -> WinResult<Self> {
        let device = unsafe { enumerator.GetDevice(&HSTRING::from(id))? };
        let volume: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let callback = EndpointVolumeNotifier::create(tx);
        unsafe { volume.RegisterControlChangeNotify(&callback)? };
        let meter = unsafe { device.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None) }.ok();
        let name = device_name(&device).unwrap_or_else(|| "Unknown device".into());
        Ok(Self { id: id.to_string(), name, device, volume, callback, meter })
    }

    pub(super) fn refresh_name(&mut self, devices: &[DeviceInfo]) {
        if let Some(d) = devices.iter().find(|d| d.id == self.id) {
            self.name.clone_from(&d.name);
        }
    }

    pub(super) fn state(&self) -> WinResult<EndpointState> {
        unsafe {
            let volume = self.volume.GetMasterVolumeLevelScalar()?;
            let muted = self.volume.GetMute()?.as_bool();
            let balance = if self.volume.GetChannelCount()? == 2 {
                let l = self.volume.GetChannelVolumeLevelScalar(0)?;
                let r = self.volume.GetChannelVolumeLevelScalar(1)?;
                Some(balance_from_channels(l, r))
            } else {
                None
            };
            Ok(EndpointState { id: self.id.clone(), name: self.name.clone(), volume, muted, balance })
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        unsafe {
            let _ = self.volume.UnregisterControlChangeNotify(&self.callback);
        }
    }
}
