//! Single-instance guard based on a session-local named mutex.

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::HSTRING;

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
    let name = mutex_name();
    unsafe {
        match CreateMutexW(None, false, &HSTRING::from(name)) {
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

fn mutex_name() -> String {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("GLASS_MIXER_DEV_PROFILE")
        && std::path::Path::new(&dir).is_absolute()
    {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        dir.to_string_lossy().to_lowercase().hash(&mut hash);
        return format!("Local\\GlassMixer.Dev.{:016X}", hash.finish());
    }
    "Local\\GlassMixer.SingleInstance".into()
}
