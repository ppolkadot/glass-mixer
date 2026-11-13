//! Native glass backdrop (Mica / Acrylic) through DWM.
//!
//! The window is created transparent (per-pixel alpha), the DWM frame is
//! extended over the whole client area, and DWM draws the system backdrop
//! behind the egui content. Where Windows can't provide a backdrop, the app
//! falls back to an opaque surface drawn by egui.

use super::window::Hwnd;
use crate::settings::Backdrop;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_BORDER_COLOR,
    DWMWA_COLOR_NONE, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND, DWMWINDOWATTRIBUTE, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
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

pub(crate) fn border_colorref(p: &crate::ui::theme::Palette) -> u32 {
    if !p.glass || p.high_contrast {
        return DWMWA_COLOR_NONE;
    }
    let [fr, fg, fb, fa] = p.border.to_srgba_unmultiplied();
    let [br, bg, bb, _] = p.base.to_srgba_unmultiplied();
    let a = u32::from(fa);
    let inv = 255 - a;
    let blend = |f: u8, b: u8| (u32::from(f) * a + u32::from(b) * inv + 127) / 255;
    let r = blend(fr, br);
    let g = blend(fg, bg);
    let b = blend(fb, bb);
    r | (g << 8) | (b << 16)
}

/// Sets the Windows 11 window border colour to match the current palette.
pub fn set_border(hwnd: Hwnd, support: Support, p: &crate::ui::theme::Palette) {
    if support == Support::None {
        return;
    }
    let color = border_colorref(p);
    if let Err(e) = set_attr(hwnd, DWMWA_BORDER_COLOR, &color) {
        log::debug!("DWMWA_BORDER_COLOR unavailable: {e}");
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

    #[test]
    fn border_colorref_blends_glass_border_and_uses_none_for_solid_or_high_contrast() {
        use crate::settings::Accent;
        use crate::ui::theme::palette;

        assert_eq!(border_colorref(&palette(true, false, Accent::Violet, false)), DWMWA_COLOR_NONE);
        assert_eq!(border_colorref(&palette(false, true, Accent::Violet, true)), DWMWA_COLOR_NONE);

        let dark_glass = palette(true, true, Accent::Violet, false);
        let dark_color = border_colorref(&dark_glass);
        assert_ne!(dark_color, DWMWA_COLOR_NONE);
        let light_glass = palette(false, true, Accent::Violet, false);
        let light_color = border_colorref(&light_glass);
        assert_ne!(light_color, DWMWA_COLOR_NONE);
        assert_ne!(dark_color, light_color);
        assert_eq!(dark_color & 0xFF00_0000, 0, "COLORREF upper byte must be zero");
    }
}
