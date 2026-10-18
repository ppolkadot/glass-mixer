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
use serde::{Deserialize, Serialize};
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey},
        WindowsAndMessaging::{GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY},
    },
};

use crate::audio::{AudioHandle, Command};

const CONFIGURE_MESSAGE: u32 = WM_APP + 71;
/// How long shutdown waits for the worker before detaching it.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

/// A window action requested by a shortcut. The UI thread performs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    ToggleWindow,
}

/// A binding that is saved but not active, with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedBinding {
    pub action: HotkeyAction,
    pub chord: String,
    pub reason: String,
}

impl FailedBinding {
    pub fn message(&self) -> String {
        format!("{} ({}): {}", self.action.label(), self.chord, self.reason)
    }
}

/// Requested settings and active registrations can differ after a conflict.
#[derive(Clone, Debug, Default)]
pub struct HotkeyStatus {
    pub active: Vec<HotkeyBinding>,
    pub failed: Vec<FailedBinding>,
    /// Service-level problems, such as a stopped worker thread.
    pub errors: Vec<String>,
    pub pending: bool,
}

impl HotkeyStatus {
    /// Every problem as one line, for notices.
    pub fn messages(&self) -> Vec<String> {
        self.failed.iter().map(FailedBinding::message).chain(self.errors.iter().cloned()).collect()
    }
}

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
                run(rx, sink, events_tx, &context, &worker_status);
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

    /// Enqueues a change; `status()` reports the exact active set and each
    /// binding that could not be registered once the worker finishes. Working
    /// bindings stay active when others fail. Calling this again retries every
    /// binding that is not active.
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

    /// Stops the worker. The worker never calls into the UI thread, so it exits
    /// promptly; if it somehow does not within `SHUTDOWN_WAIT` it is detached
    /// rather than allowed to block application exit.
    pub fn shutdown(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        let _ = self.tx.send(Request::Shutdown);
        // If the Windows queue is full, its next message also drains requests.
        // If the thread is already gone, the wait below ends immediately.
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
}

impl Registration {
    /// Returns the reason when Windows rejects the chord.
    fn new(id: i32, binding: HotkeyBinding, chord: Chord) -> Result<Self, String> {
        unsafe { RegisterHotKey(None, id, HOT_KEY_MODIFIERS(chord.modifiers) | MOD_NOREPEAT, chord.key) }
            .map_err(|error| format!("Windows could not register this shortcut ({error}). It may be reserved or used by another app; choose another chord or release it in that app, then use Retry."))?;
        Ok(Self { id, binding, chord })
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Err(error) = unsafe { UnregisterHotKey(None, self.id) } {
            log::warn!("could not unregister shortcut {}: {error}", self.binding.chord);
        }
    }
}

/// What a pressed shortcut asks for.
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

    /// Makes the active set match `bindings` as closely as Windows allows.
    /// Invalid, duplicate and rejected bindings are returned; every other
    /// binding is registered (or stays registered) regardless.
    fn configure(&mut self, bindings: &[HotkeyBinding], step: u8) -> Vec<FailedBinding> {
        let Validated { valid, rejected } = validate_bindings(bindings);
        let mut failed = rejected;
        // Release chords nobody wants any more. Chords that are still wanted keep
        // their registration, including ones reassigned to another action.
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
            match Registration::new(self.next_id, binding.clone(), chord) {
                Ok(registration) => self.active.push(registration),
                Err(reason) => failed.push(FailedBinding { action: binding.action, chord: binding.chord, reason }),
            }
        }
        self.step = step.clamp(1, 100);
        failed
    }

    /// Maps a `WM_HOTKEY` message to its action. Pure: performs no side effects.
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
        Some(match registration.binding.action {
            HotkeyAction::ToggleOutputMute => Resolved::Audio(Command::ToggleOutputMute),
            HotkeyAction::ToggleInputMute => Resolved::Audio(Command::ToggleInputMute),
            HotkeyAction::VolumeUp => Resolved::Audio(Command::AdjustOutputVolume { delta }),
            HotkeyAction::VolumeDown => Resolved::Audio(Command::AdjustOutputVolume { delta: -delta }),
            HotkeyAction::ToggleWindow => Resolved::Window(HotkeyEvent::ToggleWindow),
        })
    }
}

fn run(
    rx: Receiver<Request>,
    sink: impl Fn(Command),
    events: Sender<HotkeyEvent>,
    context: &egui::Context,
    status: &Mutex<SharedStatus>,
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
                    let failed = registrations.configure(&bindings, step);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HotkeyAction {
    ToggleOutputMute,
    ToggleInputMute,
    VolumeUp,
    VolumeDown,
    ToggleWindow,
}

impl HotkeyAction {
    pub const ALL: [Self; 5] =
        [Self::ToggleOutputMute, Self::ToggleInputMute, Self::VolumeUp, Self::VolumeDown, Self::ToggleWindow];

    pub fn label(self) -> &'static str {
        match self {
            Self::ToggleOutputMute => "Toggle output mute",
            Self::ToggleInputMute => "Toggle microphone mute",
            Self::VolumeUp => "Volume up",
            Self::VolumeDown => "Volume down",
            Self::ToggleWindow => "Show / hide mixer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyBinding {
    pub action: HotkeyAction,
    pub chord: String,
}

/// Bindings split into those that can be registered and those that cannot.
struct Validated {
    valid: Vec<(HotkeyBinding, Chord)>,
    rejected: Vec<FailedBinding>,
}

/// Validates each binding on its own. When two bindings share an action or a
/// chord, the earlier one wins and the later one is rejected.
fn validate_bindings(bindings: &[HotkeyBinding]) -> Validated {
    let mut valid: Vec<(HotkeyBinding, Chord)> = Vec::new();
    let mut rejected = Vec::new();
    for binding in bindings {
        let reject = |reason: String| FailedBinding { action: binding.action, chord: binding.chord.clone(), reason };
        if valid.iter().any(|(other, _)| other.action == binding.action) {
            rejected.push(reject("choose only one shortcut for this action.".into()));
            continue;
        }
        match parse_chord(&binding.chord) {
            Ok(chord) => {
                if let Some((other, _)) = valid.iter().find(|(_, existing)| *existing == chord) {
                    rejected.push(reject(format!(
                        "{} is also assigned to {}. Choose a different chord.",
                        binding.chord,
                        other.action.label()
                    )));
                } else {
                    valid.push((binding.clone(), chord));
                }
            }
            Err(error) => rejected.push(reject(error)),
        }
    }
    Validated { valid, rejected }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Chord {
    modifiers: u32,
    key: u32,
}

fn parse_chord(value: &str) -> Result<Chord, String> {
    let mut modifiers = 0;
    let mut key = None;
    for part in value.split('+') {
        let part = part.trim().to_ascii_uppercase();
        let modifier = match part.as_str() {
            "CTRL" | "CONTROL" => 2,
            "ALT" => 1,
            "SHIFT" => 4,
            _ => 0,
        };
        if modifier != 0 {
            if modifiers & modifier != 0 || key.is_some() {
                return Err("Put each modifier once, before the key (for example Ctrl+Alt+M).".into());
            }
            modifiers |= modifier;
            continue;
        }
        if key.is_some() {
            return Err("Choose exactly one key after the modifiers.".into());
        }
        let code = match part.as_str() {
            "LEFT" => 0x25,
            "UP" => 0x26,
            "RIGHT" => 0x27,
            "DOWN" => 0x28,
            "SPACE" => 0x20,
            "ENTER" | "RETURN" => 0x0D,
            "TAB" => 0x09,
            "ESC" | "ESCAPE" => 0x1B,
            "HOME" => 0x24,
            "END" => 0x23,
            "PAGEUP" | "PGUP" => 0x21,
            "PAGEDOWN" | "PGDN" => 0x22,
            "INSERT" | "INS" => 0x2D,
            "DELETE" | "DEL" => 0x2E,
            "BACKSPACE" => 0x08,
            _ if part.len() == 1 && part.as_bytes()[0].is_ascii_alphanumeric() => part.as_bytes()[0] as u32,
            _ => {
                let function = part.strip_prefix('F').and_then(|n| n.parse::<u32>().ok());
                match function {
                    Some(n) if (1..=24).contains(&n) && part == format!("F{n}") => 0x6F + n,
                    _ => return Err("Use a letter, digit, F1–F24, arrow, or navigation key.".into()),
                }
            }
        };
        key = Some(code);
    }
    let key = key.ok_or_else(|| "Add a key after the modifiers (for example Ctrl+Alt+M).".to_owned())?;
    if modifiers == 0 {
        return Err("Include Ctrl, Alt, or Shift so ordinary typing stays available.".into());
    }
    if key == 0x7B {
        return Err("F12 is reserved by Windows; choose another key.".into());
    }
    if modifiers & 3 == 3 && key == 0x2E {
        return Err("Ctrl+Alt+Delete is reserved by Windows; choose another chord.".into());
    }
    Ok(Chord { modifiers, key })
}

#[derive(Default)]
pub struct HotkeyEditor {
    drafts: std::collections::HashMap<HotkeyAction, String>,
    initialized: bool,
    error: Option<String>,
}

fn primary_chord_for_action(
    action: HotkeyAction,
    bindings: &[HotkeyBinding],
    validated: &Validated,
    status: &HotkeyStatus,
) -> Option<String> {
    if let Some(active) = status
        .active
        .iter()
        .find(|b| b.action == action && bindings.iter().any(|raw| raw.action == action && raw.chord == b.chord))
    {
        return Some(active.chord.clone());
    }
    if let Some((valid, _)) = validated.valid.iter().find(|(b, _)| b.action == action) {
        return Some(valid.chord.clone());
    }
    bindings.iter().find(|b| b.action == action).map(|b| b.chord.clone())
}

impl HotkeyEditor {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        bindings: &mut Vec<HotkeyBinding>,
        step: &mut u8,
        service: &HotkeyService,
    ) {
        let status = service.status();
        if self.show_with_status(ui, bindings, step, &status)
            && let Err(error) = service.configure(bindings, *step)
        {
            self.error = Some(error);
        }
    }

    pub fn show_with_status(
        &mut self,
        ui: &mut egui::Ui,
        bindings: &mut Vec<HotkeyBinding>,
        step: &mut u8,
        status: &HotkeyStatus,
    ) -> bool {
        let validated = validate_bindings(bindings);
        if !self.initialized {
            for action in HotkeyAction::ALL {
                if let Some(chord) = primary_chord_for_action(action, bindings, &validated, status) {
                    self.drafts.entry(action).or_insert(chord);
                }
            }
            self.initialized = true;
        }
        let mut retry_clicked = false;
        ui.label("No shortcuts are assigned by default. Type a chord such as Ctrl+Alt+M, then Bind. Windows may reject keys used by another app.");
        ui.horizontal(|ui| {
            let label = ui.label("Volume step");
            crate::ui::widgets::scroll_on_focus(ui.add(egui::DragValue::new(step).range(1..=20).suffix("%")))
                .labelled_by(label.id)
                .on_hover_text("Output volume change for each volume shortcut, 1 to 20 percent");
        });
        for action in HotkeyAction::ALL {
            ui.push_id(action, |ui| {
                let current =
                    primary_chord_for_action(action, bindings, &validated, status).unwrap_or_else(|| "Off".into());
                let label = ui.label(format!("{} — {current}", action.label()));
                if current != "Off" {
                    let is_active = status.active.iter().any(|b| b.action == action && b.chord == current);
                    let mut consumed_primary_failure = false;
                    let state = if is_active {
                        "Active".to_owned()
                    } else if let Some(failure) =
                        status.failed.iter().find(|f| f.action == action && f.chord == current)
                    {
                        consumed_primary_failure = true;
                        format!("Not active: {}", failure.reason)
                    } else if status.pending {
                        "Registering…".to_owned()
                    } else {
                        "Not active".to_owned()
                    };
                    ui.add(egui::Label::new(egui::RichText::new(state).small()).wrap());
                    let mut skipped_primary = false;
                    for extra in status.failed.iter().filter(|f| f.action == action) {
                        if consumed_primary_failure && !skipped_primary && extra.chord == current {
                            skipped_primary = true;
                            continue;
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("Ignored duplicate ({}): {}", extra.chord, extra.reason))
                                    .small(),
                            )
                            .wrap(),
                        );
                    }
                }
                ui.horizontal(|ui| {
                    let draft = self.drafts.entry(action).or_default();
                    crate::ui::widgets::scroll_on_focus(
                        ui.add(egui::TextEdit::singleline(draft).hint_text("Ctrl+Alt+M").desired_width(140.0)),
                    )
                    .labelled_by(label.id)
                    .on_hover_text(format!("Shortcut for {}", action.label()));
                    // Short visible text keeps the row inside the card; the accessible
                    // name still says which action the button is for.
                    let bind = crate::ui::widgets::scroll_on_focus(ui.button("Bind"));
                    crate::ui::widgets::set_name(ui, &bind, &format!("Bind {}", action.label()));
                    if bind.on_hover_text(format!("Bind {}", action.label())).clicked() {
                        let mut proposed = bindings.clone();
                        proposed.retain(|b| b.action != action);
                        proposed.push(HotkeyBinding { action, chord: draft.trim().to_string() });
                        // Only this binding's own problems block it; another
                        // action's bad entry must not stop the user from binding.
                        match validate_bindings(&proposed).rejected.into_iter().find(|f| f.action == action) {
                            None => {
                                *bindings = proposed;
                                self.error = None;
                            }
                            Some(failure) => self.error = Some(failure.message()),
                        }
                    }
                    let clear = crate::ui::widgets::scroll_on_focus(
                        ui.add_enabled(current != "Off", egui::Button::new("Clear")),
                    );
                    crate::ui::widgets::set_name(ui, &clear, &format!("Clear {}", action.label()));
                    if clear.on_hover_text(format!("Clear {}", action.label())).clicked() {
                        bindings.retain(|b| b.action != action);
                        self.drafts.remove(&action);
                        self.error = None;
                    }
                });
            });
        }
        if let Some(error) = &self.error {
            ui.label(format!("Shortcut error: {error}"));
        }
        if status.pending {
            ui.label("Registering shortcuts with Windows…");
        }
        for error in &status.errors {
            ui.label(format!("Shortcut error: {error}"));
        }
        if !status.failed.is_empty() {
            ui.add(
                egui::Label::new(format!(
                    "{} shortcut entry(ies) are not active. Valid non-conflicting shortcuts remain active. If another app held a key, release it and click Retry failed shortcuts; use Bind or Clear to fix invalid or duplicate entries.",
                    status.failed.len()
                ))
                .wrap(),
            );
            if crate::ui::widgets::scroll_on_focus(
                ui.add_enabled(!status.pending, egui::Button::new("Retry failed shortcuts")),
            )
            .on_hover_text("Try to register every shortcut that is not active")
            .clicked()
            {
                retry_clicked = true;
            }
        }
        ui.label(format!("{} active shortcut(s)", status.active.len()));
        retry_clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(action: HotkeyAction, chord: &str) -> HotkeyBinding {
        HotkeyBinding { action, chord: chord.into() }
    }

    #[test]
    fn duplicate_chords_are_rejected_even_with_different_spelling() {
        let checked = validate_bindings(&[
            binding(HotkeyAction::VolumeUp, "Ctrl+Alt+A"),
            binding(HotkeyAction::VolumeDown, " alt + control + a "),
        ]);
        assert_eq!(checked.valid.len(), 1);
        assert_eq!(checked.valid[0].0.action, HotkeyAction::VolumeUp, "the earlier binding wins");
        assert_eq!(checked.rejected.len(), 1);
        assert_eq!(checked.rejected[0].action, HotkeyAction::VolumeDown);
    }

    #[test]
    fn one_invalid_binding_does_not_reject_the_valid_ones() {
        let checked = validate_bindings(&[
            binding(HotkeyAction::ToggleWindow, "Ctrl+Alt+F23"),
            binding(HotkeyAction::VolumeUp, "Ctrl+Alt+garbage"),
            binding(HotkeyAction::VolumeDown, "Ctrl+Alt+F22"),
        ]);
        let valid: Vec<_> = checked.valid.iter().map(|(b, _)| b.action).collect();
        assert_eq!(valid, [HotkeyAction::ToggleWindow, HotkeyAction::VolumeDown]);
        assert_eq!(checked.rejected.len(), 1);
        assert_eq!(checked.rejected[0].action, HotkeyAction::VolumeUp);
        assert!(checked.rejected[0].message().starts_with("Volume up (Ctrl+Alt+garbage): "));
    }

    #[test]
    fn duplicate_actions_keep_the_first_and_report_the_rest() {
        let checked = validate_bindings(&[
            binding(HotkeyAction::VolumeUp, "Ctrl+F1"),
            binding(HotkeyAction::VolumeUp, "Ctrl+F2"),
            binding(HotkeyAction::VolumeDown, "garbage"),
        ]);
        assert_eq!(checked.valid.len(), 1);
        assert_eq!(checked.rejected.len(), 2);
    }

    #[test]
    fn empty_configuration_disables_every_hotkey() {
        let checked = validate_bindings(&[]);
        assert!(checked.valid.is_empty() && checked.rejected.is_empty());
    }

    #[test]
    fn parses_modifiers_case_whitespace_and_key_aliases() {
        assert_eq!(parse_chord(" shift + CTRL + a ").unwrap(), Chord { modifiers: 6, key: 65 });
        assert_eq!(parse_chord("Alt+Left").unwrap(), Chord { modifiers: 1, key: 0x25 });
        assert_eq!(parse_chord("Ctrl+F24").unwrap(), Chord { modifiers: 2, key: 0x87 });
        assert_eq!(parse_chord("Ctrl+0").unwrap(), Chord { modifiers: 2, key: 48 });
        assert_eq!(parse_chord("Ctrl+PageDown"), parse_chord("Ctrl+PgDn"));
    }

    #[test]
    fn rejects_ambiguous_reserved_or_incomplete_chords() {
        for invalid in [
            "",
            "A",
            "Ctrl",
            "Ctrl+",
            "+A",
            "Ctrl+Ctrl+A",
            "Ctrl+A+B",
            "Win+A",
            "Ctrl+F0",
            "Ctrl+F25",
            "Ctrl+F12",
            "Ctrl+Ä",
            "Ctrl++",
            "Ctrl+F01",
            "Ctrl+Alt+Delete",
        ] {
            assert!(parse_chord(invalid).is_err(), "accepted invalid chord: {invalid}");
        }
    }

    fn lparam(chord: Chord) -> isize {
        ((chord.key << 16) | chord.modifiers) as isize
    }

    fn registrations_for(action: HotkeyAction, chord: &str, step: u8) -> Registrations {
        let parsed = parse_chord(chord).unwrap();
        Registrations {
            active: vec![Registration { id: 7, binding: binding(action, chord), chord: parsed }],
            next_id: 7,
            step,
        }
    }

    #[test]
    fn window_shortcut_is_queued_for_the_ui_thread_not_performed() {
        let chord = parse_chord("Ctrl+Alt+F23").unwrap();
        let registrations = registrations_for(HotkeyAction::ToggleWindow, "Ctrl+Alt+F23", 5);
        assert_eq!(registrations.resolve(7, lparam(chord)), Some(Resolved::Window(HotkeyEvent::ToggleWindow)));
        // Nothing in this module can touch a window: it does not import `platform::window`.
        assert!(!include_str!("hotkeys.rs").split("#[cfg(test)]").next().unwrap().contains("window::"));
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
    /// Never falls back to synthetic `PostThreadMessageW`: if UIPI or desktop state
    /// blocks `SendInput`, an error is returned so the caller cannot report real
    /// keyboard delivery when none occurred.
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
        let mut service = HotkeyService::start_with(|_| {}, egui::Context::default()).unwrap();
        service.configure(&[binding(HotkeyAction::ToggleWindow, "Ctrl+Alt+F23")], 5).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while service.status().pending {
            assert!(Instant::now() < deadline, "hotkey worker did not finish configuring");
            std::thread::sleep(Duration::from_millis(10));
        }
        if service.status().active.is_empty() {
            // If another process owns Ctrl+Alt+F23, clean up without false failure.
            service.shutdown();
            return;
        }
        let thread_id = service.thread_id;
        let took = exercise_shutdown_under_pressure(&mut service, Duration::from_millis(60), move || {
            let _ = unsafe { PostThreadMessageW(thread_id, WM_HOTKEY, WPARAM(1), LPARAM((0x86 << 16) | 3)) };
        });
        assert!(took < Duration::from_millis(1500), "shutdown took {took:?} under posted WM_HOTKEY pressure");
    }

    #[test]
    fn editor_presentation_handles_active_invalid_conflicting_duplicate_pending_and_cleared_states() {
        use crate::settings::Density;
        use crate::ui::test_support::Harness;

        let harness = Harness::new(Density::Comfortable);
        let mut editor = HotkeyEditor::default();
        // Hand-edited settings with:
        // - valid-first + duplicate-second for VolumeUp (Ctrl+Alt+F23 then Ctrl+Alt+F22),
        // - invalid-first + valid-second for ToggleWindow (garbage then Ctrl+F1),
        // - invalid VolumeDown (Ctrl+Alt+nonsense),
        // - conflicting ToggleOutputMute (Ctrl+Alt+M).
        let mut bindings = vec![
            binding(HotkeyAction::VolumeUp, "Ctrl+Alt+F23"),
            binding(HotkeyAction::VolumeUp, "Ctrl+Alt+F22"),
            binding(HotkeyAction::ToggleWindow, "garbage"),
            binding(HotkeyAction::ToggleWindow, "Ctrl+F1"),
            binding(HotkeyAction::VolumeDown, "Ctrl+Alt+nonsense"),
            binding(HotkeyAction::ToggleOutputMute, "Ctrl+Alt+M"),
        ];
        let mut step = 5_u8;
        let checked = validate_bindings(&bindings);
        let mut failed = checked.rejected;
        failed.push(FailedBinding {
            action: HotkeyAction::ToggleOutputMute,
            chord: "Ctrl+Alt+M".into(),
            reason: "Windows could not register this shortcut; another app may already use it.".into(),
        });
        let status = HotkeyStatus {
            active: vec![
                binding(HotkeyAction::VolumeUp, "Ctrl+Alt+F23"),
                binding(HotkeyAction::ToggleWindow, "Ctrl+F1"),
            ],
            failed,
            pending: false,
            errors: Vec::new(),
        };

        let frame = harness.run(360.0, 720.0, |ui| {
            let _ = editor.show_with_status(ui, &mut bindings, &mut step, &status);
        });
        let labels: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        // Valid-first duplicate binding (Ctrl+Alt+F23) is shown as header AND draft AND Active.
        assert!(labels.contains(&"Volume up — Ctrl+Alt+F23"), "primary duplicate must be shown: {labels:?}");
        assert!(
            labels.iter().filter(|&&l| l == "Active").count() == 2,
            "both active bindings (valid-first VolumeUp and invalid-first ToggleWindow) must show Active: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.starts_with("Ignored duplicate (Ctrl+Alt+F22):")),
            "rejected duplicate for the same action must be reported separately: {labels:?}"
        );
        // Invalid-first + valid-second (ToggleWindow: garbage then Ctrl+F1) selects Ctrl+F1 as
        // primary and reports garbage as an ignored duplicate entry.
        assert!(
            labels.contains(&"Show / hide mixer — Ctrl+F1"),
            "retained valid/active binding must be selected even when preceded by an invalid entry: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.starts_with("Ignored duplicate (garbage):")),
            "rejected invalid first entry must still be displayed alongside the active second entry: {labels:?}"
        );
        let vol_up_input = frame
            .widgets
            .iter()
            .find(|w| w.role == egui::accesskit::Role::TextInput && w.name == "Volume up — Ctrl+Alt+F23")
            .expect("Volume up text input");
        assert_eq!(
            vol_up_input.value.as_deref(),
            Some("Ctrl+Alt+F23"),
            "draft input must initialize with the first (active) binding, not the ignored duplicate"
        );
        let toggle_win_input = frame
            .widgets
            .iter()
            .find(|w| w.role == egui::accesskit::Role::TextInput && w.name == "Show / hide mixer — Ctrl+F1")
            .expect("Show / hide mixer text input");
        assert_eq!(
            toggle_win_input.value.as_deref(),
            Some("Ctrl+F1"),
            "draft input for invalid-first duplicate must initialize with the retained valid/active binding"
        );
        assert!(
            labels.iter().any(|l| l.contains("Not active: Use a letter, digit")),
            "invalid binding reason must be shown: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("Not active: Windows could not register this shortcut")),
            "conflicting binding reason must be shown: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("use Bind or Clear to fix invalid or duplicate entries")),
            "summary banner must distinguish retryable conflicts from invalid/duplicate entries: {labels:?}"
        );

        // Pending state shows Registering… and disables Retry.
        let pending_status =
            HotkeyStatus { active: Vec::new(), failed: status.failed.clone(), pending: true, errors: Vec::new() };
        let pending_frame = harness.run(360.0, 720.0, |ui| {
            let _ = editor.show_with_status(ui, &mut bindings, &mut step, &pending_status);
        });
        assert!(pending_frame.widgets.iter().any(|w| w.name == "Registering shortcuts with Windows…"));
        let retry_btn =
            pending_frame.widgets.iter().find(|w| w.name == "Retry failed shortcuts").expect("Retry button");
        assert!(retry_btn.disabled, "Retry button must be disabled while registration is pending");

        // Cleared state disables Clear buttons and shows Off.
        bindings.clear();
        let cleared_status = HotkeyStatus::default();
        let cleared_frame = harness.run(360.0, 720.0, |ui| {
            let _ = editor.show_with_status(ui, &mut bindings, &mut step, &cleared_status);
        });
        let clear_vol_up =
            cleared_frame.widgets.iter().find(|w| w.name == "Clear Volume up").expect("Clear Volume up button");
        assert!(clear_vol_up.disabled, "Clear button must be disabled when action is Off");
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
