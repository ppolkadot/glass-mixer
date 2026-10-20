//! Windows integration: window management, system tray, glass backdrop, app
//! icons, fonts, launch-at-login, single-instance and system preferences.

pub mod backdrop;
pub mod exit_request;
pub mod fonts;
pub mod hotkeys;
pub mod icons;
pub mod single_instance;
pub mod startup;
pub mod sysprefs;
pub mod tray;
pub mod window;

use windows::core::HSTRING;

/// Reads a `REG_DWORD` from `HKEY_CURRENT_USER`/`HKEY_LOCAL_MACHINE`.
pub(crate) fn reg_dword(root: windows::Win32::System::Registry::HKEY, subkey: &str, value: &str) -> Option<u32> {
    use windows::Win32::System::Registry::{RRF_RT_REG_DWORD, RegGetValueW};
    let mut data = 0u32;
    let mut size = 4u32;
    let err = unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    err.is_ok().then_some(data)
}

/// Reads a `REG_SZ` value.
pub(crate) fn reg_string(root: windows::Win32::System::Registry::HKEY, subkey: &str, value: &str) -> Option<String> {
    use windows::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};
    let subkey = HSTRING::from(subkey);
    let value = HSTRING::from(value);
    let mut size = 0u32;
    let err = unsafe { RegGetValueW(root, &subkey, &value, RRF_RT_REG_SZ, None, None, Some(&mut size)) };
    if err.is_err() || size == 0 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let err = unsafe {
        RegGetValueW(root, &subkey, &value, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size))
    };
    if err.is_err() {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}
