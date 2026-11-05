//! Reports which process owns the foreground window, for focus rules.
//!
//! An out-of-context WinEvent hook delivers `EVENT_SYSTEM_FOREGROUND` to this
//! module's own message-loop thread. The callback only reads the window's
//! process ID and hands it on; resolving the app and changing levels happens
//! on the audio engine thread. The watcher runs only while a focus rule is set.

use std::{
    cell::RefCell,
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        WindowsAndMessaging::{
            EVENT_SYSTEM_FOREGROUND, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, MSG, OBJID_WINDOW,
            PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT,
        },
    },
};

const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

type Report = Box<dyn Fn(u32)>;

thread_local! {
    /// The hook callback has no context argument; it runs on the watcher thread.
    static REPORT: RefCell<Option<Report>> = const { RefCell::new(None) };
}

pub struct ForegroundWatch {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl ForegroundWatch {
    /// Calls `report` with the foreground process ID, now and on every change.
    /// Glass Mixer's own windows are not reported.
    pub fn start(report: impl Fn(u32) + Send + 'static) -> Result<Self, String> {
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
        let thread = std::thread::Builder::new()
            .name("foreground-watch".into())
            .spawn(move || {
                let mut message = MSG::default();
                // Creates the message queue before the thread ID is handed out.
                unsafe {
                    let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                }
                REPORT.with(|r| *r.borrow_mut() = Some(Box::new(report)));
                let hook = unsafe {
                    SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        None,
                        Some(on_event),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };
                if hook.is_invalid() {
                    let _ = ready_tx.send(Err(format!(
                        "Couldn't watch the foreground window ({}).",
                        windows::core::Error::from_thread()
                    )));
                    return;
                }
                let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
                report_window(unsafe { GetForegroundWindow() });
                while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {}
                unsafe {
                    let _ = UnhookWinEvent(hook);
                }
                REPORT.with(|r| *r.borrow_mut() = None);
            })
            .map_err(|e| format!("Couldn't start the foreground watcher: {e}"))?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self { thread_id, thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("The foreground watcher stopped during startup.".into()),
        }
    }
}

impl Drop for ForegroundWatch {
    fn drop(&mut self) {
        let Some(thread) = self.thread.take() else { return };
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        let deadline = Instant::now() + SHUTDOWN_WAIT;
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if thread.is_finished() {
            let _ = thread.join();
        } else {
            log::warn!("foreground watcher did not stop within {SHUTDOWN_WAIT:?}; detaching it");
        }
    }
}

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if event == EVENT_SYSTEM_FOREGROUND && object == OBJID_WINDOW.0 {
        report_window(hwnd);
    }
}

fn report_window(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid != 0 && pid != std::process::id() {
        REPORT.with(|r| {
            if let Some(report) = r.borrow().as_ref() {
                report(pid);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watcher_starts_and_stops_promptly() {
        let watch = ForegroundWatch::start(|_| {}).expect("hook");
        let started = Instant::now();
        drop(watch);
        assert!(started.elapsed() < Duration::from_millis(500), "took {:?}", started.elapsed());
    }

    #[test]
    fn the_callback_never_touches_audio_or_windows_beyond_reading_the_process() {
        // Keeps the hook cheap: no COM, no blocking calls, no window changes.
        let source = include_str!("foreground.rs");
        let (callback, _) = source.split_once("\nmod tests {").unwrap();
        let callback = &callback[callback.find("unsafe extern \"system\" fn on_event").unwrap()..];
        for forbidden in ["crate::audio", "CoInitialize", "SendMessage", "Sleep", "lock()"] {
            assert!(!callback.contains(forbidden), "the hook path uses {forbidden}");
        }
    }
}
