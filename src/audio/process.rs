//! Process identity (executable path, package identity, file description) and
//! event-driven process-exit watching for audio session owners.

use super::Msg;
use crossbeam_channel::Sender;
use std::{
    ffi::c_void,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Threading::{
    INFINITE, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    QueryFullProcessImageNameW, RegisterWaitForSingleObject, UnregisterWaitEx, WT_EXECUTEONLYONCE,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

#[derive(Clone, Debug, Default)]
pub struct ProcessInfo {
    pub exe_path: Option<PathBuf>,
    /// Application User Model ID for packaged apps.
    pub aumid: Option<String>,
    /// `FileDescription` from the executable's version resource.
    pub description: Option<String>,
}

/// An open process handle; closed on drop.
pub struct ProcessHandle(HANDLE);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Opens a process with the minimal rights needed. Elevated or protected
/// processes may refuse even limited access; callers fall back gracefully.
pub fn open(pid: u32) -> Option<ProcessHandle> {
    unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, false, pid)
            .or_else(|_| OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid))
            .ok()
            .map(ProcessHandle)
    }
}

pub fn query(handle: &ProcessHandle) -> ProcessInfo {
    let exe_path = image_path(handle.0);
    let aumid = app_user_model_id(handle.0);
    let description = exe_path.as_ref().and_then(|p| file_description(p));
    ProcessInfo { exe_path, aumid, description }
}

fn image_path(process: HANDLE) -> Option<PathBuf> {
    let mut buf = vec![0u16; 32768];
    let mut len = buf.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).ok()?;
    }
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

fn app_user_model_id(process: HANDLE) -> Option<String> {
    let mut len = 0u32;
    // First call reports the required length (APPMODEL_ERROR_NO_APPLICATION for unpackaged).
    let _ = unsafe { GetApplicationUserModelId(process, &mut len, None) };
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    let err = unsafe { GetApplicationUserModelId(process, &mut len, Some(PWSTR(buf.as_mut_ptr()))) };
    if err != ERROR_SUCCESS {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1)]);
    (!s.is_empty()).then_some(s)
}

/// Reads `FileDescription` from the version resource, trying the file's own
/// translations first and then US English.
pub fn file_description(path: &std::path::Path) -> Option<String> {
    let wide = HSTRING::from(path.as_os_str());
    let size = unsafe { GetFileVersionInfoSizeW(&wide, None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(&wide, None, size, data.as_mut_ptr() as *mut c_void).ok()? };

    let mut candidates = Vec::new();
    let mut ptr: *mut c_void = std::ptr::null_mut();
    let mut len = 0u32;
    let ok = unsafe {
        VerQueryValueW(data.as_ptr() as *const c_void, &HSTRING::from(r"\VarFileInfo\Translation"), &mut ptr, &mut len)
    };
    if ok.as_bool() && !ptr.is_null() {
        let pairs = unsafe { std::slice::from_raw_parts(ptr as *const u16, len as usize / 2) };
        for pair in pairs.as_chunks::<2>().0 {
            candidates.push(format!("{:04x}{:04x}", pair[0], pair[1]));
        }
    }
    candidates.extend(["040904b0".to_string(), "040904e4".to_string()]);

    for lang in candidates {
        let key = HSTRING::from(format!(r"\StringFileInfo\{lang}\FileDescription"));
        let mut value: *mut c_void = std::ptr::null_mut();
        let mut chars = 0u32;
        let ok = unsafe { VerQueryValueW(data.as_ptr() as *const c_void, &key, &mut value, &mut chars) };
        if ok.as_bool() && !value.is_null() && chars > 1 {
            let s = unsafe { PCWSTR(value as *const u16).to_string() }.unwrap_or_default();
            let s = s.trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

struct WatchContext {
    tx: Sender<Msg>,
    exited: Arc<AtomicBool>,
}

/// Notifies the engine when a process exits, without polling. Uses a thread-pool
/// wait (`RegisterWaitForSingleObject`) on the process handle.
pub struct ProcessWatch {
    process: ProcessHandle,
    wait: HANDLE,
    context: *mut WatchContext,
    exited: Arc<AtomicBool>,
}

impl ProcessWatch {
    pub fn new(process: ProcessHandle, tx: Sender<Msg>) -> Option<Self> {
        let exited = Arc::new(AtomicBool::new(false));
        let context = Box::into_raw(Box::new(WatchContext { tx, exited: exited.clone() }));
        let mut wait = HANDLE::default();
        let registered = unsafe {
            RegisterWaitForSingleObject(
                &mut wait,
                process.0,
                Some(on_process_exit),
                Some(context as *const c_void),
                INFINITE,
                WT_EXECUTEONLYONCE,
            )
        };
        if registered.is_err() {
            // Without SYNCHRONIZE access the wait cannot be registered.
            drop(unsafe { Box::from_raw(context) });
            return None;
        }
        Some(Self { process, wait, context, exited })
    }

    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::Relaxed)
    }
}

unsafe extern "system" fn on_process_exit(context: *mut c_void, _timed_out: bool) {
    let ctx = unsafe { &*(context as *const WatchContext) };
    ctx.exited.store(true, Ordering::Relaxed);
    let _ = ctx.tx.send(Msg::SessionsChanged);
}

impl Drop for ProcessWatch {
    fn drop(&mut self) {
        unsafe {
            // INVALID_HANDLE_VALUE: block until any in-flight callback has returned,
            // so the context can be freed safely.
            let _ = UnregisterWaitEx(self.wait, Some(INVALID_HANDLE_VALUE));
            drop(Box::from_raw(self.context));
        }
        let _ = &self.process;
    }
}
