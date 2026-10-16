//! Read-only: enumerate device properties and query stream-format support.
//! Does not initialize/start streams, change defaults, write properties or require elevation.
//! cargo run --example sound_properties_probe

#[allow(dead_code)]
#[path = "../src/audio/properties.rs"]
mod properties;

use properties::{AudioFormat, read_device_properties, read_system_mono};
use windows::{
    Win32::{
        Media::Audio::{
            AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED, DEVICE_STATE_ACTIVE, IAudioClient, IMMDevice,
            IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, eCapture, eRender,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
        },
    },
    core::Result,
};

struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn main() -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    }
    let _com = Com;
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    println!("System mono: {:?}", read_system_mono());
    for (flow, label) in [(eRender, "output"), (eCapture, "input")] {
        let devices = unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)? };
        let count = unsafe { devices.GetCount()? };
        println!("{label}: {count} active device(s)");
        for i in 0..count {
            let device = unsafe { devices.Item(i)? };
            let id = unsafe { device.GetId()? };
            println!("  device {}: {}", i + 1, unsafe { id.to_string()? });
            unsafe {
                CoTaskMemFree(Some(id.0.cast()));
            }
            let props = read_device_properties(&device, flow == eRender);
            println!("  selected default format: {:?}", props.default_format.as_ref().map(ToString::to_string));
            println!("  internal mix format: {:?}", props.mix_format.as_ref().map(ToString::to_string));
            println!("  legacy system effects enabled: {:?}", props.enhancements);
            println!("  spatial availability: {:?}", props.spatial.as_ref().map(ToString::to_string));
            probe_formats(&device)?;
        }
    }
    Ok(())
}

fn probe_formats(device: &IMMDevice) -> Result<()> {
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    for rate in [44_100, 48_000, 96_000] {
        let format = WAVEFORMATEX {
            wFormatTag: 1,
            nChannels: 2,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * 4,
            nBlockAlign: 4,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let mut closest = std::ptr::null_mut();
        let shared = unsafe { client.IsFormatSupported(AUDCLNT_SHAREMODE_SHARED, &format, Some(&mut closest)) };
        let closest_text = if closest.is_null() {
            None
        } else {
            let bytes = unsafe { std::slice::from_raw_parts(closest.cast(), 18 + (*closest).cbSize as usize) };
            let parsed = AudioFormat::from_wave_bytes(bytes).map(|f| f.to_string());
            unsafe {
                CoTaskMemFree(Some(closest.cast()));
            }
            Some(parsed)
        };
        let exclusive = unsafe { client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &format, None) };
        println!("  PCM 2ch 16-bit {rate} Hz: shared={shared:?}, exclusive={exclusive:?}, closest={closest_text:?}");
    }
    Ok(())
}
