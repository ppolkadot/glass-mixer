//! Launch at Windows login via the per-user `Run` registry key (no admin needed).
//!
//! The registry is the single source of truth: the setting is not duplicated in
//! `settings.json`, so it can't drift if the user changes it in Task Manager.

use super::reg_string;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_BINARY, REG_SZ, RRF_RT_REG_BINARY, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::HSTRING;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const VALUE_NAME: &str = "GlassMixer";
pub const MINIMIZED_ARG: &str = "--minimized";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginState {
    Disabled,
    Enabled,
    /// Registered, but disabled in Task Manager's Startup apps page.
    BlockedByUser,
}

fn command_line() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" {MINIMIZED_ARG}", exe.display()))
}

pub fn state() -> LoginState {
    if reg_string(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME).is_none() {
        return LoginState::Disabled;
    }
    // StartupApproved: first byte 0x02/0x06 = enabled, odd values = disabled. Missing = enabled.
    let mut data = [0u8; 12];
    let mut size = data.len() as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(APPROVED_KEY),
            &HSTRING::from(VALUE_NAME),
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if err.is_ok() && size > 0 && data[0] & 1 == 1 { LoginState::BlockedByUser } else { LoginState::Enabled }
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let run_key = HSTRING::from(RUN_KEY);
    let name = HSTRING::from(VALUE_NAME);
    if enabled {
        let cmd = command_line().ok_or("Couldn't determine the application path.")?;
        let wide: Vec<u16> = cmd.encode_utf16().chain(Some(0)).collect();
        let err = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &run_key,
                &name,
                REG_SZ.0,
                Some(wide.as_ptr() as *const _),
                (wide.len() * 2) as u32,
            )
        };
        err.ok().map_err(|e| format!("Couldn't enable launch at login: {}", e.message()))?;
        // Clear a previous "disabled in Task Manager" flag, since the user explicitly opted in.
        let approved = [2u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let _ = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(APPROVED_KEY),
                &name,
                REG_BINARY.0,
                Some(approved.as_ptr() as *const _),
                approved.len() as u32,
            )
        };
        log::info!("launch at login enabled: {cmd}");
    } else {
        let err = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &run_key, &name) };
        if err.is_err() && state() != LoginState::Disabled {
            return Err(format!("Couldn't disable launch at login: {}", err.to_hresult().message()));
        }
        log::info!("launch at login disabled");
    }
    Ok(())
}

/// If launch at login is enabled but points at an old location (the exe was
/// moved or updated), re-register the current path.
pub fn repair_path_if_enabled() {
    let Some(current) = reg_string(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME) else { return };
    if command_line().is_some_and(|cmd| cmd != current) {
        let blocked = state() == LoginState::BlockedByUser;
        if !blocked {
            let _ = set_enabled(true);
        }
    }
}
