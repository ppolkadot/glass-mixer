//! Small COM helpers shared by the audio and platform modules.

use windows::Win32::System::Com::{
    COINIT, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree,
    CoUninitialize,
};
use windows::core::{HRESULT, PWSTR};

/// Initializes COM on the current thread for its lifetime.
pub struct ComGuard(bool);

impl ComGuard {
    pub fn multithreaded() -> Self {
        Self::init(COINIT_MULTITHREADED)
    }

    pub fn apartment() -> Self {
        Self::init(COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
    }

    fn init(model: COINIT) -> Self {
        let hr = unsafe { CoInitializeEx(None, model) };
        if hr.is_err() {
            log::error!("CoInitializeEx failed: {hr:?}");
        }
        // S_FALSE (already initialized) still needs a balancing CoUninitialize.
        Self(hr.is_ok())
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// Converts a COM-allocated string into a `String` and frees it.
pub fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

pub const AUDCLNT_E_DEVICE_INVALIDATED: HRESULT = HRESULT(0x8889_0004_u32 as i32);
pub const AUDCLNT_E_SERVICE_NOT_RUNNING: HRESULT = HRESULT(0x8889_0010_u32 as i32);
const RPC_E_DISCONNECTED: HRESULT = HRESULT(0x8001_0108_u32 as i32);
const RPC_S_SERVER_UNAVAILABLE: HRESULT = HRESULT(0x8007_06BA_u32 as i32);
const RPC_S_CALL_FAILED: HRESULT = HRESULT(0x8007_06BE_u32 as i32);
const E_NOTFOUND: HRESULT = HRESULT(0x8007_0490_u32 as i32);

/// Errors after which every Core Audio object must be recreated
/// (device invalidated, audio service restarted, ...).
pub fn is_fatal(code: HRESULT) -> bool {
    matches!(
        code,
        AUDCLNT_E_DEVICE_INVALIDATED
            | AUDCLNT_E_SERVICE_NOT_RUNNING
            | RPC_E_DISCONNECTED
            | RPC_S_SERVER_UNAVAILABLE
            | RPC_S_CALL_FAILED
    )
}

pub fn is_not_found(code: HRESULT) -> bool {
    code == E_NOTFOUND
}
