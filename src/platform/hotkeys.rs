//! Optional global shortcuts. A dedicated Win32 message loop sleeps in
//! `GetMessageW`; configuration and shutdown wake it with thread messages.

use std::{sync::Arc, thread::JoinHandle};

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

use super::window::{self, Hwnd};
use crate::audio::{AudioHandle, Command};

const CONFIGURE_MESSAGE: u32 = WM_APP + 71;

/// Requested settings and active registrations can differ after a conflict.
#[derive(Clone, Debug, Default)]
pub struct HotkeyStatus {
    pub active: Vec<HotkeyBinding>,
    pub errors: Vec<String>,
    pub pending: bool,
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
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
    status: Arc<Mutex<SharedStatus>>,
}

impl HotkeyService {
    /// Starts with no registered shortcuts. Call `configure` to opt in.
    pub fn start(audio: AudioHandle, hwnd: Hwnd, context: egui::Context) -> Result<Self, String> {
        let (tx, rx) = crossbeam_channel::unbounded();
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
                run(rx, audio, hwnd, &context, &worker_status);
            })
            .map_err(|error| format!("Could not start global shortcuts: {error}"))?;
        match ready_rx.recv() {
            Ok(thread_id) => Ok(Self { tx, thread_id, thread: Some(thread), status }),
            Err(error) => {
                let _ = thread.join();
                Err(format!("Global shortcuts stopped during startup: {error}"))
            }
        }
    }

    /// Enqueues a change; `status()` reports the exact active set and any
    /// registration failures when the worker finishes. The previous working
    /// configuration remains active if validation or registration is rejected.
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

    pub fn shutdown(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        let _ = self.tx.send(Request::Shutdown);
        // If the Windows queue is full, its next message also drains requests.
        // If the thread is already gone, joining it is immediate.
        let _ = unsafe { PostThreadMessageW(self.thread_id, CONFIGURE_MESSAGE, WPARAM(0), LPARAM(0)) };
        if thread.join().is_err() {
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
    fn new(id: i32, binding: HotkeyBinding, chord: Chord) -> Result<Self, String> {
        unsafe { RegisterHotKey(None, id, HOT_KEY_MODIFIERS(chord.modifiers) | MOD_NOREPEAT, chord.key) }
            .map_err(|error| format!("{} ({}): Windows could not register this shortcut ({error}). It may be reserved or used by another app; choose another chord or release it in that app.", binding.action.label(), binding.chord))?;
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

    fn configure(&mut self, bindings: &[HotkeyBinding], step: u8) -> Result<(), Vec<String>> {
        let parsed = validate_bindings(bindings)?;
        let mut added = Vec::new();
        let mut errors = Vec::new();
        for (binding, chord) in &parsed {
            if self.active.iter().any(|registration| registration.chord == *chord) {
                continue;
            }
            // Application hotkey IDs must be in 0x0000..=0xBFFF.
            loop {
                self.next_id = self.next_id % 0xBFFF + 1;
                if !self.active.iter().chain(&added).any(|r| r.id == self.next_id) {
                    break;
                }
            }
            match Registration::new(self.next_id, binding.clone(), *chord) {
                Ok(registration) => added.push(registration),
                Err(error) => errors.push(error),
            }
        }
        if !errors.is_empty() {
            // Drop only the tentative additions. Old registrations have not
            // been touched, including chords reassigned to another action.
            return Err(errors);
        }
        self.active.retain(|registration| parsed.iter().any(|(_, chord)| registration.chord == *chord));
        self.active.extend(added);
        for registration in &mut self.active {
            if let Some((binding, _)) = parsed.iter().find(|(_, chord)| *chord == registration.chord) {
                registration.binding = binding.clone();
            }
        }
        self.step = step.clamp(1, 100);
        Ok(())
    }

    fn dispatch(&self, message: &MSG, audio: &AudioHandle, hwnd: Hwnd) {
        let Some(registration) = self.active.iter().find(|r| r.id as usize == message.wParam.0) else {
            return;
        };
        // Ignore a queued message belonging to an old registration if an ID
        // eventually wraps and is reused for a different chord.
        let modifiers = message.lParam.0 as u32 & 7;
        let key = (message.lParam.0 as u32 >> 16) & 0xFFFF;
        if registration.chord != (Chord { modifiers, key }) {
            return;
        }
        let delta = self.step as f32 / 100.0;
        match registration.binding.action {
            HotkeyAction::ToggleOutputMute => audio.send(Command::ToggleOutputMute),
            HotkeyAction::ToggleInputMute => audio.send(Command::ToggleInputMute),
            HotkeyAction::VolumeUp => audio.send(Command::AdjustOutputVolume { delta }),
            HotkeyAction::VolumeDown => audio.send(Command::AdjustOutputVolume { delta: -delta }),
            HotkeyAction::ToggleWindow => {
                log::debug!("global shortcut toggling window {:?}, shown={}", hwnd, window::is_shown(hwnd));
                if window::is_shown(hwnd) {
                    window::hide(hwnd);
                } else {
                    window::show(hwnd);
                }
                log::debug!("global shortcut window now shown={}", window::is_shown(hwnd));
            }
        }
    }
}

fn run(rx: Receiver<Request>, audio: AudioHandle, hwnd: Hwnd, context: &egui::Context, status: &Mutex<SharedStatus>) {
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
        for request in rx.try_iter() {
            match request {
                Request::Shutdown => break 'messages,
                Request::Configure { bindings, step, revision } => {
                    let errors = registrations.configure(&bindings, step).err().unwrap_or_default();
                    if errors.is_empty() {
                        log::info!("registered {} global shortcuts", registrations.active.len());
                    } else {
                        log::warn!("global shortcut registration failed: {}", errors.join("; "));
                    }
                    let mut shared = status.lock();
                    shared.current.active = registrations.bindings();
                    if shared.revision == revision {
                        shared.current.errors = errors;
                        shared.current.pending = false;
                    }
                    context.request_repaint();
                }
            }
        }
        if message.message == WM_HOTKEY {
            log::debug!(
                "global shortcut message: id={}, modifiers={}, key={}",
                message.wParam.0,
                message.lParam.0 as u32 & 0xffff,
                (message.lParam.0 as u32 >> 16) & 0xffff
            );
            registrations.dispatch(&message, &audio, hwnd);
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

fn validate_bindings(bindings: &[HotkeyBinding]) -> Result<Vec<(HotkeyBinding, Chord)>, Vec<String>> {
    let mut parsed = Vec::new();
    let mut errors = Vec::new();
    let mut actions = std::collections::HashSet::new();
    let mut chords = std::collections::HashMap::new();
    for binding in bindings {
        if !actions.insert(binding.action) {
            errors.push(format!("{}: choose only one shortcut for this action.", binding.action.label()));
        }
        match parse_chord(&binding.chord) {
            Ok(chord) => {
                if let Some(other) = chords.insert(chord, binding.action) {
                    errors.push(format!(
                        "{}: {} is also assigned to {}. Choose a different chord.",
                        binding.action.label(),
                        binding.chord,
                        other.label()
                    ));
                }
                parsed.push((binding.clone(), chord));
            }
            Err(error) => errors.push(format!("{} ({}): {error}", binding.action.label(), binding.chord)),
        }
    }
    if errors.is_empty() { Ok(parsed) } else { Err(errors) }
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

impl HotkeyEditor {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        bindings: &mut Vec<HotkeyBinding>,
        step: &mut u8,
        service: &HotkeyService,
    ) {
        if !self.initialized {
            for binding in bindings.iter() {
                self.drafts.insert(binding.action, binding.chord.clone());
            }
            self.initialized = true;
        }
        ui.label("No shortcuts are assigned by default. Type a chord such as Ctrl+Alt+M, then Bind. Windows may reject keys used by another app.");
        ui.horizontal(|ui| {
            ui.label("Volume step");
            ui.add(egui::DragValue::new(step).range(1..=20).suffix("%"))
                .on_hover_text("Output volume change for each volume shortcut, 1 to 20 percent");
        });
        for action in HotkeyAction::ALL {
            ui.push_id(action, |ui| {
                let current = bindings
                    .iter()
                    .find(|b| b.action == action)
                    .map(|b| b.chord.clone())
                    .unwrap_or_else(|| "Off".into());
                ui.label(format!("{} — {current}", action.label()));
                ui.horizontal(|ui| {
                    let draft = self.drafts.entry(action).or_default();
                    ui.add(egui::TextEdit::singleline(draft).hint_text("Ctrl+Alt+M").desired_width(140.0))
                        .on_hover_text(format!("Shortcut for {}", action.label()));
                    if ui.button(format!("Bind {}", action.label())).clicked() {
                        let mut proposed = bindings.clone();
                        proposed.retain(|b| b.action != action);
                        proposed.push(HotkeyBinding { action, chord: draft.trim().to_string() });
                        match validate_bindings(&proposed) {
                            Ok(_) => {
                                *bindings = proposed;
                                self.error = None;
                            }
                            Err(errors) => self.error = Some(errors.join(" ")),
                        }
                    }
                    if ui
                        .add_enabled(current != "Off", egui::Button::new(format!("Clear {}", action.label())))
                        .clicked()
                    {
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
        let status = service.status();
        if status.pending {
            ui.label("Registering shortcuts with Windows…");
        }
        for error in &status.errors {
            ui.label(format!("Shortcut error: {error}"));
        }
        ui.label(format!("{} active shortcut(s)", status.active.len()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_chords_even_with_different_spelling() {
        let bindings = [
            HotkeyBinding { action: HotkeyAction::VolumeUp, chord: "Ctrl+Alt+A".into() },
            HotkeyBinding { action: HotkeyAction::VolumeDown, chord: " alt + control + a ".into() },
        ];
        assert!(validate_bindings(&bindings).is_err());
    }

    #[test]
    fn rejects_duplicate_actions_and_reports_all_invalid_entries() {
        let bindings = [
            HotkeyBinding { action: HotkeyAction::VolumeUp, chord: "Ctrl+F1".into() },
            HotkeyBinding { action: HotkeyAction::VolumeUp, chord: "Ctrl+F2".into() },
            HotkeyBinding { action: HotkeyAction::VolumeDown, chord: "garbage".into() },
        ];
        assert_eq!(validate_bindings(&bindings).unwrap_err().len(), 2);
    }

    #[test]
    fn empty_configuration_disables_every_hotkey() {
        assert_eq!(validate_bindings(&[]).unwrap(), Vec::new());
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

    #[test]
    #[ignore = "registers a real Windows global shortcut; unregisters before returning"]
    fn registration_reports_conflicts_and_releases_key() {
        let requested = [HotkeyBinding { action: HotkeyAction::VolumeUp, chord: "Ctrl+Alt+F23".into() }];
        let mut ours = Registrations::default();
        ours.configure(&requested, 5).unwrap();
        let mut second = Registrations { next_id: 100, ..Default::default() };
        assert!(second.configure(&requested, 5).is_err());
        drop(ours);
        second.configure(&requested, 5).unwrap();
    }
}
