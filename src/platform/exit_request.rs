//! Lets an uninstaller ask a running copy to exit cleanly: `glass-mixer.exe --exit`.
//!
//! Each running copy waits on a named event derived from its executable path.
//! `--exit` signals the event for its own path and waits for that copy's normal
//! shutdown, terminating it only if it doesn't exit in time. Copies started from
//! other folders, such as a portable copy, are not affected.

use std::ffi::c_void;
use std::path::Path;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::Threading::{
    CreateEventW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, RegisterWaitForSingleObject, ResetEvent, SetEvent, TerminateProcess,
    UnregisterWaitEx, WT_EXECUTEONLYONCE, WaitForSingleObject,
};
use windows::core::HSTRING;

pub const EXIT_ARG: &str = "--exit";

/// How long `--exit` waits for a clean shutdown before terminating the copy.
pub const EXIT_WAIT: Duration = Duration::from_secs(10);

/// `Local\GlassMixer.Exit.<hash>`: one event per executable path and logon session.
fn event_name(exe: &Path) -> String {
    // FNV-1a over the lower-cased path; Windows paths are case-insensitive.
    let hash = exe
        .to_string_lossy()
        .to_lowercase()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3));
    format!("Local\\GlassMixer.Exit.{hash:016X}")
}

struct Context {
    on_request: Box<dyn Fn() + Send + Sync>,
}

/// Calls `on_request` (on a thread-pool thread) when `--exit` is run for this
/// executable. Dropping it stops listening.
pub struct ExitListener {
    event: HANDLE,
    wait: HANDLE,
    context: *mut Context,
}

impl ExitListener {
    pub fn start(on_request: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        Self::start_for(&exe, on_request)
    }

    fn start_for(exe: &Path, on_request: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let name = HSTRING::from(event_name(exe));
        let event = match unsafe { CreateEventW(None, true, false, &name) } {
            Ok(event) => event,
            Err(e) => {
                log::warn!("exit requests unavailable: {e}");
                return None;
            }
        };
        // A request left over from an earlier copy must not stop this one.
        let _ = unsafe { ResetEvent(event) };
        let context = Box::into_raw(Box::new(Context { on_request: Box::new(on_request) }));
        let mut wait = HANDLE::default();
        let registered = unsafe {
            RegisterWaitForSingleObject(
                &mut wait,
                event,
                Some(on_signal),
                Some(context as *const c_void),
                INFINITE,
                WT_EXECUTEONLYONCE,
            )
        };
        if let Err(e) = registered {
            log::warn!("exit requests unavailable: {e}");
            unsafe {
                drop(Box::from_raw(context));
                let _ = CloseHandle(event);
            }
            return None;
        }
        Some(Self { event, wait, context })
    }
}

unsafe extern "system" fn on_signal(context: *mut c_void, _timed_out: bool) {
    let context = unsafe { &*(context as *const Context) };
    log::info!("exit requested by another process");
    (context.on_request)();
}

impl Drop for ExitListener {
    fn drop(&mut self) {
        unsafe {
            // INVALID_HANDLE_VALUE: wait for an in-flight callback before freeing its context.
            let _ = UnregisterWaitEx(self.wait, Some(INVALID_HANDLE_VALUE));
            drop(Box::from_raw(self.context));
            let _ = CloseHandle(self.event);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExitOutcome {
    /// No copy from this executable path was running.
    NotRunning,
    /// Every copy finished its normal shutdown.
    Exited,
    /// A copy did not exit within the wait and was terminated.
    Terminated,
    /// A copy is still running and could not be stopped.
    StillRunning,
}

/// Asks the copy running from this executable's path to exit and waits for it.
pub fn request_exit_and_wait(wait: Duration) -> ExitOutcome {
    match std::env::current_exe() {
        Ok(exe) => match super::window::instance_pids(&exe) {
            Ok(pids) => request_exit_for(&exe, wait, &pids),
            Err(e) => {
                log::error!("--exit: {e}");
                ExitOutcome::StillRunning
            }
        },
        Err(e) => {
            log::warn!("--exit: couldn't determine the executable path: {e}");
            ExitOutcome::StillRunning
        }
    }
}

fn request_exit_for(exe: &Path, wait: Duration, pids: &[u32]) -> ExitOutcome {
    // Open the processes before signalling, so a fast exit can't be missed.
    let mut processes = Vec::new();
    let mut outcome = if pids.is_empty() { ExitOutcome::NotRunning } else { ExitOutcome::Exited };
    for &pid in pids {
        let rights = PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
        let opened = unsafe { OpenProcess(rights | PROCESS_TERMINATE, false, pid) }
            .or_else(|_| unsafe { OpenProcess(rights, false, pid) });
        match opened {
            Ok(handle) => match super::window::query_process_image_path(handle) {
                Ok(image) if super::window::same_executable(&image, exe) => processes.push(handle),
                result => {
                    if let Err(e) = result {
                        log::error!("--exit: couldn't verify process {pid}: {e}");
                        outcome = ExitOutcome::StillRunning;
                    }
                    unsafe {
                        let _ = CloseHandle(handle);
                    }
                }
            },
            Err(e) if e.code() == ERROR_INVALID_PARAMETER.to_hresult() => {} // Already exited.
            Err(e) => {
                log::error!("--exit: couldn't open process {pid}: {e}");
                outcome = ExitOutcome::StillRunning;
            }
        }
    }
    // A listener may already have been dropped while shutdown is still in
    // progress. Its absence never bypasses waiting on the target processes.
    let name = HSTRING::from(event_name(exe));
    if let Ok(event) = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &name) } {
        if let Err(e) = unsafe { SetEvent(event) } {
            log::warn!("--exit: couldn't signal the running copy: {e}");
        }
        unsafe {
            let _ = CloseHandle(event);
        }
    }
    let deadline = Instant::now() + wait;
    for process in processes {
        let left = deadline.saturating_duration_since(Instant::now());
        let millis = left.as_millis().min(u128::from(u32::MAX - 1)) as u32;
        match unsafe { WaitForSingleObject(process, millis) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => {
                log::warn!("--exit: a running copy did not exit within {wait:?}; terminating it");
                let terminated = unsafe { TerminateProcess(process, 1) };
                // Termination is asynchronous. Success requires confirmed exit,
                // including when it raced a clean exit and TerminateProcess failed.
                if unsafe { WaitForSingleObject(process, 2000) } != WAIT_OBJECT_0 {
                    log::error!("--exit: process exit was not confirmed: {terminated:?}");
                    outcome = ExitOutcome::StillRunning;
                } else if terminated.is_ok() && outcome != ExitOutcome::StillRunning {
                    outcome = ExitOutcome::Terminated;
                }
            }
            other => {
                log::error!("--exit: couldn't wait on target process: {other:?}");
                outcome = ExitOutcome::StillRunning;
            }
        }
        unsafe {
            let _ = CloseHandle(process);
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TestChild {
        child: std::process::Child,
        exe: PathBuf,
    }

    impl TestChild {
        fn new(label: &str) -> Self {
            Self::with_name(label, &format!("glass-mixer-exit-{label}.exe"))
        }

        fn with_name(label: &str, filename: &str) -> Self {
            use std::os::windows::process::CommandExt;
            let dir = std::env::temp_dir().join(format!("glass-mixer-exit-{label}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let exe = dir.join(filename);
            let cmd = std::env::var_os("ComSpec").unwrap_or_else(|| r"C:\Windows\System32\cmd.exe".into());
            std::fs::copy(cmd, &exe).unwrap();
            let child = std::process::Command::new(&exe)
                .args(["/C", "pause"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .spawn()
                .unwrap();
            Self { child, exe }
        }
    }

    impl Drop for TestChild {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(self.exe.parent().unwrap());
        }
    }

    #[test]
    fn a_missing_event_does_not_skip_a_running_process() {
        let mut target = TestChild::new("missing-event");
        let outcome = request_exit_for(&target.exe, Duration::from_millis(50), &[target.child.id()]);
        assert_eq!(outcome, ExitOutcome::Terminated);
        assert!(target.child.try_wait().unwrap().is_some());
    }

    #[test]
    fn dropping_the_listener_during_shutdown_does_not_skip_the_process() {
        let mut target = TestChild::new("dropped-listener");
        let listener = ExitListener::start_for(&target.exe, || {}).unwrap();
        drop(listener);
        let outcome = request_exit_for(&target.exe, Duration::from_millis(50), &[target.child.id()]);
        assert_eq!(outcome, ExitOutcome::Terminated);
        assert!(target.child.try_wait().unwrap().is_some());
    }

    #[test]
    fn a_windowless_process_is_found_by_executable_path() {
        let target = TestChild::new("windowless");
        let other = TestChild::with_name("other-path", target.exe.file_name().unwrap().to_str().unwrap());
        let pids = super::super::window::instance_pids(&target.exe).unwrap();
        assert!(pids.contains(&target.child.id()), "a target without a window must be found");
        assert!(!pids.contains(&other.child.id()), "a different executable path must not match");
    }

    #[test]
    fn a_pid_now_belonging_to_a_different_executable_is_not_stopped() {
        let target = TestChild::new("pid-path-target");
        let mut other = TestChild::with_name("pid-path-other", target.exe.file_name().unwrap().to_str().unwrap());
        let _listener = ExitListener::start_for(&target.exe, || {}).unwrap();
        let outcome = request_exit_for(&target.exe, Duration::from_millis(50), &[other.child.id()]);
        assert!(other.child.try_wait().unwrap().is_none(), "an unrelated executable path must never be terminated");
        assert_ne!(outcome, ExitOutcome::Terminated);
    }

    #[test]
    fn each_install_path_has_its_own_event() {
        let installed = Path::new(r"C:\Users\me\AppData\Local\Programs\Glass Mixer\glass-mixer.exe");
        let portable = Path::new(r"D:\Tools\glass-mixer.exe");
        assert_eq!(event_name(installed), event_name(&PathBuf::from(installed.to_string_lossy().to_uppercase())));
        assert_ne!(event_name(installed), event_name(portable));
        assert!(event_name(portable).starts_with("Local\\GlassMixer.Exit."));
    }

    #[test]
    fn no_listener_means_not_running() {
        let exe = Path::new(r"C:\nowhere\glass-mixer-test-not-running.exe");
        assert_eq!(request_exit_for(exe, Duration::from_millis(50), &[]), ExitOutcome::NotRunning);
    }

    #[test]
    fn a_request_reaches_only_the_listener_for_that_path() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mine = PathBuf::from(format!(r"C:\test\{}\glass-mixer.exe", std::process::id()));
        let other = PathBuf::from(format!(r"C:\portable\{}\glass-mixer.exe", std::process::id()));
        let tx_other = tx.clone();
        let _listener = ExitListener::start_for(&mine, move || tx.send("mine").unwrap()).unwrap();
        let _other = ExitListener::start_for(&other, move || tx_other.send("other").unwrap()).unwrap();

        assert_eq!(request_exit_for(&mine, Duration::from_millis(50), &[]), ExitOutcome::NotRunning);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok("mine"));
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "the other path must not be signalled");
    }

    #[test]
    fn a_copy_that_does_not_exit_in_time_is_terminated() {
        let mut target = TestChild::new("stuck");
        // A listener that ignores the request, like a copy whose UI thread hangs.
        let _listener = ExitListener::start_for(&target.exe, || {}).unwrap();
        let started = Instant::now();
        let outcome = request_exit_for(&target.exe, Duration::from_millis(300), &[target.child.id()]);
        assert_eq!(outcome, ExitOutcome::Terminated);
        assert!(started.elapsed() < Duration::from_secs(3), "took {:?}", started.elapsed());
        assert!(target.child.try_wait().unwrap().is_some(), "the stuck process must be gone");
    }

    #[test]
    fn a_stale_request_does_not_stop_a_new_listener() {
        let exe = PathBuf::from(format!(r"C:\stale\{}\glass-mixer.exe", std::process::id()));
        // Someone else holds the event open in the signalled state.
        let held = unsafe { CreateEventW(None, true, true, &HSTRING::from(event_name(&exe))) }.unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        let listener = ExitListener::start_for(&exe, move || tx.send(()).unwrap()).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "the old signal must be cleared");
        drop(listener);
        unsafe {
            let _ = CloseHandle(held);
        }
    }

    #[test]
    fn dropping_a_listener_stops_it() {
        let exe = PathBuf::from(format!(r"C:\dropped\{}\glass-mixer.exe", std::process::id()));
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        drop(ExitListener::start_for(&exe, move || tx.send(()).unwrap()).unwrap());
        assert_eq!(request_exit_for(&exe, Duration::from_millis(50), &[]), ExitOutcome::NotRunning);
        assert!(rx.try_recv().is_err());
    }
}
