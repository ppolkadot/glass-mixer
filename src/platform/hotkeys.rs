//! Optional global shortcuts. A dedicated Win32 message loop sleeps in
//! `GetMessageW`; configuration and shutdown wake it with thread messages.
//!
//! The worker thread never touches the UI window. Window actions are queued as
//! [`HotkeyEvent`]s and carried out by the UI thread, so shutdown never waits on a
//! cross-thread window call. Each binding is registered on its own: one invalid or
//! conflicting chord is reported as a [`FailedBinding`] and the others stay active.

use std::{
    sync::Arc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey},
        WindowsAndMessaging::{GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY},
    },
};

use crate::audio::{AudioHandle, Command};
pub use crate::platform::hotkey_model::*;
use crate::platform::hotkey_model::{Chord, Validated, validate_bindings};

const CONFIGURE_MESSAGE: u32 = WM_APP + 71;
/// How long shutdown waits for the worker before detaching it.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

#[derive(Default)]
struct SharedStatus {
    current: HotkeyStatus,
    revision: u64,
}

enum Request {
    Configure { bindings: Vec<HotkeyBinding>, step: u8, revision: u64 },
    Shutdown,
}

pub struct HotkeyService {
    tx: Sender<Request>,
    events: Receiver<HotkeyEvent>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
    status: Arc<Mutex<SharedStatus>>,
}

impl HotkeyService {
    /// Starts with no registered shortcuts. Call `configure` to opt in.
    pub fn start(audio: AudioHandle, context: egui::Context) -> Result<Self, String> {
        Self::start_with(move |command| audio.send(command), context)
    }

    /// Like `start`, with the audio sink supplied by the caller.
    pub fn start_with(sink: impl Fn(Command) + Send + 'static, context: egui::Context) -> Result<Self, String> {
        Self::start_inner(sink, context, Registration::new)
    }

    /// Bindings resolve without `RegisterHotKey`, so tests don't depend on
    /// which chords other processes own.
    #[cfg(test)]
    fn start_synthetic(sink: impl Fn(Command) + Send + 'static, context: egui::Context) -> Result<Self, String> {
        Self::start_inner(sink, context, synthetic_registration)
    }

    fn start_inner(
        sink: impl Fn(Command) + Send + 'static,
        context: egui::Context,
        register: Registrar,
    ) -> Result<Self, String> {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (events_tx, events) = crossbeam_channel::unbounded();
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
        let status = Arc::new(Mutex::new(SharedStatus::default()));
        let worker_status = status.clone();
        let thread = std::thread::Builder::new()
            .name("global-hotkeys".into())
            .spawn(move || {
                // PeekMessage creates the message queue before start returns,
                // so even an immediate configure/drop can safely post to it.
                let mut message = MSG::default();
                unsafe {
                    let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                }
                let thread_id = unsafe { GetCurrentThreadId() };
                if ready_tx.send(thread_id).is_err() {
                    return;
                }
                run(rx, sink, events_tx, &context, &worker_status, register);
            })
            .map_err(|error| format!("Could not start global shortcuts: {error}"))?;
        match ready_rx.recv() {
            Ok(thread_id) => Ok(Self { tx, events, thread_id, thread: Some(thread), status }),
            Err(error) => {
                let _ = thread.join();
                Err(format!("Global shortcuts stopped during startup: {error}"))
            }
        }
    }

    /// Enqueues a change; `status()` reports the result once the worker has
    /// applied it. Calling again retries every binding that is not active.
    pub fn configure(&self, bindings: &[HotkeyBinding], step: u8) -> Result<(), String> {
        let mut status = self.status.lock();
        status.revision = status.revision.wrapping_add(1);
        status.current.pending = true;
        status.current.errors.clear();
        let request = Request::Configure { bindings: bindings.to_vec(), step, revision: status.revision };
        let sent = self.tx.send(request).map_err(|_| "The global shortcut service has stopped.".to_owned());
        let result = sent.and_then(|()| unsafe {
            PostThreadMessageW(self.thread_id, CONFIGURE_MESSAGE, WPARAM(0), LPARAM(0))
                .map_err(|error| format!("Could not update global shortcuts: {error}. Restart Glass Mixer to retry."))
        });
        if let Err(error) = &result {
            status.current.pending = false;
            status.current.errors.push(error.clone());
        }
        result
    }

    pub fn status(&self) -> HotkeyStatus {
        self.status.lock().current.clone()
    }

    /// Window actions queued since the last call. Call from the UI thread.
    pub fn take_events(&self) -> Vec<HotkeyEvent> {
        self.events.try_iter().collect()
    }

    /// Stops the worker, detaching it if it hasn't finished within `SHUTDOWN_WAIT`.
    pub fn shutdown(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        let _ = self.tx.send(Request::Shutdown);
        // If the post fails because the queue is full, the next message drains requests too.
        let _ = unsafe { PostThreadMessageW(self.thread_id, CONFIGURE_MESSAGE, WPARAM(0), LPARAM(0)) };
        let deadline = Instant::now() + SHUTDOWN_WAIT;
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if !thread.is_finished() {
            log::error!("global shortcut thread did not stop within {SHUTDOWN_WAIT:?}; detaching it");
        } else if thread.join().is_err() {
            log::error!("global shortcut thread panicked");
        }
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Registration {
    id: i32,
    binding: HotkeyBinding,
    chord: Chord,
    /// Synthetic test registrations answer `WM_HOTKEY` messages but were never
    /// passed to `RegisterHotKey`, so they must not be unregistered on drop.
    #[cfg(test)]
    synthetic: bool,
}

/// Turns one validated binding into a registration; tests substitute a synthetic one.
type Registrar = fn(i32, HotkeyBinding, Chord) -> Result<Registration, String>;

impl Registration {
    /// Returns the reason when Windows rejects the chord.
    fn new(id: i32, binding: HotkeyBinding, chord: Chord) -> Result<Self, String> {
        unsafe { RegisterHotKey(None, id, HOT_KEY_MODIFIERS(chord.modifiers) | MOD_NOREPEAT, chord.key) }
            .map_err(|error| format!("Windows could not register this shortcut ({error}). It may be reserved or used by another app; choose another chord or release it in that app, then use Retry."))?;
        Ok(Self {
            id,
            binding,
            chord,
            #[cfg(test)]
            synthetic: false,
        })
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        #[cfg(test)]
        if self.synthetic {
            return;
        }
        if let Err(error) = unsafe { UnregisterHotKey(None, self.id) } {
            log::warn!("could not unregister shortcut {}: {error}", self.binding.chord);
        }
    }
}

#[cfg(test)]
fn synthetic_registration(id: i32, binding: HotkeyBinding, chord: Chord) -> Result<Registration, String> {
    Ok(Registration { id, binding, chord, synthetic: true })
}

#[derive(Debug, PartialEq)]
enum Resolved {
    Audio(Command),
    Window(HotkeyEvent),
}

#[derive(Default)]
struct Registrations {
    active: Vec<Registration>,
    next_id: i32,
    step: u8,
}

impl Registrations {
    fn bindings(&self) -> Vec<HotkeyBinding> {
        self.active.iter().map(|registration| registration.binding.clone()).collect()
    }

    #[cfg(test)]
    fn configure(&mut self, bindings: &[HotkeyBinding], step: u8) -> Vec<FailedBinding> {
        self.configure_with(bindings, step, Registration::new)
    }

    /// Makes the active set match `bindings` and returns the bindings that are
    /// invalid, duplicated or rejected; all others stay registered.
    fn configure_with(&mut self, bindings: &[HotkeyBinding], step: u8, register: Registrar) -> Vec<FailedBinding> {
        let Validated { valid, rejected } = validate_bindings(bindings);
        let mut failed = rejected;
        // Chords still wanted keep their registration, even if reassigned to another action.
        self.active.retain(|registration| valid.iter().any(|(_, chord)| registration.chord == *chord));
        for (binding, chord) in valid {
            if let Some(existing) = self.active.iter_mut().find(|registration| registration.chord == chord) {
                existing.binding = binding;
                continue;
            }
            // Application hotkey IDs must be in 0x0000..=0xBFFF.
            loop {
                self.next_id = self.next_id % 0xBFFF + 1;
                if !self.active.iter().any(|r| r.id == self.next_id) {
                    break;
                }
            }
            match register(self.next_id, binding.clone(), chord) {
                Ok(registration) => self.active.push(registration),
                Err(reason) => failed.push(FailedBinding {
                    action: binding.action,
                    target: binding.target,
                    chord: binding.chord,
                    reason,
                }),
            }
        }
        self.step = step.clamp(1, 100);
        failed
    }

    fn resolve(&self, id: usize, lparam: isize) -> Option<Resolved> {
        let registration = self.active.iter().find(|r| r.id as usize == id)?;
        // Ignore a queued message belonging to an old registration if an ID
        // eventually wraps and is reused for a different chord.
        let modifiers = lparam as u32 & 7;
        let key = (lparam as u32 >> 16) & 0xFFFF;
        if registration.chord != (Chord { modifiers, key }) {
            return None;
        }
        let delta = self.step as f32 / 100.0;
        let binding = &registration.binding;
        let app = || match &binding.target {
            Some(HotkeyTarget::App { key, .. }) => Some(key.clone()),
            _ => None,
        };
        Some(match binding.action {
            HotkeyAction::ToggleOutputMute => Resolved::Audio(Command::ToggleOutputMute),
            HotkeyAction::ToggleInputMute => Resolved::Audio(Command::ToggleInputMute),
            HotkeyAction::VolumeUp => Resolved::Audio(Command::AdjustOutputVolume { delta }),
            HotkeyAction::VolumeDown => Resolved::Audio(Command::AdjustOutputVolume { delta: -delta }),
            HotkeyAction::ToggleWindow => Resolved::Window(HotkeyEvent::ToggleWindow),
            HotkeyAction::CycleOutputDevice => Resolved::Audio(Command::CycleOutputDevice),
            HotkeyAction::AppMute => Resolved::Audio(Command::ToggleAppMute { key: app()? }),
            HotkeyAction::AppVolumeUp => Resolved::Audio(Command::AdjustAppVolume { key: app()?, delta }),
            HotkeyAction::AppVolumeDown => Resolved::Audio(Command::AdjustAppVolume { key: app()?, delta: -delta }),
            HotkeyAction::AppSolo => Resolved::Audio(Command::ToggleSolo { key: app()? }),
            HotkeyAction::ActivateProfile => match &binding.target {
                Some(HotkeyTarget::Profile { name }) => Resolved::Window(HotkeyEvent::ActivateProfile(name.clone())),
                _ => return None,
            },
        })
    }
}

fn run(
    rx: Receiver<Request>,
    sink: impl Fn(Command),
    events: Sender<HotkeyEvent>,
    context: &egui::Context,
    status: &Mutex<SharedStatus>,
    register: Registrar,
) {
    let mut registrations = Registrations::default();
    let mut message = MSG::default();
    'messages: loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result <= 0 {
            if result < 0 {
                status.lock().current.errors.push(format!(
                    "Global shortcuts stopped: {}. Restart Glass Mixer to retry.",
                    windows::core::Error::from_thread()
                ));
            }
            break;
        }
        loop {
            match rx.try_recv() {
                Ok(Request::Shutdown) | Err(crossbeam_channel::TryRecvError::Disconnected) => break 'messages,
                Ok(Request::Configure { bindings, step, revision }) => {
                    let failed = registrations.configure_with(&bindings, step, register);
                    if failed.is_empty() {
                        log::info!("registered {} global shortcuts", registrations.active.len());
                    } else {
                        log::warn!(
                            "{} global shortcuts active; not active: {}",
                            registrations.active.len(),
                            failed.iter().map(FailedBinding::message).collect::<Vec<_>>().join("; ")
                        );
                    }
                    let mut shared = status.lock();
                    shared.current.active = registrations.bindings();
                    if shared.revision == revision {
                        shared.current.failed = failed;
                        shared.current.pending = false;
                    }
                    context.request_repaint();
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
            }
        }
        if message.message == WM_HOTKEY {
            log::debug!(
                "global shortcut message: id={}, modifiers={}, key={}",
                message.wParam.0,
                message.lParam.0 as u32 & 0xffff,
                (message.lParam.0 as u32 >> 16) & 0xffff
            );
            match registrations.resolve(message.wParam.0, message.lParam.0) {
                Some(Resolved::Audio(command)) => sink(command),
                Some(Resolved::Window(event)) => {
                    let _ = events.send(event);
                }
                None => {}
            }
            context.request_repaint();
        }
    }
    drop(registrations);
    let mut shared = status.lock();
    shared.current.active.clear();
    shared.current.pending = false;
    context.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AppKey;
    use crate::platform::hotkey_model::parse_chord;

    fn binding(action: HotkeyAction, chord: &str) -> HotkeyBinding {
        HotkeyBinding { action, chord: chord.into(), target: None }
    }

    fn lparam(chord: Chord) -> isize {
        ((chord.key << 16) | chord.modifiers) as isize
    }

    fn registrations_for(action: HotkeyAction, chord: &str, step: u8) -> Registrations {
        let parsed = parse_chord(chord).unwrap();
        Registrations {
            active: vec![Registration { id: 7, binding: binding(action, chord), chord: parsed, synthetic: true }],
            next_id: 7,
            step,
        }
    }

    fn app_target(path: &str, name: &str) -> Option<HotkeyTarget> {
        Some(HotkeyTarget::App { key: AppKey::Exe(path.into()), name: name.into() })
    }

    fn targeted(action: HotkeyAction, chord: &str, target: Option<HotkeyTarget>) -> HotkeyBinding {
        HotkeyBinding { action, chord: chord.into(), target }
    }

    fn resolve_one(binding: HotkeyBinding, step: u8) -> Option<Resolved> {
        let chord = parse_chord(&binding.chord).unwrap();
        let registrations =
            Registrations { active: vec![Registration { id: 7, binding, chord, synthetic: true }], next_id: 7, step };
        registrations.resolve(7, lparam(chord))
    }

    #[test]
    fn app_profile_and_device_shortcuts_resolve_to_their_commands() {
        let discord = AppKey::Exe(r"c:\apps\discord.exe".into());
        let target = app_target(r"c:\apps\discord.exe", "Discord");
        let cases = [
            (HotkeyAction::AppMute, Resolved::Audio(Command::ToggleAppMute { key: discord.clone() })),
            (HotkeyAction::AppVolumeUp, Resolved::Audio(Command::AdjustAppVolume { key: discord.clone(), delta: 0.1 })),
            (
                HotkeyAction::AppVolumeDown,
                Resolved::Audio(Command::AdjustAppVolume { key: discord.clone(), delta: -0.1 }),
            ),
            (HotkeyAction::AppSolo, Resolved::Audio(Command::ToggleSolo { key: discord })),
        ];
        for (action, expected) in cases {
            assert_eq!(resolve_one(targeted(action, "Ctrl+Alt+D", target.clone()), 10), Some(expected), "{action:?}");
        }
        let profile = Some(HotkeyTarget::Profile { name: "Gaming".into() });
        assert_eq!(
            resolve_one(targeted(HotkeyAction::ActivateProfile, "Ctrl+Alt+G", profile), 5),
            Some(Resolved::Window(HotkeyEvent::ActivateProfile("Gaming".into())))
        );
        assert_eq!(
            resolve_one(binding(HotkeyAction::CycleOutputDevice, "Ctrl+Alt+O"), 5),
            Some(Resolved::Audio(Command::CycleOutputDevice))
        );
        assert_eq!(
            resolve_one(targeted(HotkeyAction::AppMute, "Ctrl+Alt+D", None), 5),
            None,
            "an app action without its app does nothing"
        );
    }

    #[test]
    fn window_shortcut_is_queued_for_the_ui_thread_not_performed() {
        let chord = parse_chord("Ctrl+Alt+F23").unwrap();
        let registrations = registrations_for(HotkeyAction::ToggleWindow, "Ctrl+Alt+F23", 5);
        assert_eq!(registrations.resolve(7, lparam(chord)), Some(Resolved::Window(HotkeyEvent::ToggleWindow)));
        // Nothing in this module can touch a window: it does not import `platform::window`.
        // Everything before the test module is checked, including test-only seams.
        let source = include_str!("hotkeys.rs");
        let (module, _) = source.split_once("\nmod tests {").expect("test module marker");
        assert!(module.contains("RegisterHotKey("), "the scan must cover the worker, not stop early");
        assert!(!module.contains("window::"));
    }

    #[test]
    fn volume_shortcuts_use_the_configured_step_and_stale_messages_are_ignored() {
        let chord = parse_chord("Ctrl+Alt+Up").unwrap();
        let registrations = registrations_for(HotkeyAction::VolumeUp, "Ctrl+Alt+Up", 7);
        assert_eq!(
            registrations.resolve(7, lparam(chord)),
            Some(Resolved::Audio(Command::AdjustOutputVolume { delta: 0.07 }))
        );
        assert_eq!(registrations.resolve(8, lparam(chord)), None, "unknown id");
        let other = parse_chord("Ctrl+Alt+Down").unwrap();
        assert_eq!(registrations.resolve(7, lparam(other)), None, "id reused for a different chord");
    }

    #[test]
    fn service_starts_and_stops_promptly_with_no_shortcuts() {
        let mut service = HotkeyService::start_with(|_| {}, egui::Context::default()).unwrap();
        service.configure(&[], 5).unwrap();
        let started = Instant::now();
        service.shutdown();
        assert!(started.elapsed() < Duration::from_millis(1500), "shutdown took {:?}", started.elapsed());
    }

    #[test]
    fn worker_exits_and_ignores_stale_hotkey_when_request_channel_disconnects() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (events_tx, events_rx) = crossbeam_channel::unbounded();
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
        let status = Arc::new(Mutex::new(SharedStatus::default()));
        let worker_status = status.clone();
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sink_called = called.clone();
        let ctx = egui::Context::default();
        let thread = std::thread::spawn(move || {
            let mut message = MSG::default();
            unsafe {
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            }
            ready_tx.send(unsafe { GetCurrentThreadId() }).unwrap();
            run(
                rx,
                move |_| sink_called.store(true, std::sync::atomic::Ordering::SeqCst),
                events_tx,
                &ctx,
                &worker_status,
                Registration::new,
            );
        });
        let thread_id = ready_rx.recv().unwrap();
        // Drop the sender without sending Request::Shutdown, then post a wakeup/hotkey message.
        drop(tx);
        let chord = parse_chord("Ctrl+Alt+Up").unwrap();
        unsafe {
            PostThreadMessageW(thread_id, WM_HOTKEY, WPARAM(1), LPARAM(lparam(chord))).unwrap();
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(thread.is_finished(), "worker stayed alive after request channel disconnected");
        thread.join().unwrap();
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst), "stale hotkey invoked sink after disconnect");
        assert!(events_rx.try_recv().is_err(), "stale hotkey queued window event after disconnect");
    }

    #[test]
    #[ignore = "registers real Windows global shortcuts; unregisters before returning"]
    fn registration_reports_conflicts_and_releases_key() {
        let requested = [binding(HotkeyAction::VolumeUp, "Ctrl+Alt+F23")];
        let mut ours = Registrations::default();
        assert!(ours.configure(&requested, 5).is_empty());
        let mut second = Registrations { next_id: 100, ..Default::default() };
        assert_eq!(second.configure(&requested, 5).len(), 1);
        drop(ours);
        assert!(second.configure(&requested, 5).is_empty());
    }

    #[test]
    #[ignore = "registers real Windows global shortcuts; unregisters before returning"]
    fn a_conflicting_binding_leaves_the_others_active_and_retry_recovers() {
        let wanted = [
            binding(HotkeyAction::ToggleWindow, "Ctrl+Alt+F23"),
            binding(HotkeyAction::VolumeUp, "Ctrl+Alt+F22"),
            binding(HotkeyAction::VolumeDown, "Ctrl+Alt+nonsense"),
        ];
        // Another "application" owns Ctrl+Alt+F22.
        let mut other_app = Registrations::default();
        assert!(other_app.configure(&[binding(HotkeyAction::ToggleOutputMute, "Ctrl+Alt+F22")], 5).is_empty());

        let mut ours = Registrations { next_id: 100, ..Default::default() };
        let failed = ours.configure(&wanted, 5);
        let failed_actions: Vec<_> = failed.iter().map(|f| f.action).collect();
        assert_eq!(failed_actions, [HotkeyAction::VolumeDown, HotkeyAction::VolumeUp]);
        assert_eq!(
            ours.bindings(),
            [binding(HotkeyAction::ToggleWindow, "Ctrl+Alt+F23")],
            "working binding stays active"
        );

        // The other app lets go; configuring again retries the conflicting binding.
        drop(other_app);
        let failed = ours.configure(&wanted, 5);
        assert_eq!(failed.len(), 1, "only the invalid binding still fails");
        assert_eq!(ours.bindings().len(), 2);
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum InjectionError {
        BlockedZero,
        Partial { sent: u32, expected: u32 },
        NotDelivered,
    }

    fn try_inject_ctrl_alt_f21_with(
        mut send_batch: impl FnMut(&[windows::Win32::UI::Input::KeyboardAndMouse::INPUT]) -> u32,
    ) -> Result<(), InjectionError> {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
        };
        let key = |vk: u16, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs =
            [key(0x11, false), key(0x12, false), key(0x84, false), key(0x84, true), key(0x12, true), key(0x11, true)];
        let expected = inputs.len() as u32;
        let sent = send_batch(&inputs);
        if sent == 0 {
            return Err(InjectionError::BlockedZero);
        }
        if sent != expected {
            // Release any depressed keys if a batch was only partially injected.
            let releases = [key(0x84, true), key(0x12, true), key(0x11, true)];
            let _ = send_batch(&releases);
            return Err(InjectionError::Partial { sent, expected });
        }
        Ok(())
    }

    /// Presses and releases Ctrl+Alt+F21 through the real Windows input queue.
    /// Returns an error, never a fallback, if `SendInput` is blocked.
    fn try_press_ctrl_alt_f21() -> Result<(), InjectionError> {
        use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, SendInput};
        try_inject_ctrl_alt_f21_with(|batch| unsafe { SendInput(batch, std::mem::size_of::<INPUT>() as i32) })
    }

    struct InjectorGuard {
        stop: Arc<std::sync::atomic::AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl InjectorGuard {
        fn spawn(mut inject: impl FnMut() + Send + 'static) -> Self {
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let thread_stop = stop.clone();
            let handle = std::thread::spawn(move || {
                while !thread_stop.load(std::sync::atomic::Ordering::Relaxed) {
                    inject();
                    std::thread::sleep(Duration::from_millis(5));
                }
            });
            Self { stop, handle: Some(handle) }
        }

        fn stop_and_join(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    impl Drop for InjectorGuard {
        fn drop(&mut self) {
            self.stop_and_join();
        }
    }

    fn exercise_shutdown_under_pressure(
        service: &mut HotkeyService,
        wait_window: Duration,
        inject: impl FnMut() + Send + 'static,
    ) -> Duration {
        let mut guard = InjectorGuard::spawn(inject);
        std::thread::sleep(wait_window);
        assert!(!service.take_events().is_empty(), "the injected chord never reached the service");
        let started = Instant::now();
        service.shutdown();
        let took = started.elapsed();
        guard.stop_and_join();
        took
    }

    fn exercise_real_input_shutdown_under_pressure(
        service: &mut HotkeyService,
        wait_window: Duration,
        mut inject: impl FnMut() -> Result<(), InjectionError> + Send + 'static,
    ) -> Result<Duration, InjectionError> {
        let first_err: Arc<Mutex<Option<InjectionError>>> = Arc::new(Mutex::new(None));
        let err_slot = first_err.clone();
        let ok_batches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ok_slot = ok_batches.clone();
        let mut guard = InjectorGuard::spawn(move || match inject() {
            Ok(()) => {
                ok_slot.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            Err(e) => {
                let mut slot = err_slot.lock();
                if slot.is_none() {
                    *slot = Some(e);
                }
            }
        });
        std::thread::sleep(wait_window);
        // Drop the error mutex before joining: the injector may still need it
        // to publish another failed batch while shutdown is in progress.
        let recorded_error = first_err.lock().clone();
        if let Some(err) = recorded_error {
            guard.stop_and_join();
            service.shutdown();
            return Err(err);
        }
        if ok_batches.load(std::sync::atomic::Ordering::SeqCst) == 0 || service.take_events().is_empty() {
            guard.stop_and_join();
            service.shutdown();
            return Err(InjectionError::NotDelivered);
        }
        let started = Instant::now();
        service.shutdown();
        let took = started.elapsed();
        guard.stop_and_join();
        if let Some(err) = first_err.lock().clone() {
            return Err(err);
        }
        Ok(took)
    }

    #[test]
    fn injector_guard_stops_and_joins_when_assertion_fails_during_pressure_test() {
        let mut service = HotkeyService::start_with(|_| {}, egui::Context::default()).unwrap();
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let alive_in_thread = alive.clone();
        let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ticks_in_thread = ticks.clone();

        struct ThreadLivenessOnExit(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for ThreadLivenessOnExit {
            fn drop(&mut self) {
                self.0.store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let liveness = Some(ThreadLivenessOnExit(alive_in_thread.clone()));
            exercise_shutdown_under_pressure(&mut service, Duration::from_millis(25), move || {
                if liveness.is_some() {
                    alive_in_thread.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                let _keep = &liveness;
                ticks_in_thread.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })
        }));
        assert!(outcome.is_err(), "no-delivery path must fail the assertion as expected");
        assert!(ticks.load(std::sync::atomic::Ordering::SeqCst) > 0, "injector must have run before panic");
        assert!(
            !alive.load(std::sync::atomic::Ordering::SeqCst),
            "InjectorGuard Drop must stop and join the injector thread during stack unwinding"
        );
        let ticks_after_unwind = ticks.load(std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            ticks.load(std::sync::atomic::Ordering::SeqCst),
            ticks_after_unwind,
            "injector thread must not continue ticking after unwind"
        );
        service.shutdown();
    }

    #[test]
    fn real_input_pressure_rejects_zero_and_partial_injection_and_cleans_up_injector() {
        // 1. Force zero injection through the seam: must report BlockedZero, never claim
        // real-delivery success, and stop/join the injector thread via InjectorGuard.
        let mut service_zero = HotkeyService::start_with(|_| {}, egui::Context::default()).unwrap();
        let zero_ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let zero_ticks_in_thread = zero_ticks.clone();
        let res_zero =
            exercise_real_input_shutdown_under_pressure(&mut service_zero, Duration::from_millis(25), move || {
                zero_ticks_in_thread.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                try_inject_ctrl_alt_f21_with(|_| 0)
            });
        assert_eq!(res_zero, Err(InjectionError::BlockedZero));
        let stopped_zero = zero_ticks.load(std::sync::atomic::Ordering::SeqCst);
        assert!(stopped_zero > 0);
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(zero_ticks.load(std::sync::atomic::Ordering::SeqCst), stopped_zero);

        // 2. Force partial injection (2 of 6 events): must issue key-up release cleanup,
        // report Partial { sent: 2, expected: 6 }, and stop/join the injector thread.
        let mut service_partial = HotkeyService::start_with(|_| {}, egui::Context::default()).unwrap();
        let release_batches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let release_batches_in_thread = release_batches.clone();
        let res_partial =
            exercise_real_input_shutdown_under_pressure(&mut service_partial, Duration::from_millis(25), move || {
                let mut call = 0usize;
                let releases = release_batches_in_thread.clone();
                try_inject_ctrl_alt_f21_with(|batch| {
                    call += 1;
                    if call == 1 {
                        assert_eq!(batch.len(), 6);
                        2
                    } else {
                        assert_eq!(batch.len(), 3, "partial injection must release F21, Alt, and Ctrl");
                        releases.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        3
                    }
                })
            });
        assert_eq!(res_partial, Err(InjectionError::Partial { sent: 2, expected: 6 }));
        assert!(release_batches.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }

    #[test]
    fn shutdown_completes_under_posted_hotkey_message_pressure() {
        // Synthetic registrations need no global chord, so this always runs.
        let mut service = HotkeyService::start_synthetic(|_| {}, egui::Context::default()).unwrap();
        service.configure(&[binding(HotkeyAction::ToggleWindow, "Ctrl+Alt+F23")], 5).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while service.status().pending {
            assert!(Instant::now() < deadline, "hotkey worker did not finish configuring");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            service.status().active.len(),
            1,
            "the synthetic binding must be active without Windows registration: {:?}",
            service.status()
        );
        let thread_id = service.thread_id;
        let took = exercise_shutdown_under_pressure(&mut service, Duration::from_millis(60), move || {
            let _ = unsafe { PostThreadMessageW(thread_id, WM_HOTKEY, WPARAM(1), LPARAM((0x86 << 16) | 3)) };
        });
        assert!(took < Duration::from_millis(1500), "shutdown took {took:?} under posted WM_HOTKEY pressure");
    }

    #[test]
    #[ignore = "registers Ctrl+Alt+F21 and injects that key chord into the desktop input queue"]
    fn shutdown_completes_while_the_window_shortcut_is_being_pressed() {
        let mut service = HotkeyService::start_with(|_| {}, egui::Context::default()).unwrap();
        service.configure(&[binding(HotkeyAction::ToggleWindow, "Ctrl+Alt+F21")], 5).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while service.status().pending || service.status().active.is_empty() {
            assert!(Instant::now() < deadline, "Ctrl+Alt+F21 could not be registered: {:?}", service.status());
            std::thread::sleep(Duration::from_millis(10));
        }
        let took = exercise_real_input_shutdown_under_pressure(
            &mut service,
            Duration::from_millis(300),
            try_press_ctrl_alt_f21,
        )
        .unwrap_or_else(|err| {
            panic!(
                "real desktop SendInput injection unavailable or incomplete ({err:?}); run from an unlocked, non-elevated foreground desktop session"
            )
        });
        assert!(took < Duration::from_millis(1500), "shutdown took {took:?} under key pressure");
    }
}
