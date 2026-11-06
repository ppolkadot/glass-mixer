//! Audio subsystem.
//!
//! All Core Audio objects live on one dedicated MTA thread (the *engine*). The UI
//! talks to it through [`AudioHandle`]: commands go in over a channel, and the
//! engine publishes immutable [`AudioSnapshot`]s that it re-reads from Windows
//! after every change. Windows remains the single source of truth; the engine
//! never keeps its own copy of volume levels.

pub mod app_routing;
pub mod attenuation;
pub mod automation;
mod callbacks;
pub(crate) mod com;
mod engine;
pub mod model;
mod policy_config;
mod power;
pub mod preferences;
mod process;
pub mod properties;

pub use model::*;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

/// Requests from the UI (or tray) to the audio engine.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    ToggleInputMute,
    AdjustOutputVolume {
        delta: f32,
    },
    /// Applies a profile from the Profiles view.
    ApplyPreset(preferences::VolumePreset),
    /// Applies a profile from its shortcut; the result is shown in the tray.
    ActivateProfile(preferences::VolumePreset),
    SetPresetRules(Vec<preferences::VolumePreset>),
    OpenSoundSettings,
    OpenMonoSettings,
    /// Lazily open all active endpoints for independent controls.
    LoadDevices,
    SetDeviceVolume {
        id: String,
        volume: f32,
    },
    SetDeviceMute {
        id: String,
        muted: bool,
    },
    SetDeviceBalance {
        id: String,
        balance: f32,
    },
    SetEndpointVolume {
        flow: Flow,
        volume: f32,
    },
    SetEndpointMute {
        flow: Flow,
        muted: bool,
    },
    SetEndpointBalance {
        flow: Flow,
        balance: f32,
    },
    ToggleOutputMute,
    SetDefaultDevice {
        flow: Flow,
        id: String,
    },
    SetAppVolume {
        key: AppKey,
        volume: f32,
    },
    SetAppMute {
        key: AppKey,
        muted: bool,
    },
    SetAppBalance {
        key: AppKey,
        balance: f32,
    },
    /// Sets one session's volume (see [`StreamEntry::id`]).
    SetStreamVolume {
        id: String,
        volume: f32,
    },
    SetStreamMute {
        id: String,
        muted: bool,
    },
    /// Sets the output device of every process of the app; `None` resets it to
    /// the system default.
    SetAppOutput {
        key: AppKey,
        device: Option<String>,
    },
    /// Resets every app's device preference in Windows, including apps Glass
    /// Mixer never changed. The UI asks for confirmation first.
    ResetAllAppDevices,
    /// Apps lowered while another app is in the foreground. Empty turns it off.
    SetFocusRules(Vec<(AppKey, attenuation::Reduction)>),
    /// The process that owns the foreground window (from the foreground watcher).
    ForegroundChanged {
        pid: u32,
    },
    /// Lowers the target apps while the microphone picks up speech; `None` turns it off.
    SetMicDucking(Option<MicDucking>),
    /// Mutes every session of the app, or unmutes them all if all are muted.
    ToggleAppMute {
        key: AppKey,
    },
    /// Changes the app's volume (its loudest session's) by `delta`, for all sessions.
    AdjustAppVolume {
        key: AppKey,
        delta: f32,
    },
    /// The output devices `CycleOutputDevice` switches between, in order.
    SetPinnedOutputs(Vec<String>),
    /// Makes the next connected pinned output the default.
    CycleOutputDevice,
    /// Starts Solo for the app, or ends it if that app is already soloed.
    ToggleSolo {
        key: AppKey,
    },
    EndSolo,
    /// How Solo lowers the other apps: `factor` of their volume, or muted.
    SetSoloOptions {
        factor: f32,
        mute: bool,
    },
    /// Tear down and rebuild every Core Audio object.
    Refresh,
}

/// Microphone ducking: which apps are lowered, how much, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct MicDucking {
    pub gate: automation::MicGateConfig,
    pub reduction: attenuation::Reduction,
    pub targets: Vec<AppKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// A short user-facing message produced by the engine.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub level: NoticeLevel,
    pub text: String,
    /// The result of something the user just did (for example a shortcut), worth
    /// showing in the tray even as information while the window is hidden.
    pub announce: bool,
}

/// Messages processed by the engine thread: UI commands plus notifications
/// forwarded from Windows callbacks.
#[derive(Debug)]
pub(crate) enum Msg {
    Cmd(Command),
    /// A device was added/removed/changed state, or a device property changed.
    DevicesChanged,
    DefaultDeviceChanged,
    EndpointVolumeChanged,
    /// A session was created, expired or disconnected.
    SessionsChanged,
    /// A session's volume, mute, state or name changed.
    SessionValuesChanged,
    /// The machine resumed from sleep.
    Resumed,
    /// UI visibility changed (controls meter polling).
    Wake,
    Shutdown,
}

pub(crate) type Listener = Arc<dyn Fn() + Send + Sync>;
type EngineThread = (JoinHandle<()>, Receiver<()>);

pub(crate) struct Shared {
    snapshot: Mutex<Arc<AudioSnapshot>>,
    meters: Mutex<Arc<Meters>>,
    notices: Mutex<VecDeque<Notice>>,
    listeners: Mutex<Vec<Listener>>,
    meters_wanted: AtomicBool,
}

impl Shared {
    pub(crate) fn publish(&self, snapshot: AudioSnapshot) {
        {
            let mut current = self.snapshot.lock();
            if **current == snapshot {
                return;
            }
            *current = Arc::new(snapshot);
        }
        self.notify();
    }

    pub(crate) fn publish_meters(&self, meters: Arc<Meters>) {
        *self.meters.lock() = meters;
        self.notify();
    }

    pub(crate) fn push_notice(&self, level: NoticeLevel, text: impl Into<String>) {
        self.push(level, text.into(), false);
    }

    /// An information notice the tray shows while the window is hidden.
    pub(crate) fn push_announcement(&self, text: impl Into<String>) {
        self.push(NoticeLevel::Info, text.into(), true);
    }

    fn push(&self, level: NoticeLevel, text: String, announce: bool) {
        match level {
            NoticeLevel::Error => log::error!("{text}"),
            NoticeLevel::Warning => log::warn!("{text}"),
            NoticeLevel::Info => log::info!("{text}"),
        }
        {
            let mut q = self.notices.lock();
            if q.back().is_some_and(|n| n.text == text) {
                return;
            }
            q.push_back(Notice { level, text, announce });
            while q.len() > 8 {
                q.pop_front();
            }
        }
        self.notify();
    }

    pub(crate) fn meters_wanted(&self) -> bool {
        self.meters_wanted.load(Ordering::Relaxed)
    }

    fn notify(&self) {
        let listeners = self.listeners.lock().clone();
        for l in listeners {
            l();
        }
    }
}

/// Cheap, cloneable handle to the audio engine thread.
#[derive(Clone)]
pub struct AudioHandle {
    tx: Sender<Msg>,
    shared: Arc<Shared>,
    thread: Arc<Mutex<Option<EngineThread>>>,
}

impl AudioHandle {
    /// Starts the engine thread. Initialization happens asynchronously; the
    /// first snapshot has [`ServiceStatus::Starting`].
    pub fn start() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Arc::new(AudioSnapshot::default())),
            meters: Mutex::new(Arc::default()),
            notices: Mutex::new(VecDeque::new()),
            listeners: Mutex::new(Vec::new()),
            meters_wanted: AtomicBool::new(false),
        });
        let thread = {
            let shared = shared.clone();
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("audio-engine".into())
                .spawn(move || {
                    engine::run(rx, tx, shared);
                    let _ = done_tx.send(());
                })
                .expect("failed to spawn audio thread")
        };
        Self { tx, shared, thread: Arc::new(Mutex::new(Some((thread, done_rx)))) }
    }

    /// A handle with no engine behind it, for UI tests: commands are accepted and dropped.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Arc::new(AudioSnapshot::default())),
            meters: Mutex::new(Arc::default()),
            notices: Mutex::new(VecDeque::new()),
            listeners: Mutex::new(Vec::new()),
            meters_wanted: AtomicBool::new(false),
        });
        Self { tx, shared, thread: Arc::new(Mutex::new(None)) }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(Msg::Cmd(cmd));
    }

    pub fn snapshot(&self) -> Arc<AudioSnapshot> {
        self.shared.snapshot.lock().clone()
    }

    /// The latest peak levels; cheap to call every frame.
    pub fn meters(&self) -> Arc<Meters> {
        self.shared.meters.lock().clone()
    }

    pub fn take_notices(&self) -> Vec<Notice> {
        self.shared.notices.lock().drain(..).collect()
    }

    /// Registers a callback invoked (from the engine thread) whenever new state is published.
    pub fn add_listener(&self, f: impl Fn() + Send + Sync + 'static) {
        self.shared.listeners.lock().push(Arc::new(f));
    }

    /// Enables peak-meter polling. Only enable while the window is visible.
    pub fn set_meters_wanted(&self, wanted: bool) {
        if self.shared.meters_wanted.swap(wanted, Ordering::Relaxed) != wanted {
            let _ = self.tx.send(Msg::Wake);
        }
    }

    /// Stops the engine, unregistering all callbacks and releasing COM objects.
    /// Waits up to two seconds for the thread to finish.
    pub fn shutdown(&self) {
        let Some((handle, done)) = self.thread.lock().take() else {
            return;
        };
        let _ = self.tx.send(Msg::Shutdown);
        match done.recv_timeout(Duration::from_secs(2)) {
            Ok(()) => {
                let _ = handle.join();
                log::info!("audio engine stopped");
            }
            Err(_) => log::warn!("audio engine did not stop within 2s; detaching"),
        }
    }
}
