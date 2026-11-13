//! Native tray popup with a process-private Polaris Semibold font.
//! Windows still owns navigation, selection, dismissal and menu accessibility.

use super::{fonts::POLARIS_SEMIBOLD, sysprefs::SystemPrefs, window::Hwnd};
use crate::{settings::Accent, ui::theme};
use std::cell::Cell;
use windows::{
    Win32::{
        Foundation::{COLORREF, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::Gdi::*,
        UI::{
            Controls::{
                DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_DISABLED, ODS_GRAYED, ODS_NOACCEL, ODS_SELECTED, ODT_MENU,
            },
            HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI},
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

const SUBCLASS_ID: usize = 0x474d_504f;
const SHOW_MENU: u32 = WM_APP + 73;
pub const OPEN: u32 = 1;
pub const MUTE: u32 = 2;
pub const EXIT: u32 = 3;
const ITEMS: [(u32, &str); 3] = [(OPEN, "&Open Mixer"), (MUTE, "&Mute"), (EXIT, "E&xit")];

fn system_palette() -> theme::Palette {
    let prefs = SystemPrefs::read();
    theme::palette(prefs.dark || prefs.high_contrast, false, Accent::default(), prefs.high_contrast)
}

fn colorref(c: egui::Color32) -> COLORREF {
    COLORREF(u32::from(c.r()) | (u32::from(c.g()) << 8) | (u32::from(c.b()) << 16))
}

fn mnemonic(label: &str) -> Option<char> {
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c == '&' {
            match chars.next() {
                Some('&') => continue,
                Some(key) => return Some(key.to_ascii_lowercase()),
                None => return None,
            }
        }
    }
    None
}

fn menu_char_index(ch: char) -> Option<usize> {
    let target = ch.to_ascii_lowercase();
    let mut pos = 0usize;
    for (id, label) in ITEMS {
        if id == EXIT {
            pos += 1;
        }
        if mnemonic(label) == Some(target) {
            return Some(pos);
        }
        pos += 1;
    }
    None
}

unsafe fn dpi_at_point(hwnd: HWND, pos: POINT) -> u32 {
    let monitor = unsafe { MonitorFromPoint(pos, MONITOR_DEFAULTTONEAREST) };
    let mut dpi_x = 0u32;
    let mut dpi_y = 0u32;
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_ok() && dpi_x >= 96 {
        return dpi_x;
    }
    unsafe { GetDpiForWindow(hwnd) }.max(96)
}

struct State {
    menu: HMENU,
    font: Cell<HFONT>,
    resource: HANDLE,
    dpi: Cell<u32>,
    palette: Cell<theme::Palette>,
    bg_brush: Cell<HBRUSH>,
}

pub struct TrayMenu {
    hwnd: Hwnd,
    state: Box<State>,
    attached: bool,
}

impl TrayMenu {
    pub fn new(hwnd: Hwnd) -> Result<Self, String> {
        unsafe {
            let menu = CreatePopupMenu().map_err(|e| e.to_string())?;
            let mut result = Self {
                hwnd,
                state: Box::new(State {
                    menu,
                    font: Cell::new(HFONT::default()),
                    resource: HANDLE::default(),
                    dpi: Cell::new(96),
                    palette: Cell::new(system_palette()),
                    bg_brush: Cell::new(HBRUSH::default()),
                }),
                attached: false,
            };
            let mut count = 0;
            result.state.resource = AddFontMemResourceEx(
                POLARIS_SEMIBOLD.as_ptr().cast(),
                POLARIS_SEMIBOLD.len() as u32,
                None,
                &raw mut count,
            );
            if result.state.resource.is_invalid() || count == 0 {
                return Err("Couldn't load Polaris Semibold for the tray menu.".into());
            }
            result.state.set_dpi(GetDpiForWindow(hwnd.get()).max(96))?;
            result.state.refresh_theme();
            for (id, label) in ITEMS {
                if id == EXIT {
                    AppendMenuW(menu, MF_SEPARATOR, 0, None).map_err(|e| e.to_string())?;
                }
                let text: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                AppendMenuW(menu, MF_STRING, id as usize, windows::core::PCWSTR(text.as_ptr()))
                    .map_err(|e| e.to_string())?;
                let info = MENUITEMINFOW {
                    cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_FTYPE | MIIM_DATA,
                    fType: MFT_OWNERDRAW,
                    dwItemData: (&*result.state as *const State) as usize,
                    ..Default::default()
                };
                SetMenuItemInfoW(menu, id, false, &info).map_err(|e| e.to_string())?;
            }
            if !SetWindowSubclass(hwnd.get(), Some(subclass), SUBCLASS_ID, (&*result.state as *const State) as usize)
                .as_bool()
            {
                return Err("Couldn't attach the tray menu renderer.".into());
            }
            result.attached = true;
            Ok(result)
        }
    }

    pub fn update(&self, muted: bool, enabled: bool) {
        unsafe {
            CheckMenuItem(self.state.menu, MUTE, (MF_BYCOMMAND | if muted { MF_CHECKED } else { MF_UNCHECKED }).0);
            let _ = EnableMenuItem(self.state.menu, MUTE, MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED });
        }
    }

    /// The event handler runs on the UI thread, as does this window subclass.
    pub fn show(hwnd: Hwnd) -> u32 {
        unsafe { SendMessageW(hwnd.get(), SHOW_MENU, None, None).0 as u32 }
    }
}

impl State {
    unsafe fn refresh_theme(&self) {
        let p = system_palette();
        self.palette.set(p);
        let brush = unsafe { CreateSolidBrush(colorref(p.surface)) };
        if brush.is_invalid() {
            return;
        }
        let info = MENUINFO {
            cbSize: std::mem::size_of::<MENUINFO>() as u32,
            fMask: MIM_BACKGROUND,
            hbrBack: brush,
            ..Default::default()
        };
        if unsafe { SetMenuInfo(self.menu, &info) }.is_err() {
            unsafe {
                let _ = DeleteObject(brush.into());
            }
            return;
        }
        let prev = self.bg_brush.replace(brush);
        if !prev.is_invalid() {
            unsafe {
                let _ = DeleteObject(prev.into());
            }
        }
    }

    unsafe fn set_dpi(&self, dpi: u32) -> Result<(), String> {
        let prev = self.font.get();
        if dpi == self.dpi.get() && !prev.is_invalid() {
            return Ok(());
        }
        let font = unsafe {
            CreateFontW(
                -(14 * dpi as i32 / 96),
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Salted Flutters Polaris"),
            )
        };
        if font.is_invalid() {
            return Err("Couldn't create the Polaris tray font.".into());
        }
        self.font.set(font);
        self.dpi.set(dpi);
        if !prev.is_invalid() {
            unsafe {
                let _ = DeleteObject(prev.into());
            }
        }
        Ok(())
    }

    fn px(&self, logical: i32) -> i32 {
        logical * self.dpi.get() as i32 / 96
    }

    unsafe fn measure(&self, item: &mut MEASUREITEMSTRUCT) {
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            return;
        }
        let old = unsafe { SelectObject(dc, self.font.get().into()) };
        let mut size = SIZE::default();
        let text: Vec<u16> = "Open Mixer".encode_utf16().collect();
        unsafe {
            let _ = GetTextExtentPoint32W(dc, &text, &mut size);
            SelectObject(dc, old);
            let _ = DeleteDC(dc);
        }
        item.itemWidth = (size.cx + self.px(48)) as u32;
        item.itemHeight = size.cy.max(self.px(18)) as u32 + self.px(10) as u32;
    }

    unsafe fn draw(&self, item: &DRAWITEMSTRUCT) {
        let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
        let disabled = (item.itemState.0 & ODS_DISABLED.0 != 0) || (item.itemState.0 & ODS_GRAYED.0 != 0);
        let p = self.palette.get();
        let bg = if selected { if p.high_contrast { p.accent } else { p.surface_hover } } else { p.surface };
        let fg = if disabled {
            p.text_muted
        } else if selected && p.high_contrast {
            p.on_accent
        } else {
            p.text
        };
        let label = ITEMS.iter().find(|(id, _)| *id == item.itemID).map_or("", |(_, label)| *label);
        let mut text: Vec<u16> = label.encode_utf16().collect();
        unsafe {
            let saved = SaveDC(item.hDC);
            if saved == 0 {
                return;
            }
            let brush = CreateSolidBrush(colorref(bg));
            if !brush.is_invalid() {
                FillRect(item.hDC, &item.rcItem, brush);
                let _ = DeleteObject(brush.into());
            }
            SelectObject(item.hDC, self.font.get().into());
            SetBkMode(item.hDC, TRANSPARENT);
            SetTextColor(item.hDC, colorref(fg));
            let mut rect = RECT { left: item.rcItem.left + self.px(30), ..item.rcItem };
            let flags = DT_SINGLELINE
                | DT_VCENTER
                | if item.itemState.0 & ODS_NOACCEL.0 != 0 { DT_HIDEPREFIX } else { DRAW_TEXT_FORMAT(0) };
            DrawTextW(item.hDC, &mut text, &mut rect, flags);
            if item.itemState.0 & windows::Win32::UI::Controls::ODS_CHECKED.0 != 0 {
                let mut check: Vec<u16> = "✓".encode_utf16().collect();
                rect =
                    RECT { left: item.rcItem.left + self.px(8), right: item.rcItem.left + self.px(26), ..item.rcItem };
                DrawTextW(item.hDC, &mut check, &mut rect, DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
            }
            let _ = RestoreDC(item.hDC, saved);
        }
    }
}

unsafe extern "system" fn subclass(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM, _: usize, data: usize) -> LRESULT {
    let state = unsafe { &*(data as *const State) };
    match message {
        SHOW_MENU => unsafe {
            let mut pos = POINT::default();
            let _ = GetCursorPos(&mut pos);
            if let Err(e) = state.set_dpi(dpi_at_point(hwnd, pos)) {
                log::warn!("{e}");
            }
            state.refresh_theme();
            let _ = SetForegroundWindow(hwnd);
            let command = TrackPopupMenu(state.menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pos.x, pos.y, None, hwnd, None);
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            LRESULT(command.0 as isize)
        },
        WM_MEASUREITEM if lp.0 != 0 => unsafe {
            let item = &mut *(lp.0 as *mut MEASUREITEMSTRUCT);
            if item.CtlType == ODT_MENU && item.itemData == data {
                state.measure(item);
                return LRESULT(1);
            }
            DefSubclassProc(hwnd, message, wp, lp)
        },
        WM_DRAWITEM if lp.0 != 0 => unsafe {
            let item = &*(lp.0 as *const DRAWITEMSTRUCT);
            if item.CtlType == ODT_MENU && item.itemData == data {
                state.draw(item);
                return LRESULT(1);
            }
            DefSubclassProc(hwnd, message, wp, lp)
        },
        WM_MENUCHAR if lp.0 == state.menu.0 as isize => {
            if let Some(index) = char::from_u32(wp.0 as u32 & 0xffff).and_then(menu_char_index) {
                let disabled =
                    unsafe { GetMenuState(state.menu, index as u32, MF_BYPOSITION) } & (MF_DISABLED | MF_GRAYED).0 != 0;
                if disabled { LRESULT(0) } else { LRESULT((2 << 16) | index as isize) }
            } else {
                LRESULT(0)
            }
        }
        _ => unsafe { DefSubclassProc(hwnd, message, wp, lp) },
    }
}

impl Drop for TrayMenu {
    fn drop(&mut self) {
        unsafe {
            if self.attached {
                let _ = RemoveWindowSubclass(self.hwnd.get(), Some(subclass), SUBCLASS_ID);
            }
            let _ = DestroyMenu(self.state.menu);
            let brush = self.state.bg_brush.get();
            if !brush.is_invalid() {
                let _ = DeleteObject(brush.into());
            }
            let font = self.state.font.get();
            if !font.is_invalid() {
                let _ = DeleteObject(font.into());
            }
            if !self.state.resource.is_invalid() {
                let _ = RemoveFontMemResourceEx(self.state.resource);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerators_and_colorrefs_derive_from_items_and_palette() {
        assert_eq!(menu_char_index('o'), Some(0));
        assert_eq!(menu_char_index('O'), Some(0));
        assert_eq!(menu_char_index('m'), Some(1));
        assert_eq!(menu_char_index('x'), Some(3));
        assert_eq!(menu_char_index('e'), None);
        assert_eq!(colorref(egui::Color32::from_rgb(0x12, 0x34, 0x56)), COLORREF(0x0056_3412));
    }

    #[test]
    fn native_menu_uses_polaris_semibold_and_keeps_strings_and_mute_state() {
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("menu-test"),
                WINDOW_STYLE(0),
                0,
                0,
                1,
                1,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .unwrap();
            let menu = TrayMenu::new(Hwnd(hwnd.0 as isize)).unwrap();
            assert!(!menu.state.bg_brush.get().is_invalid(), "MIM_BACKGROUND brush must be initialized");
            let dc = CreateCompatibleDC(None);
            let old = SelectObject(dc, menu.state.font.get().into());
            let mut face = [0u16; 64];
            let len = GetTextFaceW(dc, Some(&mut face));
            assert_eq!(String::from_utf16_lossy(&face[..len as usize - 1]), "Salted Flutters Polaris");
            let mut metrics = TEXTMETRICW::default();
            assert!(GetTextMetricsW(dc, &mut metrics).as_bool());
            assert_eq!(metrics.tmWeight, 600, "the selected GDI face is actually Semibold");
            SelectObject(dc, old);
            let _ = DeleteDC(dc);
            assert_eq!(GetMenuItemCount(Some(menu.state.menu)), 4);
            let mut text = [0u16; 40];
            let len = GetMenuStringW(menu.state.menu, OPEN, Some(&mut text), MF_BYCOMMAND);
            assert_eq!(
                String::from_utf16_lossy(&text[..len as usize]),
                "&Open Mixer",
                "accessible strings remain attached to owner-drawn items"
            );
            menu.update(true, false);
            let flags = GetMenuState(menu.state.menu, MUTE, MF_BYCOMMAND);
            assert_ne!(flags & MF_CHECKED.0, 0);
            assert_ne!(flags & MF_GRAYED.0, 0);
            assert_eq!(
                SendMessageW(hwnd, WM_MENUCHAR, Some(WPARAM('m' as usize)), Some(LPARAM(menu.state.menu.0 as isize))).0,
                0
            );
            menu.update(false, true);
            let flags = GetMenuState(menu.state.menu, MUTE, MF_BYCOMMAND);
            assert_eq!(flags & (MF_CHECKED | MF_GRAYED).0, 0);
            assert_eq!(
                SendMessageW(hwnd, WM_MENUCHAR, Some(WPARAM('m' as usize)), Some(LPARAM(menu.state.menu.0 as isize))).0,
                (2 << 16) | 1
            );
            let mut normal = MEASUREITEMSTRUCT::default();
            menu.state.measure(&mut normal);
            menu.state.set_dpi(192).unwrap();
            let mut scaled = MEASUREITEMSTRUCT::default();
            menu.state.measure(&mut scaled);
            assert!(scaled.itemWidth > normal.itemWidth && scaled.itemHeight > normal.itemHeight);
            drop(menu);
            DestroyWindow(hwnd).unwrap();
        }
    }
}
