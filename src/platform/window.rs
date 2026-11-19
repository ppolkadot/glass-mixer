//! Direct Win32 window management for the main window.
//!
//! Showing the window from the tray goes through Win32 directly rather than
//! egui viewport commands: it is immediate even while eframe throttles a hidden
//! window, and `SetForegroundWindow` must run inside the tray click handler to be
//! allowed by Windows' focus-stealing rules.

use std::ffi::c_void;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, HANDLE, HWND, LPARAM, RECT,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_HIDE,
    SW_RESTORE, SW_SHOW, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow,
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

/// Physical screen coordinates, independent of the window's current DPI.
pub fn outer_position(hwnd: Hwnd) -> Option<(f32, f32)> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd.get(), &mut rect) }.ok()?;
    Some((rect.left as f32, rect.top as f32))
}

pub fn set_outer_position(hwnd: Hwnd, x: i32, y: i32) -> windows::core::Result<()> {
    unsafe { SetWindowPos(hwnd.get(), None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) }
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

fn process_image_path(pid: u32) -> windows::core::Result<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?;
        let result = query_process_image_path(h);
        let _ = CloseHandle(h);
        result
    }
}

/// Query the same process handle that will be waited on or terminated, so PID
/// reuse between discovery and opening cannot target another executable.
pub(super) fn query_process_image_path(handle: HANDLE) -> windows::core::Result<String> {
    let mut buf = vec![0u16; 32_768];
    let mut len = buf.len() as u32;
    unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)? };
    Ok(String::from_utf16_lossy(&buf[..len as usize]))
}

pub(super) fn same_executable(image: &str, exe: &std::path::Path) -> bool {
    fn normalized(path: &str) -> String {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_lowercase()
    }
    normalized(image) == normalized(&exe.to_string_lossy())
}

fn process_image_name(pid: u32) -> Option<String> {
    let path = process_image_path(pid).ok()?;
    std::path::Path::new(&path).file_name().map(|f| f.to_string_lossy().to_lowercase())
}

/// Process IDs of other running copies started from exactly `exe` (compared
/// without regard to case). Discovery works before window creation and after
/// window destruction. An inaccessible candidate is reported, not silently skipped.
pub fn instance_pids(exe: &std::path::Path) -> Result<Vec<u32>, String> {
    let name = exe.file_name().ok_or("the executable has no filename")?.to_string_lossy().to_lowercase();
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|e| format!("couldn't enumerate processes: {e}"))?;
    let result = (|| {
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut next = unsafe { Process32FirstW(snapshot, &mut entry) };
        let mut found = Vec::new();
        loop {
            if let Err(e) = next {
                return if e.code() == ERROR_NO_MORE_FILES.to_hresult() {
                    Ok(found)
                } else {
                    Err(format!("couldn't enumerate processes: {e}"))
                };
            }
            let len = entry.szExeFile.iter().position(|&unit| unit == 0).unwrap_or(entry.szExeFile.len());
            if entry.th32ProcessID != std::process::id()
                && String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase() == name
            {
                match process_image_path(entry.th32ProcessID) {
                    Ok(image) if same_executable(&image, exe) => found.push(entry.th32ProcessID),
                    Ok(_) => {}
                    // The process may have exited since the snapshot was taken.
                    Err(e) if e.code() == ERROR_INVALID_PARAMETER.to_hresult() => {}
                    Err(e) => return Err(format!("couldn't identify candidate process {}: {e}", entry.th32ProcessID)),
                }
            }
            next = unsafe { Process32NextW(snapshot, &mut entry) };
        }
    })();
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    result
}

/// Opens a folder in File Explorer. `explorer.exe` is started by its full path
/// because Windows looks for a bare program name in the application's folder first.
pub fn open_folder(dir: &std::path::Path) -> Result<(), String> {
    let explorer = explorer_path().ok_or("Couldn't find the Windows folder.")?;
    std::process::Command::new(explorer).arg(dir).spawn().map(drop).map_err(|e| e.to_string())
}

fn explorer_path() -> Option<std::path::PathBuf> {
    use windows::Win32::UI::Shell::{FOLDERID_Windows, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    let windows_dir = unsafe { SHGetKnownFolderPath(&FOLDERID_Windows, KF_FLAG_DEFAULT, None) }.ok()?;
    let windows_dir = crate::audio::com::take_pwstr(windows_dir);
    (!windows_dir.is_empty()).then(|| std::path::Path::new(&windows_dir).join("explorer.exe"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_is_resolved_to_an_absolute_path_in_the_windows_folder() {
        let path = explorer_path().expect("the Windows folder is known");
        assert!(path.is_absolute(), "{path:?}");
        assert!(path.ends_with("explorer.exe"));
        assert!(path.is_file(), "{path:?} should exist");
    }

    #[test]
    fn no_other_copy_of_the_test_binary_has_a_mixer_window() {
        assert!(instance_pids(&std::env::current_exe().unwrap()).unwrap().is_empty());
    }
}
