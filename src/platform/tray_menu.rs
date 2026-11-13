//! Native tray popup with a process-private Polaris Semibold font.
//! Windows still owns navigation, selection, dismissal and menu accessibility.

use super::{fonts::POLARIS_SEMIBOLD, window::Hwnd};
use windows::{
    Win32::{
        Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::Gdi::*,
        UI::{
            Controls::{
                DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_DISABLED, ODS_GRAYED, ODS_NOACCEL, ODS_SELECTED, ODT_MENU,
            },
            HiDpi::GetDpiForWindow,
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

struct State {
    menu: HMENU,
    font: HFONT,
    resource: HANDLE,
    dpi: u32,
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
                state: Box::new(State { menu, font: HFONT::default(), resource: HANDLE::default(), dpi: 96 }),
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
    unsafe fn set_dpi(&mut self, dpi: u32) -> Result<(), String> {
        if dpi == self.dpi && !self.font.is_invalid() {
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
        if !self.font.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.font.into());
            }
        }
        self.font = font;
        self.dpi = dpi;
        Ok(())
    }

    fn px(&self, logical: i32) -> i32 {
        logical * self.dpi as i32 / 96
    }

    unsafe fn measure(&self, item: &mut MEASUREITEMSTRUCT) {
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            return;
        }
        let old = unsafe { SelectObject(dc, self.font.into()) };
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
        let background = if selected { COLOR_HIGHLIGHT } else { COLOR_MENU };
        let foreground = if disabled {
            COLOR_GRAYTEXT
        } else if selected {
            COLOR_HIGHLIGHTTEXT
        } else {
            COLOR_MENUTEXT
        };
        let label = ITEMS.iter().find(|(id, _)| *id == item.itemID).map_or("", |(_, label)| *label);
        let mut text: Vec<u16> = label.encode_utf16().collect();
        unsafe {
            let saved = SaveDC(item.hDC);
            if saved == 0 {
                return;
            }
            FillRect(item.hDC, &item.rcItem, GetSysColorBrush(background));
            SelectObject(item.hDC, self.font.into());
            SetBkMode(item.hDC, TRANSPARENT);
            SetTextColor(item.hDC, windows::Win32::Foundation::COLORREF(GetSysColor(foreground)));
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
    let state = unsafe { &mut *(data as *mut State) };
    match message {
        SHOW_MENU => unsafe {
            if let Err(e) = state.set_dpi(GetDpiForWindow(hwnd).max(96)) {
                log::warn!("{e}");
            }
            let mut pos = POINT::default();
            let _ = GetCursorPos(&mut pos);
            let _ = SetForegroundWindow(hwnd);
            let menu = state.menu;
            // Do not keep a mutable borrow across the nested menu message loop.
            let command = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pos.x, pos.y, None, hwnd, None);
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
            let index = match char::from_u32(wp.0 as u32 & 0xffff).map(|c| c.to_ascii_lowercase()) {
                Some('o') => Some(0),
                Some('m') => Some(1),
                Some('x') => Some(3),
                _ => None,
            };
            if let Some(index) = index {
                let disabled =
                    unsafe { GetMenuState(state.menu, index as u32, MF_BYPOSITION) } & (MF_DISABLED | MF_GRAYED).0 != 0;
                if disabled { LRESULT(0) } else { LRESULT((2 << 16) | index) }
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
            if !self.state.font.is_invalid() {
                let _ = DeleteObject(self.state.font.into());
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
            let mut menu = TrayMenu::new(Hwnd(hwnd.0 as isize)).unwrap();
            let dc = CreateCompatibleDC(None);
            let old = SelectObject(dc, menu.state.font.into());
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
