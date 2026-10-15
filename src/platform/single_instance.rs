//! Single-instance guard based on a session-local named mutex.

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::w;

pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Returns `None` when another instance already holds the mutex in this logon session.
pub fn acquire() -> Option<InstanceGuard> {
    unsafe {
        match CreateMutexW(None, false, w!("Local\\GlassMixer.SingleInstance")) {
            Ok(handle) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    let _ = CloseHandle(handle);
                    None
                } else {
                    Some(InstanceGuard(handle))
                }
            }
            // If the mutex can't be created at all, don't block startup.
            Err(e) => {
                log::warn!("single-instance mutex unavailable: {e}");
                Some(InstanceGuard(HANDLE::default()))
            }
        }
    }
}
