//! Native glass backdrop (Mica / Acrylic) through DWM.
//!
//! The window is created transparent (per-pixel alpha), the DWM frame is
//! extended over the whole client area, and DWM draws the system backdrop
//! behind the egui content. Where Windows can't provide a backdrop, the app
//! falls back to an opaque surface drawn by egui.

use super::window::Hwnd;
use crate::settings::Backdrop;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DWMWINDOWATTRIBUTE,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::UI::Controls::MARGINS;

/// Undocumented attribute used by Windows 11 21H2 (build 22000) for Mica.
const DWMWA_MICA_EFFECT: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(1029);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// Windows 11 22H2+: Mica and Acrylic via `DWMWA_SYSTEMBACKDROP_TYPE`.
    Full,
    /// Windows 11 21H2: Mica only, via an undocumented attribute.
    MicaOnly,
    /// Windows 10: no system backdrop for Win32 windows.
    None,
}

impl Support {
    pub fn for_build(build: u32) -> Self {
        match build {
            22621.. => Support::Full,
            22000.. => Support::MicaOnly,
            _ => Support::None,
        }
    }
}

/// What was actually applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Applied {
    pub glass: bool,
    pub dark: bool,
    pub kind: Option<Backdrop>,
}

fn set_attr<T>(hwnd: Hwnd, attr: DWMWINDOWATTRIBUTE, value: &T) -> windows::core::Result<()> {
    unsafe { DwmSetWindowAttribute(hwnd.get(), attr, value as *const T as *const _, std::mem::size_of::<T>() as u32) }
}

/// Applies (or removes) the backdrop and sets the title bar to light/dark.
/// `requested` is `None` when glass should be off.
pub fn apply(hwnd: Hwnd, support: Support, requested: Option<Backdrop>, dark: bool) -> Applied {
    let dark_flag: i32 = dark.into();
    if let Err(e) = set_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark_flag) {
        log::debug!("dark title bar unavailable: {e}");
    }
    if support != Support::None {
        let _ = set_attr(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUND);
    }

    let kind = match (support, requested) {
        (Support::None, _) | (_, None) => None,
        (Support::MicaOnly, Some(_)) => Some(Backdrop::Mica),
        (Support::Full, Some(k)) => Some(k),
    };

    let margins = if kind.is_some() {
        MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 }
    } else {
        MARGINS::default()
    };
    if let Err(e) = unsafe { DwmExtendFrameIntoClientArea(hwnd.get(), &margins) } {
        log::warn!("DwmExtendFrameIntoClientArea failed: {e}");
        return Applied { glass: false, dark, kind: None };
    }

    let ok = match support {
        Support::Full => {
            let value: DWM_SYSTEMBACKDROP_TYPE = match kind {
                Some(Backdrop::Acrylic) => DWMSBT_TRANSIENTWINDOW,
                Some(Backdrop::Mica) => DWMSBT_MAINWINDOW,
                None => DWMSBT_NONE,
            };
            set_attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &value)
        }
        Support::MicaOnly => {
            let on: i32 = kind.is_some().into();
            set_attr(hwnd, DWMWA_MICA_EFFECT, &on)
        }
        Support::None => Ok(()),
    };
    match ok {
        Ok(()) => Applied { glass: kind.is_some(), dark, kind },
        Err(e) => {
            log::warn!("system backdrop rejected: {e}");
            let _ = unsafe { DwmExtendFrameIntoClientArea(hwnd.get(), &MARGINS::default()) };
            Applied { glass: false, dark, kind: None }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_by_build() {
        assert_eq!(Support::for_build(19045), Support::None);
        assert_eq!(Support::for_build(22000), Support::MicaOnly);
        assert_eq!(Support::for_build(22621), Support::Full);
        assert_eq!(Support::for_build(26200), Support::Full);
    }
}
