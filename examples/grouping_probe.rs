//! Read-only: lists the default output's audio sessions with their grouping
//! parameter, to see how Windows groups the streams of multi-process apps.
//! Changes no volume, mute or device. Prints executable file names, not paths.
//! cargo run --example grouping_probe

use std::collections::HashMap;
use windows::{
    Win32::{
        Foundation::{CloseHandle, S_OK},
        Media::Audio::{
            AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2,
            IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize},
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    },
    core::{GUID, Interface, PWSTR, Result},
};

struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn exe_name(pid: u32) -> String {
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return "?".into();
    };
    let mut buf = vec![0u16; 32768];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) };
    let _ = unsafe { CloseHandle(handle) };
    if ok.is_err() {
        return "?".into();
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    path.rsplit('\\').next().unwrap_or_default().to_string()
}

fn take(p: PWSTR) -> String {
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

fn main() -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    let _com = Com;
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? };
    let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None)? };
    let sessions = unsafe { manager.GetSessionEnumerator()? };
    let count = unsafe { sessions.GetCount()? };

    let mut by_group: HashMap<GUID, Vec<String>> = HashMap::new();
    println!("{:<8} {:<26} {:<8} {:<38} instance", "pid", "exe", "state", "grouping");
    for i in 0..count {
        let control: IAudioSessionControl2 = unsafe { sessions.GetSession(i)? }.cast()?;
        let state = match unsafe { control.GetState()? } {
            s if s == AudioSessionStateActive => "active",
            s if s == AudioSessionStateExpired => "expired",
            _ => "inactive",
        };
        let pid = unsafe { control.GetProcessId() }.unwrap_or(0);
        let exe =
            if unsafe { control.IsSystemSoundsSession() } == S_OK { "(system sounds)".into() } else { exe_name(pid) };
        let grouping = unsafe { control.GetGroupingParam() }.unwrap_or_default();
        // The instance identifier ends with a per-session suffix; only that part is printed.
        let instance = take(unsafe { control.GetSessionInstanceIdentifier()? });
        let suffix = instance.rsplit('%').next().unwrap_or_default().to_string();
        println!("{pid:<8} {exe:<26} {state:<8} {grouping:<38?} …{suffix}");
        by_group.entry(grouping).or_default().push(format!("{exe}:{pid}"));
    }

    println!("\nsessions sharing a grouping parameter:");
    for (grouping, members) in by_group.iter().filter(|(_, m)| m.len() > 1) {
        println!("  {grouping:?}: {}", members.join(", "));
    }
    Ok(())
}
