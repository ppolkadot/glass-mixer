//! Windows appearance and accessibility preferences.

use super::{reg_dword, reg_string};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::BOOL;

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemPrefs {
    /// Apps use dark mode.
    pub dark: bool,
    /// "Transparency effects" is on.
    pub transparency: bool,
    /// "Animation effects" is on (reduced motion when off).
    pub animations: bool,
    pub high_contrast: bool,
    /// Windows build number (e.g. 22631).
    pub build: u32,
}

impl SystemPrefs {
    pub fn read() -> Self {
        let dark = reg_dword(HKEY_CURRENT_USER, PERSONALIZE, "AppsUseLightTheme").is_some_and(|v| v == 0);
        let transparency = reg_dword(HKEY_CURRENT_USER, PERSONALIZE, "EnableTransparency").is_none_or(|v| v != 0);

        let mut animations = BOOL(1);
        unsafe {
            let _ = SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some(&mut animations as *mut BOOL as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
        }

        let mut hc = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
        unsafe {
            let _ = SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                hc.cbSize,
                Some(&mut hc as *mut HIGHCONTRASTW as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
        }

        let build =
            reg_string(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);

        Self {
            dark,
            transparency,
            animations: animations.as_bool(),
            high_contrast: (hc.dwFlags & HCF_HIGHCONTRASTON) == HCF_HIGHCONTRASTON,
            build,
        }
    }
}
