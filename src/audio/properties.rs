//! Read-only sound properties. Call Core Audio readers on the engine's COM thread.

use std::fmt;
use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{
            ENDPOINT_SYSFX_DISABLED, ENDPOINT_SYSFX_ENABLED, IAudioClient, IMMDevice, ISpatialAudioClient,
            PKEY_AudioEndpoint_Disable_SysFx, PKEY_AudioEngine_DeviceFormat,
        },
        System::{
            Com::{
                CLSCTX_ALL, CoTaskMemFree, STGM_READ,
                StructuredStorage::{PROPVARIANT, PropVariantClear},
            },
            Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
            Variant::{VT_BLOB, VT_UI4},
        },
    },
    core::HSTRING,
};

pub const SOUND_SETTINGS_URI: &str = "ms-settings:sound";
pub const MONO_SETTINGS_URI: &str = "ms-settings:easeofaccess-audio";
pub const MONO_REGISTRY_SUBKEY: &str = r"Software\Microsoft\Multimedia\Audio";
pub const MONO_REGISTRY_VALUE: &str = "AccessibilityMonoMixState";

/// Each field can fail independently without making volume controls unavailable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceProperties {
    /// User-selected shared-mode format from PKEY_AudioEngine_DeviceFormat.
    pub default_format: Result<AudioFormat, String>,
    /// Engine's internal processing format. This is NOT the device default.
    pub mix_format: Result<AudioFormat, String>,
    /// Legacy LFX/GFX system effects flag, not every driver/vendor enhancement.
    pub enhancements: Result<bool, String>,
    pub spatial: Result<SpatialSupport, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpatialSupport {
    pub max_dynamic_objects: u32,
}

impl fmt::Display for SpatialSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.max_dynamic_objects == 0 {
            f.write_str("Dynamic spatial audio is unavailable or not enabled")
        } else {
            write!(f, "Dynamic spatial audio available ({} objects)", self.max_dynamic_objects)
        }
    }
}

/// Whether a changed endpoint property affects anything shown: device names, the
/// selected format (which the mix format follows) or the system-effects flag.
pub fn needs_device_refresh(key: &PROPERTYKEY) -> bool {
    use windows::Win32::Devices::FunctionDiscovery::{
        PKEY_Device_DeviceDesc, PKEY_Device_FriendlyName, PKEY_DeviceInterface_FriendlyName,
    };
    [
        PKEY_AudioEngine_DeviceFormat,
        PKEY_AudioEndpoint_Disable_SysFx,
        PKEY_Device_FriendlyName,
        PKEY_Device_DeviceDesc,
        PKEY_DeviceInterface_FriendlyName,
    ]
    .contains(key)
}

pub fn read_device_properties(device: &IMMDevice, is_output: bool) -> DeviceProperties {
    DeviceProperties {
        default_format: read_default_format(device),
        mix_format: read_mix_format(device),
        enhancements: read_enhancements(device),
        spatial: if is_output {
            read_spatial(device)
        } else {
            Err("Spatial playback does not apply to input devices.".into())
        },
    }
}

struct PropertyValue(PROPVARIANT);

impl Drop for PropertyValue {
    fn drop(&mut self) {
        unsafe {
            let _ = PropVariantClear(&mut self.0);
        }
    }
}

fn property(device: &IMMDevice, key: &PROPERTYKEY) -> Result<PropertyValue, String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).map_err(|e| format!("Property store unavailable: {e}"))?;
        store.GetValue(key).map(PropertyValue).map_err(|e| format!("Windows property unavailable: {e}"))
    }
}

fn read_default_format(device: &IMMDevice) -> Result<AudioFormat, String> {
    let value = property(device, &PKEY_AudioEngine_DeviceFormat)?;
    unsafe {
        let inner = &value.0.Anonymous.Anonymous;
        if inner.vt != VT_BLOB {
            return Err("Windows did not expose the selected device format.".into());
        }
        let blob = inner.Anonymous.blob;
        if blob.pBlobData.is_null() || blob.cbSize < 18 || blob.cbSize > 65_553 {
            return Err("Windows returned an invalid device-format description.".into());
        }
        AudioFormat::from_wave_bytes(std::slice::from_raw_parts(blob.pBlobData, blob.cbSize as usize))
    }
}

fn read_mix_format(device: &IMMDevice) -> Result<AudioFormat, String> {
    unsafe {
        let client: IAudioClient =
            device.Activate(CLSCTX_ALL, None).map_err(|e| format!("Audio client unavailable: {e}"))?;
        let ptr = client.GetMixFormat().map_err(|e| format!("Mix format unavailable: {e}"))?;
        if ptr.is_null() {
            return Err("Windows returned an empty mix format.".into());
        }
        // GetMixFormat owns a WAVEFORMATEX plus cbSize bytes. Copy before freeing.
        let size = 18 + (*ptr).cbSize as usize;
        let result = AudioFormat::from_wave_bytes(std::slice::from_raw_parts(ptr.cast(), size));
        CoTaskMemFree(Some(ptr.cast()));
        result
    }
}

fn read_enhancements(device: &IMMDevice) -> Result<bool, String> {
    let value = property(device, &PKEY_AudioEndpoint_Disable_SysFx)?;
    unsafe {
        let inner = &value.0.Anonymous.Anonymous;
        if inner.vt != VT_UI4 {
            return Err("This device does not expose its system effects setting.".into());
        }
        match inner.Anonymous.ulVal {
            ENDPOINT_SYSFX_ENABLED => Ok(true),
            ENDPOINT_SYSFX_DISABLED => Ok(false),
            _ => Err("Windows returned an unrecognized system effects setting.".into()),
        }
    }
}

fn read_spatial(device: &IMMDevice) -> Result<SpatialSupport, String> {
    unsafe {
        let client: ISpatialAudioClient = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| format!("Spatial audio availability could not be read: {e}"))?;
        client
            .GetMaxDynamicObjectCount()
            .map(|max_dynamic_objects| SpatialSupport { max_dynamic_objects })
            .map_err(|e| format!("Spatial audio availability could not be read: {e}"))
    }
}

/// Reads an observed Windows implementation detail, not a documented control API.
/// Missing or unexpected values remain unknown. Never writes this registry value.
pub fn read_system_mono() -> Result<bool, String> {
    let mut data = 0u32;
    let mut size = 4u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(MONO_REGISTRY_SUBKEY),
            &HSTRING::from(MONO_REGISTRY_VALUE),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return Err(format!(
            "Windows mono audio state is unavailable (registry error {}). Open Accessibility audio settings.",
            status.0
        ));
    }
    match (size, data) {
        (4, 0) => Ok(false),
        (4, 1) => Ok(true),
        _ => Err("Windows returned an unrecognized mono audio state. Open Accessibility audio settings.".into()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioFormat {
    pub channels: u16,
    pub sample_rate: u32,
    pub container_bits: u16,
    pub valid_bits: u16,
    pub encoding: String,
}

impl AudioFormat {
    pub fn from_wave_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 18 {
            return Err("Truncated WAVEFORMATEX.".into());
        }
        let u16_at = |i| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        let u32_at = |i| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        let extra = u16_at(16) as usize;
        if bytes.len() < 18 + extra {
            return Err("Truncated wave-format extension.".into());
        }
        let channels = u16_at(2);
        let sample_rate = u32_at(4);
        let container_bits = u16_at(14);
        let mut valid_bits = container_bits;
        let tag = u16_at(0);
        let encoding = if tag == 0xfffe {
            if extra < 22 {
                return Err("Truncated WAVEFORMATEXTENSIBLE.".into());
            }
            valid_bits = u16_at(18);
            // Match the whole canonical subtype GUID; an unrelated GUID's first DWORD is not a format tag.
            const SUFFIX: [u8; 12] = [0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71];
            if bytes[28..40] == SUFFIX {
                match u32_at(24) {
                    1 => "PCM".into(),
                    3 => "float".into(),
                    other => format!("subtype 0x{other:08x}"),
                }
            } else {
                "other encoding".into()
            }
        } else {
            match tag {
                1 => "PCM".into(),
                3 => "float".into(),
                other => format!("format 0x{other:04x}"),
            }
        };
        if channels == 0 || sample_rate == 0 || valid_bits > container_bits {
            return Err("Invalid channel count, sample rate or precision.".into());
        }
        Ok(Self { channels, sample_rate, container_bits, valid_bits, encoding })
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ch, {} Hz, {}-bit {}", self.channels, self.sample_rate, self.valid_bits, self.encoding)?;
        if self.valid_bits != self.container_bits {
            write!(f, " ({}-bit container)", self.container_bits)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm() -> Vec<u8> {
        vec![1, 0, 2, 0, 0x80, 0xbb, 0, 0, 0, 0xee, 2, 0, 4, 0, 16, 0, 0, 0]
    }

    #[test]
    fn parses_pcm_and_extensible_precision() {
        let parsed = AudioFormat::from_wave_bytes(&pcm()).unwrap();
        assert_eq!((parsed.channels, parsed.sample_rate, parsed.valid_bits), (2, 48_000, 16));
        assert_eq!(parsed.encoding, "PCM");
        let mut bytes = pcm();
        bytes[0..2].copy_from_slice(&0xfffe_u16.to_le_bytes());
        bytes[12..14].copy_from_slice(&8_u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&32_u16.to_le_bytes());
        bytes[16..18].copy_from_slice(&22_u16.to_le_bytes());
        bytes.extend_from_slice(&[24, 0, 3, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
        let parsed = AudioFormat::from_wave_bytes(&bytes).unwrap();
        assert_eq!((parsed.valid_bits, parsed.container_bits), (24, 32));
        assert_eq!(parsed.encoding, "PCM");
    }

    #[test]
    fn only_displayed_properties_trigger_a_device_refresh() {
        use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
        use windows::Win32::Media::Audio::{PKEY_AudioEndpoint_FormFactor, PKEY_AudioEndpoint_JackSubType};
        assert!(needs_device_refresh(&PKEY_AudioEngine_DeviceFormat));
        assert!(needs_device_refresh(&PKEY_AudioEndpoint_Disable_SysFx));
        assert!(needs_device_refresh(&PKEY_Device_FriendlyName));
        assert!(!needs_device_refresh(&PKEY_AudioEndpoint_FormFactor));
        assert!(!needs_device_refresh(&PKEY_AudioEndpoint_JackSubType));
        let vendor = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x1234), pid: 7 };
        assert!(!needs_device_refresh(&vendor), "unknown driver-specific properties are ignored");
    }

    #[test]
    fn rejects_truncated_and_invalid_formats() {
        for n in 0..18 {
            assert!(AudioFormat::from_wave_bytes(&pcm()[..n]).is_err());
        }
        let mut bytes = pcm();
        bytes[16] = 22;
        assert!(AudioFormat::from_wave_bytes(&bytes).is_err());
        bytes = pcm();
        bytes[2] = 0;
        assert!(AudioFormat::from_wave_bytes(&bytes).is_err());
    }
}
