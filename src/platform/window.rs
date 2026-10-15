//! Direct Win32 window management for the main window.
//!
//! Showing the window from the tray goes through Win32 directly rather than
//! egui viewport commands: it is immediate even while eframe throttles a hidden
//! window, and `SetForegroundWindow` must run inside the tray click handler to be
//! allowed by Windows' focus-stealing rules.

use std::ffi::c_void;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_HIDE,
    SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
};
use windows::core::{BOOL, PWSTR};

pub const WINDOW_TITLE: &str = "Glass Mixer";

/// A window handle that can be shared across threads (it is just an identifier).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hwnd(pub isize);

impl Hwnd {
    pub fn get(self) -> HWND {
        HWND(self.0 as *mut c_void)
    }
}

pub fn show(hwnd: Hwnd) {
    unsafe {
        let h = hwnd.get();
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        } else {
            let _ = ShowWindow(h, SW_SHOW);
        }
        let _ = SetForegroundWindow(h);
    }
}

pub fn hide(hwnd: Hwnd) {
    unsafe {
        let _ = ShowWindow(hwnd.get(), SW_HIDE);
    }
}

/// Cloaking keeps a window "visible" to Windows but undrawn. Used to start hidden
/// without a flash, because eframe always shows the window after its first frame.
pub fn set_cloaked(hwnd: Hwnd, cloaked: bool) {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
    let value: windows::core::BOOL = cloaked.into();
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd.get(), DWMWA_CLOAK, &value as *const _ as *const _, 4);
    }
}

pub fn is_visible(hwnd: Hwnd) -> bool {
    unsafe { IsWindowVisible(hwnd.get()).as_bool() }
}

/// Visible and not minimized.
pub fn is_shown(hwnd: Hwnd) -> bool {
    unsafe { IsWindowVisible(hwnd.get()).as_bool() && !IsIconic(hwnd.get()).as_bool() }
}

pub fn is_foreground(hwnd: Hwnd) -> bool {
    unsafe { GetForegroundWindow() == hwnd.get() }
}

fn process_image_name(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = windows::Win32::Foundation::CloseHandle(h);
        ok.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        std::path::Path::new(&path).file_name().map(|f| f.to_string_lossy().to_lowercase())
    }
}

/// Finds the main window of an already running Glass Mixer (same executable
/// name and window title) and brings it to the foreground.
pub fn activate_existing_instance() -> bool {
    struct Search {
        exe: Option<String>,
        own_pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let mut title = [0u16; 64];
        let n = unsafe { GetWindowTextW(hwnd, &mut title) } as usize;
        if String::from_utf16_lossy(&title[..n]) != WINDOW_TITLE {
            return true.into();
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid != search.own_pid && process_image_name(pid) == search.exe {
            search.found = Some(hwnd);
            return false.into();
        }
        true.into()
    }

    let exe = std::env::current_exe().ok().and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_lowercase()));
    let mut search = Search { exe, own_pid: std::process::id(), found: None };
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize));
    }
    match search.found {
        Some(h) => {
            show(Hwnd(h.0 as isize));
            true
        }
        None => false,
    }
}
