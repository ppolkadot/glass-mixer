//! Read-only: checks that the per-app device interface can be created and lists
//! the device preference Windows holds for each app with a session on the
//! default output. Changes nothing. Prints executable file names, not paths.
//! cargo run --example routing_probe

use glass_mixer::audio::{Flow, app_routing::AppRouting};
use windows::{
    Win32::{
        Foundation::{CloseHandle, S_OK},
        Media::Audio::{
            AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2,
            IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize},
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    },
    core::{Interface, PWSTR, Result},
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

fn main() -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    let _com = Com;
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? };
    let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None)? };
    let sessions = unsafe { manager.GetSessionEnumerator()? };
    let count = unsafe { sessions.GetCount()? };

    let routing = match AppRouting::create() {
        Ok(routing) => {
            println!("per-app routing interface: available");
            Some(routing)
        }
        Err(e) => {
            println!("per-app routing interface: unavailable ({e})");
            None
        }
    };
    println!("{:<8} {:<26} {:<8} preference", "pid", "exe", "state");
    for i in 0..count {
        let control: IAudioSessionControl2 = unsafe { sessions.GetSession(i)? }.cast()?;
        let state = match unsafe { control.GetState()? } {
            s if s == AudioSessionStateActive => "active",
            s if s == AudioSessionStateExpired => "expired",
            _ => "inactive",
        };
        let pid = unsafe { control.GetProcessId() }.unwrap_or(0);
        if unsafe { control.IsSystemSoundsSession() } == S_OK {
            continue;
        }
        let preference = match routing.as_ref().map(|r| r.get(pid, Flow::Output)) {
            Some(Ok(Some(id))) => format!("device {}…", id.chars().take(24).collect::<String>()),
            Some(Ok(None)) => "system default".into(),
            Some(Err(e)) => format!("error {e}"),
            None => "-".into(),
        };
        println!("{pid:<8} {:<26} {state:<8} {preference}", exe_name(pid));
    }
    Ok(())
}
