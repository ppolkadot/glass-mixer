//! Audio subsystem.
//!
//! All Core Audio objects live on one dedicated MTA thread (the *engine*). The UI
//! talks to it through [`AudioHandle`]: commands go in over a channel, and the
//! engine publishes immutable [`AudioSnapshot`]s that it re-reads from Windows
//! after every change. Windows remains the single source of truth; the engine
//! never keeps its own copy of volume levels.

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
    ApplyPreset(preferences::VolumePreset),
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
    /// Tear down and rebuild every Core Audio object.
    Refresh,
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
    meters: Mutex<Meters>,
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

    pub(crate) fn publish_meters(&self, meters: Meters) {
        *self.meters.lock() = meters;
        self.notify();
    }

    pub(crate) fn push_notice(&self, level: NoticeLevel, text: impl Into<String>) {
        let text = text.into();
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
            q.push_back(Notice { level, text });
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
            meters: Mutex::new(Meters::default()),
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

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(Msg::Cmd(cmd));
    }

    pub fn snapshot(&self) -> Arc<AudioSnapshot> {
        self.shared.snapshot.lock().clone()
    }

    pub fn meters(&self) -> Meters {
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
