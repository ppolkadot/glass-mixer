//! The audio engine thread: owns every Core Audio object, applies commands and
//! publishes snapshots re-read from Windows.

use super::app_routing::AppRouting;
use super::attenuation::{Ledger, Level, Reduction};
use super::automation::{FocusSettle, MicGate};
use super::callbacks::{
    DeviceNotifier, EndpointVolumeNotifier, OWN_EVENT_CONTEXT, SessionCreatedNotifier, SessionEvents,
};
use super::com::{ComGuard, is_fatal, is_not_found, take_pwstr};
use super::model::*;
use super::policy_config::PolicyConfig;
use super::power::PowerWatch;
use super::process::{self, ProcessInfo, ProcessWatch};
use super::{Command, Msg, NoticeLevel, Shared};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, DEVICE_STATE_ACTIVE, EDataFlow, IAudioSessionControl2,
    IAudioSessionEvents, IAudioSessionManager2, IAudioSessionNotification, IChannelAudioVolume, IMMDevice,
    IMMDeviceEnumerator, IMMNotificationClient, ISimpleAudioVolume, MMDeviceEnumerator, eCapture, eConsole, eRender,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, STGM_READ};
use windows::Win32::UI::Shell::SHLoadIndirectString;
use windows::core::{HSTRING, Interface};

const METER_INTERVAL: Duration = Duration::from_millis(33);
const RESUME_SETTLE: Duration = Duration::from_millis(1500);
/// Device add/remove/state/property notifications arrive in bursts (a Bluetooth
/// reconnect sends several); they are merged into one re-enumeration.
const DEVICE_COALESCE: Duration = Duration::from_millis(50);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// How often process markers are checked for exits that were never observed,
/// piggybacked on session refreshes (there is no timer).
const RECLAIM_INTERVAL: Duration = Duration::from_secs(300);
/// The same check while the tracker is large or saturated.
const RECLAIM_RETRY: Duration = Duration::from_secs(10);
/// How long after a shortcut's device switch the resulting "Output switched"
/// notice is still treated as its answer (and shown in the tray).
const ANNOUNCE_WINDOW: Duration = Duration::from_secs(3);
/// Focus has to stay on an app this long before focus rules react.
const FOCUS_SETTLE: Duration = Duration::from_millis(250);
/// How often the microphone level is read while microphone ducking is on.
const MIC_INTERVAL: Duration = Duration::from_millis(50);

type WinResult<T> = windows::core::Result<T>;

fn current_filetime() -> u64 {
    const UNIX_TO_FILETIME_SECONDS: u64 = 11_644_473_600;
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| (t.as_secs() + UNIX_TO_FILETIME_SECONDS) * 10_000_000 + (t.subsec_nanos() as u64 / 100))
        .unwrap_or(u64::MAX)
}

fn open_settings(uri: &str) -> Result<(), String> {
    use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let result =
        unsafe { ShellExecuteW(None, windows::core::w!("open"), &HSTRING::from(uri), None, None, SW_SHOWNORMAL) };
    if result.0 as isize <= 32 {
        Err("Couldn't open Windows Settings. Open Settings > System > Sound manually.".into())
    } else {
        Ok(())
    }
}

/// A deadline set by the first event of a burst. Later events in the window
/// don't extend it, so a steady stream still refreshes every window.
#[derive(Default)]
struct Coalesce {
    due: Option<Instant>,
}

impl Coalesce {
    fn note(&mut self, now: Instant, window: Duration) {
        self.due.get_or_insert(now + window);
    }

    /// Whether the window has elapsed; resets it if so.
    fn take_due(&mut self, now: Instant) -> bool {
        let due = self.due.is_some_and(|t| now >= t);
        if due {
            self.due = None;
        }
        due
    }

    /// A refresh that already covers the pending events.
    fn cancel(&mut self) {
        self.due = None;
    }
}

#[derive(Default, Clone, Copy)]
struct Dirty {
    rebuild: bool,
    devices: bool,
    sessions: bool,
    values: bool,
}

impl Dirty {
    fn any(&self) -> bool {
        self.rebuild || self.devices || self.sessions || self.values
    }
}

pub(crate) fn run(rx: Receiver<Msg>, tx: Sender<Msg>, shared: Arc<Shared>) {
    let _com = ComGuard::multithreaded();
    let _power = PowerWatch::register(tx.clone());
    let mut runner = Runner {
        engine: None,
        tx,
        shared,
        dirty: Dirty::default(),
        retry_at: Some(Instant::now()),
        backoff: Duration::from_millis(250),
        delayed_rebuild: None,
        device_events: Coalesce::default(),
        last_meter_poll: Instant::now(),
        last_meters: Arc::default(),
        meter_scratch: Meters::default(),
        was_unavailable: false,
        devices_loaded: false,
        preset_rules: Vec::new(),
        pinned_outputs: Vec::new(),
        solo_level: Reduction { factor: 0.2, mute: false },
        focus_rules: Vec::new(),
        focus: FocusSettle::default(),
        mic: None,
        mic_gate: MicGate::default(),
        last_mic_poll: Instant::now(),
    };

    loop {
        runner.maintain();
        let first = match runner.next_timeout() {
            Some(t) => match rx.recv_timeout(t) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => break,
            },
        };
        let mut msgs: Vec<Msg> = first.into_iter().collect();
        msgs.extend(rx.try_iter());
        if !runner.handle(msgs) {
            break;
        }
        runner.poll_meters();
        runner.automation_tick();
    }
    // Unregisters every callback and releases COM objects before COM is uninitialized.
    runner.engine = None;
    log::info!("audio engine thread exiting");
}

struct Runner {
    engine: Option<Engine>,
    tx: Sender<Msg>,
    shared: Arc<Shared>,
    dirty: Dirty,
    retry_at: Option<Instant>,
    backoff: Duration,
    delayed_rebuild: Option<Instant>,
    device_events: Coalesce,
    last_meter_poll: Instant,
    last_meters: Arc<Meters>,
    /// Reused for every poll, so a steady set of sessions polls without allocating.
    meter_scratch: Meters,
    was_unavailable: bool,
    devices_loaded: bool,
    preset_rules: Vec<super::preferences::VolumePreset>,
    pinned_outputs: Vec<String>,
    solo_level: Reduction,
    focus_rules: Vec<(AppKey, Reduction)>,
    focus: FocusSettle<AppKey>,
    mic: Option<super::MicDucking>,
    mic_gate: MicGate,
    last_mic_poll: Instant,
}

impl Runner {
    /// Returns `false` when the engine should stop.
    fn handle(&mut self, msgs: Vec<Msg>) -> bool {
        let mut commands: Vec<Command> = Vec::new();
        for msg in msgs {
            match msg {
                Msg::Shutdown => return false,
                Msg::Cmd(cmd) => push_coalesced(&mut commands, cmd),
                Msg::DevicesChanged => self.device_events.note(Instant::now(), DEVICE_COALESCE),
                // The default device decides which sessions are shown, so it is never delayed.
                Msg::DefaultDeviceChanged => self.dirty.devices = true,
                Msg::EndpointVolumeChanged | Msg::SessionValuesChanged => self.dirty.values = true,
                Msg::SessionsChanged => self.dirty.sessions = true,
                Msg::Resumed => {
                    log::info!("resumed from sleep; scheduling audio rebuild");
                    self.delayed_rebuild = Some(Instant::now() + RESUME_SETTLE);
                }
                Msg::Wake => {}
            }
        }
        for cmd in commands {
            self.apply(cmd);
        }
        self.maintain();
        true
    }

    fn apply(&mut self, cmd: Command) {
        match &cmd {
            Command::SetPinnedOutputs(ids) => {
                self.pinned_outputs.clone_from(ids);
                if let Some(engine) = self.engine.as_mut() {
                    engine.pinned_outputs.clone_from(ids);
                }
                return;
            }
            Command::SetFocusRules(rules) => {
                self.focus_rules.clone_from(rules);
                if rules.is_empty() {
                    self.focus.clear();
                }
                self.sync_automation();
                return;
            }
            Command::ForegroundChanged { pid } => {
                if let Some(engine) = self.engine.as_ref()
                    && !self.focus_rules.is_empty()
                {
                    self.focus.report(engine.key_for_pid(*pid), Instant::now());
                }
                return;
            }
            Command::SetMicDucking(config) => {
                self.mic.clone_from(config);
                self.mic_gate.reset();
                self.sync_automation();
                return;
            }
            Command::SetSoloOptions { factor, mute } => {
                self.solo_level = Reduction { factor: factor.clamp(0.0, 1.0), mute: *mute };
                if let Some(engine) = self.engine.as_mut() {
                    engine.reductions.solo_level = self.solo_level;
                    self.dirty.values = true;
                }
                return;
            }
            Command::CycleOutputDevice | Command::ToggleSolo { .. } | Command::EndSolo => {
                let Some(engine) = self.engine.as_mut() else {
                    self.shared.push_notice(NoticeLevel::Error, "The Windows audio service isn't available.");
                    return;
                };
                let result = match &cmd {
                    Command::CycleOutputDevice => engine.cycle_output(),
                    Command::ToggleSolo { key } => engine.toggle_solo(key),
                    _ => engine.end_solo(),
                };
                match result {
                    Ok(Some(announcement)) => self.shared.push_announcement(announcement),
                    Ok(None) => {}
                    Err(err) => {
                        if let ApplyError::Windows(_, e) = &err
                            && is_fatal(e.code())
                        {
                            self.dirty.rebuild = true;
                        }
                        self.shared.push_notice(NoticeLevel::Error, err.to_string());
                    }
                }
                self.dirty.values = true;
                self.dirty.devices |= matches!(cmd, Command::CycleOutputDevice);
                return;
            }
            _ => {}
        }
        if let Command::SetPresetRules(presets) = &cmd {
            self.preset_rules = presets.clone();
            if let Some(engine) = self.engine.as_mut() {
                engine.set_preset_rules(presets.clone());
            }
            return;
        }
        if let Command::ApplyPreset(preset) | Command::ActivateProfile(preset) = &cmd {
            let Some(engine) = self.engine.as_mut() else {
                self.shared.push_notice(NoticeLevel::Error, "Audio service unavailable; the profile was not applied.");
                return;
            };
            match engine.apply_profile(preset, &self.shared) {
                Ok(outcome) => {
                    self.dirty.values = true;
                    self.dirty.devices |= outcome.devices_switched;
                    let (level, text) = outcome.notice(&preset.name);
                    if level == NoticeLevel::Info && matches!(cmd, Command::ActivateProfile(_)) {
                        self.shared.push_announcement(text);
                    } else {
                        self.shared.push_notice(level, text);
                    }
                }
                Err(err) => {
                    if let ApplyError::Windows(_, e) = &err
                        && is_fatal(e.code())
                    {
                        self.dirty.rebuild = true;
                    }
                    self.dirty.values = true;
                    self.shared.push_notice(NoticeLevel::Error, err.to_string());
                }
            }
            return;
        }
        if matches!(cmd, Command::OpenSoundSettings | Command::OpenMonoSettings) {
            let uri = if cmd == Command::OpenSoundSettings {
                super::properties::SOUND_SETTINGS_URI
            } else {
                super::properties::MONO_SETTINGS_URI
            };
            if let Err(e) = open_settings(uri) {
                self.shared.push_notice(NoticeLevel::Error, e);
            }
            return;
        }
        if cmd == Command::LoadDevices {
            self.devices_loaded = true;
        }
        if cmd == Command::Refresh {
            log::info!("manual audio refresh requested");
            self.dirty.rebuild = true;
            return;
        }
        let Some(engine) = self.engine.as_mut() else {
            self.shared.push_notice(
                NoticeLevel::Error,
                "The Windows audio service isn't available, so the change wasn't applied.",
            );
            return;
        };
        match engine.apply(&cmd) {
            Ok(dirty_devices) => {
                self.dirty.values = true;
                self.dirty.devices |= dirty_devices;
            }
            Err(err) => {
                if let ApplyError::Windows(_, e) = &err
                    && is_fatal(e.code())
                {
                    self.dirty.rebuild = true;
                }
                // Re-read so the UI shows the real value, not the rejected one.
                self.dirty.values = true;
                self.dirty.sessions = true;
                self.shared.push_notice(NoticeLevel::Error, err.to_string());
            }
        }
    }

    fn maintain(&mut self) {
        let now = Instant::now();
        if self.delayed_rebuild.is_some_and(|t| now >= t) {
            self.delayed_rebuild = None;
            self.dirty.rebuild = true;
        }
        if self.device_events.take_due(now) {
            self.dirty.devices = true;
        }
        if self.dirty.rebuild {
            self.dirty = Dirty::default();
            self.engine = None;
            self.retry_at = Some(now);
        }

        if self.engine.is_none() {
            if !self.retry_at.is_some_and(|t| now >= t) {
                return;
            }
            match Engine::new(self.tx.clone()) {
                Ok(mut engine) => {
                    engine.devices_loaded = self.devices_loaded;
                    engine.set_preset_rules(self.preset_rules.clone());
                    engine.pinned_outputs.clone_from(&self.pinned_outputs);
                    engine.reductions.solo_level = self.solo_level;
                    self.engine = Some(engine);
                    self.sync_automation();
                    self.retry_at = None;
                    self.dirty = Dirty { devices: true, sessions: true, values: true, rebuild: false };
                }
                Err(e) => {
                    self.fail(format!("Windows audio service unavailable ({})", e.message()));
                    return;
                }
            }
        }

        if !self.dirty.any() || self.retry_at.is_some_and(|t| now < t) {
            return;
        }
        self.retry_at = None;
        let dirty = std::mem::take(&mut self.dirty);
        if dirty.devices {
            self.device_events.cancel();
        }
        let engine = self.engine.as_mut().expect("engine present");
        let result = engine.refresh(dirty, &self.shared).and_then(|()| engine.snapshot(&self.shared));
        match result {
            Ok(snapshot) => {
                self.backoff = Duration::from_millis(250);
                if self.was_unavailable {
                    self.was_unavailable = false;
                    self.shared.push_notice(NoticeLevel::Info, "Audio service reconnected.");
                }
                self.shared.publish(snapshot);
            }
            Err(e) if is_fatal(e.code()) => {
                log::warn!("fatal audio error, rebuilding: {e}");
                self.engine = None;
                self.fail(format!("Audio devices are reinitializing ({})", e.message()));
            }
            Err(e) => {
                // Transient: keep the engine, retry the structural refresh shortly.
                log::warn!("audio refresh failed: {e}");
                self.dirty.devices = true;
                self.dirty.sessions = true;
                self.retry_at = Some(Instant::now() + self.backoff);
                self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
            }
        }
    }

    fn fail(&mut self, message: String) {
        log::warn!("{message}; retrying in {:?}", self.backoff);
        self.retry_at = Some(Instant::now() + self.backoff);
        self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
        if !self.was_unavailable {
            self.was_unavailable = true;
            self.shared.push_notice(NoticeLevel::Warning, message.clone());
        }
        self.shared.publish(AudioSnapshot { status: ServiceStatus::Unavailable(message), ..Default::default() });
    }

    fn next_timeout(&self) -> Option<Duration> {
        let now = Instant::now();
        let mut deadline: Option<Instant> = None;
        let mut consider = |t: Instant| {
            deadline = Some(deadline.map_or(t, |d| d.min(t)));
        };
        if let Some(t) = self.retry_at
            && (self.engine.is_none() || self.dirty.any())
        {
            consider(t);
        }
        if let Some(t) = self.delayed_rebuild {
            consider(t);
        }
        if let Some(t) = self.device_events.due {
            consider(t);
        }
        if self.engine.is_some() && self.shared.meters_wanted() {
            consider(self.last_meter_poll + METER_INTERVAL);
        }
        if let Some(t) = self.focus.due(FOCUS_SETTLE) {
            consider(t);
        }
        if self.engine.is_some() && self.mic.is_some() {
            consider(self.last_mic_poll + MIC_INTERVAL);
        }
        deadline.map(|d| d.saturating_duration_since(now))
    }

    fn poll_meters(&mut self) {
        if !self.shared.meters_wanted() {
            if *self.last_meters != Meters::default() {
                self.last_meters = Arc::default();
                self.shared.publish_meters(self.last_meters.clone());
            }
            return;
        }
        if self.last_meter_poll.elapsed() < METER_INTERVAL {
            return;
        }
        self.last_meter_poll = Instant::now();
        let Some(engine) = self.engine.as_ref() else { return };
        engine.read_meters(&mut self.meter_scratch);
        // Only a visible change allocates: the published copy that triggers a repaint.
        if meters_differ(&self.meter_scratch, &self.last_meters) {
            self.last_meters = Arc::new(self.meter_scratch.clone());
            self.shared.publish_meters(self.last_meters.clone());
        }
    }
}

impl Runner {
    /// Hands the current automation state to the engine and re-reads levels.
    fn sync_automation(&mut self) {
        let Some(engine) = self.engine.as_mut() else { return };
        let r = &mut engine.reductions;
        r.focus_rules.clone_from(&self.focus_rules);
        r.foreground = self.focus.applied().cloned();
        r.mic = self.mic.as_ref().map(|m| (m.reduction, m.targets.clone()));
        r.mic_active = self.mic.is_some() && self.mic_gate.active();
        self.dirty.values = true;
    }

    fn automation_tick(&mut self) {
        let now = Instant::now();
        let mut changed = self.focus.settle(FOCUS_SETTLE, now);
        if let (Some(mic), Some(engine)) = (&self.mic, &self.engine)
            && now.duration_since(self.last_mic_poll) >= MIC_INTERVAL
        {
            self.last_mic_poll = now;
            if self.mic_gate.update(&mic.gate, engine.input_peak(), now) {
                log::debug!("microphone ducking {}", if self.mic_gate.active() { "started" } else { "ended" });
                changed = true;
            }
        }
        if changed {
            self.sync_automation();
            self.maintain();
        }
    }
}

/// Drops an earlier pending command for the same control when a newer one arrives
/// (e.g. 60 slider updates per second collapse into the latest value).
fn push_coalesced(commands: &mut Vec<Command>, cmd: Command) {
    let same_target = |a: &Command, b: &Command| match (a, b) {
        (Command::SetEndpointVolume { flow: x, .. }, Command::SetEndpointVolume { flow: y, .. })
        | (Command::SetEndpointBalance { flow: x, .. }, Command::SetEndpointBalance { flow: y, .. }) => x == y,
        (Command::SetAppVolume { key: x, .. }, Command::SetAppVolume { key: y, .. })
        | (Command::SetAppBalance { key: x, .. }, Command::SetAppBalance { key: y, .. }) => x == y,
        (Command::SetDeviceVolume { id: x, .. }, Command::SetDeviceVolume { id: y, .. })
        | (Command::SetDeviceBalance { id: x, .. }, Command::SetDeviceBalance { id: y, .. }) => x == y,
        _ => false,
    };
    commands.retain(|c| !same_target(c, &cmd));
    commands.push(cmd);
}

fn meters_differ(a: &Meters, b: &Meters) -> bool {
    const EPS: f32 = 0.01;
    let differ = |x: f32, y: f32| (x - y).abs() > EPS;
    differ(a.output, b.output)
        || differ(a.input, b.input)
        || a.apps.iter().any(|(k, &x)| differ(x, b.apps.get(k).copied().unwrap_or(0.0)))
        || b.apps.iter().any(|(k, &y)| !a.apps.contains_key(k) && differ(0.0, y))
}

/// Stores each app's loudest session peak in `apps`, reading every session
/// once. Existing entries are reused, so a steady set of apps doesn't allocate;
/// apps without a session are removed.
fn merge_peaks<'a, S: 'a>(
    apps: &mut HashMap<AppKey, f32>,
    sessions: impl Iterator<Item = &'a S> + Clone,
    key_of: impl Fn(&S) -> &AppKey,
    mut peak_of: impl FnMut(&S) -> f32,
) {
    for v in apps.values_mut() {
        *v = 0.0;
    }
    for s in sessions.clone() {
        let peak = peak_of(s);
        match apps.get_mut(key_of(s)) {
            Some(v) => *v = v.max(peak),
            None => {
                apps.insert(key_of(s).clone(), peak);
            }
        }
    }
    apps.retain(|k, _| sessions.clone().any(|s| key_of(s) == k));
}

#[derive(Debug)]
enum ApplyError {
    Windows(String, windows::core::Error),
    Message(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::Windows(what, e) => {
                let msg = e.message();
                if msg.is_empty() {
                    write!(f, "{what} (error {:#010x}).", e.code().0)
                } else {
                    write!(f, "{what}: {}", msg.trim_end_matches(['.', '\r', '\n']))
                }
            }
            ApplyError::Message(m) => f.write_str(m),
        }
    }
}

trait Context<T> {
    fn context(self, what: impl FnOnce() -> String) -> Result<T, ApplyError>;
}

impl<T> Context<T> for WinResult<T> {
    fn context(self, what: impl FnOnce() -> String) -> Result<T, ApplyError> {
        self.map_err(|e| ApplyError::Windows(what(), e))
    }
}

struct Engine {
    tx: Sender<Msg>,
    enumerator: IMMDeviceEnumerator,
    device_notifier: IMMNotificationClient,
    policy: Option<PolicyConfig>,
    outputs: Vec<DeviceInfo>,
    inputs: Vec<DeviceInfo>,
    output: Option<Endpoint>,
    input: Option<Endpoint>,
    /// Sessions of every active output, the default's first. Apps set to use
    /// another device play there, and must stay visible and controllable.
    sessions: Vec<SessionSet>,
    extra_endpoints: HashMap<String, Endpoint>,
    endpoint_errors: HashMap<String, String>,
    devices_loaded: bool,
    properties: HashMap<String, super::properties::DeviceProperties>,
    preset_rules: Vec<super::preferences::VolumePreset>,
    start_tracker: super::preferences::StartTracker,
    last_reclaim: Instant,
    pinned_outputs: Vec<String>,
    /// Set by `cycle_output`, so the resulting switch is announced.
    announce_output_until: Option<Instant>,
    reductions: Reductions,
    routing: Option<AppRouting>,
    /// Each process's device preference as last read from (or written to)
    /// Windows. Read once per process: Windows doesn't notify changes.
    routes: HashMap<(u32, Option<u64>), Route>,
}

/// Solo and ducking state: what should be lowered, and the ledger of
/// what was written so it can be undone safely.
struct Reductions {
    solo: Option<AppKey>,
    solo_level: Reduction,
    focus_rules: Vec<(AppKey, Reduction)>,
    /// The settled foreground app, `None` until one is known.
    foreground: Option<AppKey>,
    mic: Option<(Reduction, Vec<AppKey>)>,
    mic_active: bool,
    ledger: Ledger,
}

impl Reductions {
    fn wanted(&self, key: &AppKey) -> Option<Reduction> {
        // System sounds are short alerts; Solo leaves them alone.
        let solo = self.solo.as_ref().filter(|solo| *solo != key && *key != AppKey::System).map(|_| self.solo_level);
        let focus = self.background_reduction(key);
        let mic = self
            .mic
            .as_ref()
            .filter(|(_, targets)| self.mic_active && targets.contains(key))
            .map(|(reduction, _)| *reduction);
        [solo, focus, mic].into_iter().flatten().reduce(Reduction::combine)
    }

    /// The focus rule lowering `key` now, if another app is in the foreground.
    fn background_reduction(&self, key: &AppKey) -> Option<Reduction> {
        let foreground = self.foreground.as_ref()?;
        if foreground == key {
            return None;
        }
        self.focus_rules.iter().find(|(rule, _)| rule == key).map(|(_, reduction)| *reduction)
    }

    fn active(&self) -> bool {
        self.solo.is_some()
            || !self.ledger.is_empty()
            || self.mic_active
            || (self.foreground.is_some() && !self.focus_rules.is_empty())
    }
}

impl Engine {
    fn new(tx: Sender<Msg>) -> WinResult<Self> {
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let device_notifier = DeviceNotifier::create(tx.clone());
        unsafe { enumerator.RegisterEndpointNotificationCallback(&device_notifier)? };
        let policy = match PolicyConfig::create() {
            Ok(p) => Some(p),
            Err(e) => {
                log::warn!("default-device switching unavailable: {e}");
                None
            }
        };
        let routing = match AppRouting::create() {
            Ok(routing) => Some(routing),
            Err(e) => {
                log::warn!("per-app device routing unavailable: {e}");
                None
            }
        };
        log::info!("audio engine initialized");
        Ok(Self {
            tx,
            enumerator,
            device_notifier,
            policy,
            outputs: Vec::new(),
            inputs: Vec::new(),
            output: None,
            input: None,
            sessions: Vec::new(),
            extra_endpoints: HashMap::new(),
            endpoint_errors: HashMap::new(),
            devices_loaded: false,
            properties: HashMap::new(),
            preset_rules: Vec::new(),
            start_tracker: super::preferences::StartTracker::new(current_filetime()),
            last_reclaim: Instant::now(),
            pinned_outputs: Vec::new(),
            announce_output_until: None,
            routing,
            routes: HashMap::new(),
            reductions: Reductions {
                solo: None,
                solo_level: Reduction { factor: 0.2, mute: false },
                focus_rules: Vec::new(),
                foreground: None,
                mic: None,
                mic_active: false,
                ledger: Ledger::default(),
            },
        })
    }

    fn refresh(&mut self, dirty: Dirty, shared: &Shared) -> WinResult<()> {
        if dirty.devices {
            self.outputs = list_devices(&self.enumerator, eRender)?;
            self.inputs = list_devices(&self.enumerator, eCapture)?;
            let out_id = default_device_id(&self.enumerator, eRender)?;
            let in_id = default_device_id(&self.enumerator, eCapture)?;

            if self.output.as_ref().map(|e| &e.id) != out_id.as_ref() {
                let had_output = self.output.is_some();
                self.output = None;
                if let Some(id) = &out_id {
                    let ep = Endpoint::open(&self.enumerator, id, self.tx.clone())?;
                    if had_output {
                        let text = format!("Output switched to {}", ep.name);
                        if self.announce_output_until.take().is_some_and(|until| Instant::now() < until) {
                            shared.push_announcement(text);
                        } else {
                            shared.push_notice(NoticeLevel::Info, text);
                        }
                    }
                    self.output = Some(ep);
                } else {
                    shared.push_notice(NoticeLevel::Warning, "No output device is available.");
                }
            } else if let Some(ep) = self.output.as_mut() {
                ep.refresh_name(&self.outputs);
            }

            if self.input.as_ref().map(|e| &e.id) != in_id.as_ref() {
                let had_input = self.input.is_some();
                self.input = None;
                if let Some(id) = &in_id {
                    let ep = Endpoint::open(&self.enumerator, id, self.tx.clone())?;
                    if had_input {
                        shared.push_notice(NoticeLevel::Info, format!("Input switched to {}", ep.name));
                    }
                    self.input = Some(ep);
                }
            } else if let Some(ep) = self.input.as_mut() {
                ep.refresh_name(&self.inputs);
            }

            for d in &mut self.outputs {
                d.is_default = out_id.as_ref() == Some(&d.id);
            }
            self.open_session_sets(out_id.as_deref())?;
            for d in &mut self.inputs {
                d.is_default = in_id.as_ref() == Some(&d.id);
            }
            // Default endpoints already have owners. Keep one callback owner per
            // additional endpoint and release it as soon as that device disappears.
            self.extra_endpoints
                .retain(|id, _| self.outputs.iter().chain(&self.inputs).any(|d| &d.id == id && !d.is_default));
            self.endpoint_errors.clear();
            if self.devices_loaded {
                for d in self.outputs.iter().chain(&self.inputs).filter(|d| !d.is_default) {
                    if let Some(ep) = self.extra_endpoints.get_mut(&d.id) {
                        ep.name.clone_from(&d.name);
                    } else {
                        match Endpoint::open(&self.enumerator, &d.id, self.tx.clone()) {
                            Ok(ep) => {
                                self.extra_endpoints.insert(d.id.clone(), ep);
                            }
                            Err(e) if is_fatal(e.code()) => return Err(e),
                            Err(e) => {
                                self.endpoint_errors.insert(d.id.clone(), format!("Device unavailable: {e}"));
                            }
                        }
                    }
                }
            }
        }
        if dirty.devices {
            self.properties.clear();
            for (flow, devices) in [(Flow::Output, &self.outputs), (Flow::Input, &self.inputs)] {
                for d in devices {
                    if let Ok(ep) = self.device(&d.id) {
                        let props = super::properties::read_device_properties(&ep.device, flow == Flow::Output);
                        self.properties.insert(d.id.clone(), props);
                    }
                }
            }
        }
        if dirty.sessions || dirty.devices {
            for set in &mut self.sessions {
                for exited_pid in set.sync(&self.tx)? {
                    self.start_tracker.forget_exited_pid(exited_pid);
                }
            }
            let wait = if self.start_tracker.wants_reclaim() { RECLAIM_RETRY } else { RECLAIM_INTERVAL };
            if self.last_reclaim.elapsed() >= wait {
                self.last_reclaim = Instant::now();
                self.start_tracker.reclaim(current_filetime(), process::is_running);
            }
        }
        Ok(())
    }

    fn snapshot(&mut self, shared: &Shared) -> WinResult<AudioSnapshot> {
        self.apply_start_rules(shared)?;
        self.apply_reductions(shared)?;
        let mut facts = Vec::new();
        let mut needs_sync = false;
        let default_id = self.output.as_ref().map(|ep| ep.id.as_str());
        let on_default: HashSet<&AppKey> = self
            .sessions
            .iter()
            .filter(|set| Some(set.device_id.as_str()) == default_id)
            .flat_map(|set| set.sessions.iter().map(|s| &s.key))
            .collect();
        for set in &self.sessions {
            let is_default = Some(set.device_id.as_str()) == default_id;
            for s in &set.sessions {
                if set.has_exited(s.pid) {
                    needs_sync = true;
                    continue;
                }
                match s.facts(&set.device_id) {
                    // Other outputs show what plays there and apps set to use them;
                    // their idle leftovers and System sounds would only add noise.
                    Ok(Some(f))
                        if !is_default && (f.key == AppKey::System || (!f.active && on_default.contains(&f.key))) => {}
                    Ok(Some(f)) => facts.push(f),
                    Ok(None) => needs_sync = true,
                    Err(e) if is_fatal(e.code()) => return Err(e),
                    Err(e) => {
                        log::debug!("skipping session {}: {e}", s.instance_id);
                        needs_sync = true;
                    }
                }
            }
        }
        if needs_sync {
            let _ = self.tx.send(Msg::SessionsChanged);
        }
        let mut device_states = Vec::new();
        if self.devices_loaded {
            for (flow, d) in
                self.outputs.iter().map(|d| (Flow::Output, d)).chain(self.inputs.iter().map(|d| (Flow::Input, d)))
            {
                let state = match self.device(&d.id) {
                    Ok(ep) => match ep.state() {
                        Ok(state) => Ok(state),
                        Err(e) if is_fatal(e.code()) => return Err(e),
                        Err(e) => Err(e.to_string()),
                    },
                    Err(e) => Err(e.to_string()),
                };
                device_states.push(DeviceState { flow, info: d.clone(), state });
            }
        }
        let mut apps = group_sessions(&facts);
        self.read_routes(&mut apps);
        let apps_keys: Vec<AppKey> = apps.iter().map(|a| a.key.clone()).collect();
        Ok(AudioSnapshot {
            status: ServiceStatus::Ok,
            output: self.output.as_ref().map(Endpoint::state).transpose()?,
            input: self.input.as_ref().map(Endpoint::state).transpose()?,
            outputs: self.outputs.clone(),
            inputs: self.inputs.clone(),
            apps,
            device_states,
            properties: self.properties.clone(),
            can_switch_default: self.policy.is_some(),
            solo: self.reductions.solo.clone(),
            can_route_apps: self.routing.is_some(),
            mic_ducking: self.reductions.mic_active,
            background_lowered: apps_lowered_in_background(&self.reductions, &apps_keys),
        })
    }

    /// The default input's peak level, for microphone ducking.
    fn input_peak(&self) -> f32 {
        self.input.as_ref().and_then(|e| e.meter.as_ref()).and_then(|m| unsafe { m.GetPeakValue() }.ok()).unwrap_or(0.0)
    }

    /// The app identity of a process, as its sessions would have it.
    fn key_for_pid(&self, pid: u32) -> AppKey {
        if let Some(s) = self.sessions.iter().flat_map(|set| set.sessions.iter()).find(|s| s.pid == pid) {
            return s.key.clone();
        }
        let info = process::open(pid).map(|handle| process::query(&handle)).unwrap_or_default();
        app_key(&info, pid)
    }

    fn read_meters(&self, out: &mut Meters) {
        let peak = |m: &Option<IAudioMeterInformation>| {
            m.as_ref().and_then(|m| unsafe { m.GetPeakValue() }.ok()).unwrap_or(0.0)
        };
        let sessions = self.sessions.iter().flat_map(|set| set.sessions.iter());
        merge_peaks(&mut out.apps, sessions, |s: &Session| &s.key, |s| peak(&s.meter));
        out.output = self.output.as_ref().map_or(0.0, |e| peak(&e.meter));
        out.input = self.input.as_ref().map_or(0.0, |e| peak(&e.meter));
    }

    /// Applies a command. Returns whether device lists must be re-read.
    fn apply(&mut self, cmd: &Command) -> Result<bool, ApplyError> {
        let ctx = &OWN_EVENT_CONTEXT;
        match cmd {
            Command::ToggleInputMute => {
                let ep = self.endpoint(Flow::Input)?;
                let what = || format!("Couldn't change mute for {}", ep.name);
                let muted = unsafe { ep.volume.GetMute() }.context(what)?.as_bool();
                unsafe { ep.volume.SetMute(!muted, ctx) }.context(what)?;
            }
            Command::AdjustOutputVolume { delta } => {
                let ep = self.endpoint(Flow::Output)?;
                let what = || format!("Couldn't change volume for {}", ep.name);
                let current = unsafe { ep.volume.GetMasterVolumeLevelScalar() }.context(what)?;
                unsafe { ep.volume.SetMasterVolumeLevelScalar((current + delta).clamp(0.0, 1.0), ctx) }
                    .context(what)?;
            }
            Command::ApplyPreset(_) | Command::ActivateProfile(_) | Command::SetPresetRules(_) => {}
            Command::LoadDevices => {
                self.devices_loaded = true;
                return Ok(true);
            }
            Command::SetDeviceVolume { id, volume } => {
                let ep = self.device(id)?;
                unsafe { ep.volume.SetMasterVolumeLevelScalar(volume.clamp(0.0, 1.0), ctx) }
                    .context(|| format!("Couldn't change the volume of {}", ep.name))?;
            }
            Command::SetDeviceMute { id, muted } => {
                let ep = self.device(id)?;
                unsafe { ep.volume.SetMute(*muted, ctx) }
                    .context(|| format!("Couldn't change mute for {}", ep.name))?;
            }
            Command::SetDeviceBalance { id, balance } => {
                self.device(id)?.set_balance(*balance)?;
            }
            Command::SetEndpointVolume { flow, volume } => {
                let ep = self.endpoint(*flow)?;
                unsafe { ep.volume.SetMasterVolumeLevelScalar(volume.clamp(0.0, 1.0), ctx) }
                    .context(|| format!("Couldn't change the volume of {}", ep.name))?;
            }
            Command::SetEndpointMute { flow, muted } => {
                let ep = self.endpoint(*flow)?;
                unsafe { ep.volume.SetMute(*muted, ctx) }
                    .context(|| format!("Couldn't change mute for {}", ep.name))?;
            }
            Command::ToggleOutputMute => {
                let ep = self.endpoint(Flow::Output)?;
                let what = || format!("Couldn't change mute for {}", ep.name);
                let muted = unsafe { ep.volume.GetMute() }.context(what)?.as_bool();
                unsafe { ep.volume.SetMute(!muted, ctx) }.context(what)?;
            }
            Command::SetEndpointBalance { flow, balance } => {
                self.endpoint(*flow)?.set_balance(*balance)?;
            }
            Command::SetDefaultDevice { flow, id } => {
                let policy = self.policy.as_ref().ok_or_else(|| {
                    ApplyError::Message("Switching the default device isn't supported on this system.".into())
                })?;
                let list = match flow {
                    Flow::Output => &self.outputs,
                    Flow::Input => &self.inputs,
                };
                let name = list
                    .iter()
                    .find(|d| &d.id == id)
                    .map(|d| d.name.clone())
                    .ok_or_else(|| ApplyError::Message("That device is no longer available.".into()))?;
                policy.set_default(id).context(|| format!("Couldn't switch to {name}"))?;
                return Ok(true);
            }
            Command::SetAppVolume { key, volume } => {
                self.for_app(key, "volume", |s| unsafe { s.simple.SetMasterVolume(volume.clamp(0.0, 1.0), ctx) })?;
                self.mark_key_intervened(key);
            }
            Command::SetAppMute { key, muted } => {
                self.for_app(key, "mute", |s| unsafe { s.simple.SetMute(*muted, ctx) })?;
                self.mark_key_intervened(key);
            }
            Command::SetAppBalance { key, balance } => {
                let (l, r) = channels_from_balance(*balance);
                self.for_app(key, "balance", |s| match &s.channels {
                    Some(ch) if unsafe { ch.GetChannelCount() }? == 2 => unsafe {
                        ch.SetChannelVolume(0, l, ctx)?;
                        ch.SetChannelVolume(1, r, ctx)
                    },
                    _ => Ok(()),
                })?;
                self.mark_key_intervened(key);
            }
            Command::SetStreamVolume { id, volume } => {
                let session = self.stream(id)?;
                unsafe { session.simple.SetMasterVolume(volume.clamp(0.0, 1.0), ctx) }
                    .context(|| format!("Couldn't change the volume of a {} stream", session.name))?;
                let key = session.key.clone();
                self.mark_key_intervened(&key);
            }
            Command::SetStreamMute { id, muted } => {
                let session = self.stream(id)?;
                unsafe { session.simple.SetMute(*muted, ctx) }
                    .context(|| format!("Couldn't change mute for a {} stream", session.name))?;
                let key = session.key.clone();
                self.mark_key_intervened(&key);
            }
            Command::SetAppOutput { key, device } => {
                self.route_app(key, device.as_deref())?;
                return Ok(true);
            }
            Command::ResetAllAppDevices => {
                let routing = self.routing.as_ref().ok_or_else(|| {
                    ApplyError::Message("Choosing a device per app isn't supported on this version of Windows.".into())
                })?;
                routing.clear_all().context(|| "Couldn't reset the app device preferences".into())?;
                self.routes.clear();
                return Ok(true);
            }
            Command::ToggleAppMute { key } => {
                let all_muted = self
                    .sessions
                    .iter()
                    .flat_map(|set| set.sessions.iter())
                    .filter(|s| &s.key == key)
                    .all(|s| unsafe { s.simple.GetMute() }.is_ok_and(|m| m.as_bool()));
                self.for_app(key, "mute", |s| unsafe { s.simple.SetMute(!all_muted, ctx) })?;
                self.mark_key_intervened(key);
            }
            Command::AdjustAppVolume { key, delta } => {
                let current = self
                    .sessions
                    .iter()
                    .flat_map(|set| set.sessions.iter())
                    .filter(|s| &s.key == key)
                    .filter_map(|s| unsafe { s.simple.GetMasterVolume() }.ok())
                    .fold(0.0f32, f32::max);
                let volume = (current + delta).clamp(0.0, 1.0);
                self.for_app(key, "volume", |s| unsafe { s.simple.SetMasterVolume(volume, ctx) })?;
                self.mark_key_intervened(key);
            }
            Command::SetFocusRules(_) | Command::ForegroundChanged { .. } | Command::SetMicDucking(_) => {}
            Command::SetPinnedOutputs(_)
            | Command::CycleOutputDevice
            | Command::ToggleSolo { .. }
            | Command::EndSolo
            | Command::SetSoloOptions { .. } => {}
            Command::Refresh | Command::OpenSoundSettings | Command::OpenMonoSettings => {}
        }
        Ok(false)
    }

    fn set_preset_rules(&mut self, presets: Vec<super::preferences::VolumePreset>) {
        self.preset_rules = presets.into_iter().filter(|p| p.apply_on_start).collect();
        self.start_tracker.rearm(current_filetime());
    }

    /// Applies a whole profile: default devices first (app levels are kept per
    /// device, so the sessions are re-read on the new one), then the master
    /// level, then the app levels.
    fn apply_profile(
        &mut self,
        profile: &super::preferences::VolumePreset,
        shared: &Shared,
    ) -> Result<ProfileOutcome, ApplyError> {
        let mut outcome = ProfileOutcome::default();
        for (flow, choice) in [(Flow::Output, &profile.output_device), (Flow::Input, &profile.input_device)] {
            let Some(choice) = choice else { continue };
            let list = if flow == Flow::Output { &self.outputs } else { &self.inputs };
            match list.iter().find(|d| d.id.eq_ignore_ascii_case(&choice.id)) {
                None => outcome.notes.push(format!("{} isn't connected", choice.name)),
                Some(device) if device.is_default => {}
                Some(device) => {
                    let Some(policy) = self.policy.as_ref() else {
                        outcome.notes.push("switching the default device isn't supported here".into());
                        continue;
                    };
                    let name = device.name.clone();
                    policy.set_default(&device.id).context(|| format!("Couldn't switch to {name}"))?;
                    outcome.devices_switched = true;
                }
            }
        }
        if outcome.devices_switched {
            let dirty = Dirty { devices: true, sessions: true, ..Dirty::default() };
            self.refresh(dirty, shared)
                .context(|| "Profile partially applied; the new device couldn't be read".into())?;
        }
        if let Some(master) = profile.master {
            match &self.output {
                Some(ep) => unsafe {
                    let what = || format!("Couldn't change the volume of {}", ep.name);
                    ep.volume
                        .SetMasterVolumeLevelScalar(master.volume.clamp(0.0, 1.0), &OWN_EVENT_CONTEXT)
                        .context(what)?;
                    ep.volume.SetMute(master.muted, &OWN_EVENT_CONTEXT).context(what)?;
                    outcome.master = true;
                },
                None => outcome.notes.push("no output device for the master level".into()),
            }
        }
        if !profile.entries.is_empty() {
            outcome.apps = Some(self.apply_preset(profile.clone())?);
        }
        for entry in profile.entries.iter().filter(|e| e.output.is_some()) {
            let Some(device) = &entry.output else { continue };
            if self.processes_of(&entry.key).is_empty() {
                continue;
            }
            if !self.outputs.iter().any(|d| d.id.eq_ignore_ascii_case(&device.id)) {
                outcome.notes.push(format!("{} isn't connected for {}", device.name, entry.name));
            } else if let Err(e) = self.route_app(&entry.key, Some(&device.id)) {
                outcome.notes.push(format!("{}'s device ({e})", entry.name));
            }
        }
        Ok(outcome)
    }

    fn apply_preset(&mut self, preset: super::preferences::VolumePreset) -> Result<PresetOutcome, ApplyError> {
        let mut intervened = Vec::new();
        let outcome = {
            let sessions: Vec<&Session> = self.sessions.iter().flat_map(|set| set.sessions.iter()).collect();
            execute_preset(
                preset,
                &sessions,
                |s| &s.key,
                |s, volume, muted| unsafe {
                    s.simple.SetMasterVolume(volume, &OWN_EVENT_CONTEXT)?;
                    s.simple.SetMute(muted, &OWN_EVENT_CONTEXT)
                },
                |key| intervened.push(key.clone()),
            )?
        };
        for key in &intervened {
            self.mark_key_intervened(key);
        }
        Ok(outcome)
    }

    fn mark_key_intervened(&mut self, key: &AppKey) {
        for set in &self.sessions {
            for session in set.sessions.iter().filter(|s| &s.key == key) {
                if let Some(created) = set.processes.get(&session.pid).and_then(|p| p.started_at) {
                    self.start_tracker.note_process_intervention(key, session.pid, created);
                }
            }
        }
    }

    fn apply_start_rules(&mut self, shared: &Shared) -> WinResult<()> {
        if self.preset_rules.is_empty() {
            return Ok(());
        }
        let candidates: Vec<StartCandidate<&Session>> = self
            .sessions
            .iter()
            .flat_map(|set| set.sessions.iter().map(move |s| (set, s)))
            .map(|(set, s)| {
                let created = set.processes.get(&s.pid).and_then(|p| p.started_at);
                let untouched = !s.changed.load(Ordering::Relaxed)
                    && unsafe {
                        s.simple.GetMasterVolume().ok().is_some_and(|v| (v - 1.0).abs() < 0.0001)
                            && s.simple.GetMute().ok().is_some_and(|m| !m.as_bool())
                    };
                StartCandidate { id: s, key: s.key.clone(), pid: s.pid, created, untouched }
            })
            .collect();
        execute_start_rules(
            &self.preset_rules,
            &mut self.start_tracker,
            candidates,
            |s, key, volume, muted| {
                let res = unsafe {
                    s.simple
                        .SetMasterVolume(volume, &OWN_EVENT_CONTEXT)
                        .and_then(|()| s.simple.SetMute(muted, &OWN_EVENT_CONTEXT))
                };
                if let Err(err) = &res {
                    log::warn!("automatic preset failed for {key:?}, session {}: {err}", s.instance_id);
                }
                res
            },
            |msg| shared.push_notice(NoticeLevel::Error, msg),
        )
    }
    /// Switches to the next connected pinned output. Returns an announcement
    /// when nothing was switched; a switch is announced once Windows reports it.
    fn cycle_output(&mut self) -> Result<Option<String>, ApplyError> {
        use super::preferences::{CycleOutcome, next_pinned_output};
        let next = match next_pinned_output(&self.pinned_outputs, &self.outputs) {
            CycleOutcome::Switch(device) => device.clone(),
            CycleOutcome::AlreadyDefault(device) => {
                return Ok(Some(format!("{} is the only pinned output connected.", device.name)));
            }
            CycleOutcome::NothingPinned => {
                return Ok(Some("Pin output devices in Devices to cycle through them.".into()));
            }
            CycleOutcome::NoneConnected => return Ok(Some("None of the pinned output devices is connected.".into())),
        };
        let policy = self.policy.as_ref().ok_or_else(|| {
            ApplyError::Message("Switching the default device isn't supported on this system.".into())
        })?;
        policy.set_default(&next.id).context(|| format!("Couldn't switch to {}", next.name))?;
        self.announce_output_until = Some(Instant::now() + ANNOUNCE_WINDOW);
        Ok(None)
    }

    /// Sets (or resets) the output of every process of the app.
    fn route_app(&mut self, key: &AppKey, device: Option<&str>) -> Result<(), ApplyError> {
        let routing = self.routing.as_ref().ok_or_else(|| {
            ApplyError::Message("Choosing a device per app isn't supported on this version of Windows.".into())
        })?;
        let processes = self.processes_of(key);
        if processes.is_empty() {
            return Err(ApplyError::Message(
                "That app has no audio session right now; start it playing, then choose its device.".into(),
            ));
        }
        let name = self.app_name(key).unwrap_or_else(|| "the app".into());
        let mut set = 0;
        let mut first_error = None;
        for (pid, created) in processes {
            match routing.set(pid, Flow::Output, device) {
                Ok(()) => {
                    set += 1;
                    let route = device.map_or(Route::SystemDefault, |id| Route::Device(id.to_owned()));
                    self.routes.insert((pid, created), route);
                }
                // A helper process may exit between listing and routing it.
                Err(e) if created.is_none_or(|t| process::is_running(pid, t)) => {
                    first_error.get_or_insert(e);
                }
                Err(_) => {}
            }
        }
        match first_error {
            Some(e) if set == 0 => Err(ApplyError::Windows(format!("Couldn't change the device of {name}"), e)),
            _ => Ok(()),
        }
    }

    /// Every process with a session of the app, with its creation time when known.
    fn processes_of(&self, key: &AppKey) -> Vec<(u32, Option<u64>)> {
        let mut found: Vec<(u32, Option<u64>)> = Vec::new();
        for set in &self.sessions {
            for s in set.sessions.iter().filter(|s| &s.key == key && s.pid != 0) {
                let created = set.processes.get(&s.pid).and_then(|p| p.started_at);
                if !found.iter().any(|(pid, _)| *pid == s.pid) {
                    found.push((s.pid, created));
                }
            }
        }
        found
    }

    /// Fills each app's device preference, querying Windows only for processes
    /// not seen before. An app's first process that answers decides.
    fn read_routes(&mut self, apps: &mut [AppEntry]) {
        let Some(routing) = &self.routing else {
            for app in apps.iter_mut() {
                app.route = Route::Unknown;
            }
            return;
        };
        let live: Vec<(u32, Option<u64>)> =
            apps.iter().filter(|a| a.key != AppKey::System).flat_map(|a| self.processes_of(&a.key)).collect();
        self.routes.retain(|process, _| live.contains(process));
        for app in apps.iter_mut() {
            if app.key == AppKey::System {
                continue;
            }
            let mut route = Route::Unknown;
            for process in self.processes_of(&app.key) {
                let known = self.routes.entry(process).or_insert_with(|| match routing.get(process.0, Flow::Output) {
                    Ok(Some(id)) => Route::Device(id),
                    Ok(None) => Route::SystemDefault,
                    Err(e) => {
                        log::debug!("couldn't read the device preference of process {}: {e}", process.0);
                        Route::Unknown
                    }
                });
                if *known != Route::Unknown {
                    route = known.clone();
                    break;
                }
            }
            app.route = route;
        }
    }

    fn stream(&self, id: &str) -> Result<&Session, ApplyError> {
        self.sessions
            .iter()
            .flat_map(|set| set.sessions.iter())
            .find(|s| s.instance_id == id)
            .ok_or_else(|| ApplyError::Message("That audio stream has ended; the list has been refreshed.".into()))
    }

    fn app_name(&self, key: &AppKey) -> Option<String> {
        self.sessions.iter().flat_map(|set| set.sessions.iter()).find(|s| &s.key == key).map(|s| s.name.clone())
    }

    fn toggle_solo(&mut self, key: &AppKey) -> Result<Option<String>, ApplyError> {
        if self.reductions.solo.as_ref() == Some(key) {
            return self.end_solo();
        }
        let name = self.app_name(key).ok_or_else(|| {
            ApplyError::Message("That app has no audio session right now, so it can't be soloed.".into())
        })?;
        log::info!("solo started for {key:?}");
        self.reductions.solo = Some(key.clone());
        Ok(Some(format!("Solo: {name}")))
    }

    fn end_solo(&mut self) -> Result<Option<String>, ApplyError> {
        let Some(key) = self.reductions.solo.take() else {
            return Ok(None);
        };
        log::info!("solo ended for {key:?}");
        let name = self.app_name(&key).unwrap_or_else(|| "the app".into());
        Ok(Some(format!("Solo ended for {name}")))
    }

    /// Writes the reductions that should apply now and undoes those that ended.
    fn apply_reductions(&mut self, shared: &Shared) -> WinResult<()> {
        if let Some(key) = &self.reductions.solo
            && !self.sessions.iter().flat_map(|set| set.sessions.iter()).any(|s| &s.key == key)
        {
            log::info!("solo ended: {key:?} has no audio session left");
            self.reductions.solo = None;
            shared.push_announcement("Solo ended: the app closed its audio.");
        }
        if !self.reductions.active() {
            return Ok(());
        }
        for s in self.sessions.iter().flat_map(|set| set.sessions.iter()) {
            let wanted = self.reductions.wanted(&s.key);
            if wanted.is_none() && !self.reductions.ledger.contains(&s.instance_id) {
                continue;
            }
            let current = match unsafe { s.simple.GetMasterVolume().and_then(|v| Ok((v, s.simple.GetMute()?))) } {
                Ok((volume, muted)) => Level { volume, muted: muted.as_bool() },
                Err(e) if is_fatal(e.code()) => return Err(e),
                Err(e) => {
                    log::debug!("couldn't read {} for a reduction: {e}", s.name);
                    continue;
                }
            };
            if let Some(target) = self.reductions.ledger.reconcile(&s.instance_id, current, wanted)
                && let Err(e) = write_level(s, target)
            {
                if is_fatal(e.code()) {
                    return Err(e);
                }
                log::warn!("couldn't change {} for a reduction: {e}", s.name);
            }
        }
        let sessions = &self.sessions;
        self.reductions
            .ledger
            .retain(|id| sessions.iter().flat_map(|set| set.sessions.iter()).any(|s| s.instance_id == id));
        Ok(())
    }

    /// Puts back every level a reduction changed (when it is still the written
    /// one), before the sessions are released.
    fn release_reductions(&mut self) {
        let sets: Vec<&SessionSet> = self.sessions.iter().collect();
        release_in(&mut self.reductions.ledger, &sets);
        self.reductions.ledger = Ledger::default();
    }

    /// Opens a session set for every active output (the default first) and
    /// releases those of outputs that are gone.
    fn open_session_sets(&mut self, default_id: Option<&str>) -> WinResult<()> {
        let outputs = &self.outputs;
        let (keep, gone): (Vec<SessionSet>, Vec<SessionSet>) = std::mem::take(&mut self.sessions)
            .into_iter()
            .partition(|set| outputs.iter().any(|d| d.id == set.device_id));
        release_in(&mut self.reductions.ledger, &gone.iter().collect::<Vec<_>>());
        drop(gone);
        self.sessions = keep;
        for d in &self.outputs {
            if self.sessions.iter().any(|set| set.device_id == d.id) {
                continue;
            }
            let opened = unsafe { self.enumerator.GetDevice(&HSTRING::from(d.id.as_str())) }
                .and_then(|device| SessionSet::open(&device, d.id.clone(), self.tx.clone()));
            match opened {
                Ok(set) => self.sessions.push(set),
                Err(e) if is_fatal(e.code()) => return Err(e),
                Err(e) => log::warn!("couldn't open the sessions of {}: {e}", d.name),
            }
        }
        self.sessions.sort_by_key(|set| Some(set.device_id.as_str()) != default_id);
        Ok(())
    }

    fn endpoint(&self, flow: Flow) -> Result<&Endpoint, ApplyError> {
        let ep = match flow {
            Flow::Output => self.output.as_ref(),
            Flow::Input => self.input.as_ref(),
        };
        ep.ok_or_else(|| {
            ApplyError::Message(match flow {
                Flow::Output => "No output device is available.".into(),
                Flow::Input => "No input device is available.".into(),
            })
        })
    }

    fn device(&self, id: &str) -> Result<&Endpoint, ApplyError> {
        self.output
            .iter()
            .chain(self.input.iter())
            .chain(self.extra_endpoints.values())
            .find(|ep| ep.id == id)
            .ok_or_else(|| {
                ApplyError::Message(
                    self.endpoint_errors
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| "That device is no longer available. Refresh the Devices view.".into()),
                )
            })
    }

    fn for_app(&self, key: &AppKey, what: &str, f: impl Fn(&Session) -> WinResult<()>) -> Result<(), ApplyError> {
        let sessions: Vec<&Session> =
            self.sessions.iter().flat_map(|set| set.sessions.iter()).filter(|s| &s.key == key).collect();
        let Some(first) = sessions.first() else {
            return Err(ApplyError::Message(
                "That app no longer has an audio session; the list has been refreshed.".into(),
            ));
        };
        let name = first.name.clone();
        for s in &sessions {
            f(s).context(|| format!("Couldn't change {what} for {name}"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PresetOutcome {
    applied: usize,
    missing: usize,
    failed: Vec<String>,
}

impl PresetOutcome {
    fn notice(&self) -> (NoticeLevel, String) {
        if !self.failed.is_empty() {
            (
                NoticeLevel::Error,
                format!("Profile applied to {} app(s); couldn't change {}.", self.applied, self.failed.join(", ")),
            )
        } else if self.applied == 0 {
            (NoticeLevel::Warning, "None of the apps in that profile are running right now.".into())
        } else if self.missing > 0 {
            (NoticeLevel::Info, format!("Profile applied to {} app(s) ({} not running).", self.applied, self.missing))
        } else {
            (NoticeLevel::Info, format!("Profile applied to {} app(s).", self.applied))
        }
    }
}

#[derive(Debug, Default, PartialEq)]
struct ProfileOutcome {
    devices_switched: bool,
    master: bool,
    apps: Option<PresetOutcome>,
    /// Parts that could not be applied, in plain words.
    notes: Vec<String>,
}

impl ProfileOutcome {
    fn notice(&self, name: &str) -> (NoticeLevel, String) {
        let (mut level, mut text) = match &self.apps {
            Some(apps) => apps.notice(),
            None if self.devices_switched || self.master => (NoticeLevel::Info, format!("Profile {name} applied.")),
            None => (NoticeLevel::Warning, format!("Nothing in profile {name} could be applied.")),
        };
        if self.apps.is_some() {
            text = text.replacen("Profile", &format!("Profile {name}"), 1);
        }
        if !self.notes.is_empty() {
            text = format!("{text} Not applied: {}.", self.notes.join("; "));
            if level == NoticeLevel::Info {
                level = NoticeLevel::Warning;
            }
        }
        (level, text)
    }
}

fn execute_preset<S>(
    preset: super::preferences::VolumePreset,
    sessions: &[S],
    key_of: impl Fn(&S) -> &AppKey,
    mut apply_one: impl FnMut(&S, f32, bool) -> WinResult<()>,
    mut on_intervened: impl FnMut(&AppKey),
) -> Result<PresetOutcome, ApplyError> {
    let Some(preset) = preset.sanitize() else {
        return Err(ApplyError::Message("Preset has no valid name.".into()));
    };
    let plan = super::preferences::plan_preset(&preset, |key| sessions.iter().any(|s| key_of(s) == key));
    let mut outcome = PresetOutcome { applied: 0, missing: plan.missing, failed: Vec::new() };
    for entry in plan.present {
        let mut entry_failed = false;
        for session in sessions.iter().filter(|s| key_of(s) == &entry.key) {
            if let Err(err) = apply_one(session, entry.volume, entry.muted) {
                if is_fatal(err.code()) {
                    return Err(ApplyError::Windows(
                        format!("Preset partially applied; couldn't change {}", entry.name),
                        err,
                    ));
                }
                log::warn!("preset failed for {:?} ({}): {err}", entry.key, entry.name);
                entry_failed = true;
            }
        }
        if entry_failed {
            outcome.failed.push(entry.name.clone());
        } else {
            on_intervened(&entry.key);
            outcome.applied += 1;
        }
    }
    Ok(outcome)
}

struct StartCandidate<I> {
    id: I,
    key: AppKey,
    pid: u32,
    created: Option<u64>,
    untouched: bool,
}

fn execute_start_rules<I>(
    preset_rules: &[super::preferences::VolumePreset],
    start_tracker: &mut super::preferences::StartTracker,
    candidates: Vec<StartCandidate<I>>,
    mut apply_one: impl FnMut(&I, &AppKey, f32, bool) -> WinResult<()>,
    mut on_error_notice: impl FnMut(String),
) -> WinResult<()> {
    if preset_rules.is_empty() {
        return Ok(());
    }
    let mut groups: HashMap<(AppKey, u32, u64), (bool, Vec<I>)> = HashMap::new();
    for candidate in candidates {
        let Some(created) = candidate.created else {
            continue;
        };
        if preset_rules.iter().any(|p| p.entries.iter().any(|e| e.key == candidate.key)) {
            let group = groups.entry((candidate.key, candidate.pid, created)).or_insert_with(|| (true, Vec::new()));
            group.0 &= candidate.untouched;
            group.1.push(candidate.id);
        }
    }
    // Sort deterministically by creation time and PID so same-refresh processes
    // are evaluated in launch order.
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by_key(|((_, pid, created), _)| (*created, *pid));
    for ((key, pid, created), (untouched, sessions)) in groups {
        if !start_tracker.observe_process(&key, pid, created, untouched) {
            continue;
        }
        let Some(entry) = preset_rules.iter().flat_map(|p| &p.entries).find(|e| e.key == key) else {
            continue;
        };
        let mut first_error = None;
        for s in &sessions {
            if let Err(err) = apply_one(s, &key, entry.volume, entry.muted)
                && (first_error.is_none() || is_fatal(err.code()))
            {
                first_error = Some(err);
            }
        }
        if let Some(err) = first_error {
            on_error_notice(format!(
                "Automatic preset for {} could not be fully applied. Check the app levels, then use Apply now to retry.",
                entry.name
            ));
            if is_fatal(err.code()) {
                return Err(err);
            }
        } else {
            log::info!("automatic preset applied once to a new process for {key:?}");
        }
    }
    Ok(())
}

/// Restores the levels reductions changed in `sets` and forgets those sessions.
fn release_in(ledger: &mut Ledger, sets: &[&SessionSet]) {
    let mut failed = 0;
    for s in sets.iter().flat_map(|set| set.sessions.iter()) {
        if !ledger.contains(&s.instance_id) {
            continue;
        }
        let current = unsafe { s.simple.GetMasterVolume().and_then(|v| Ok((v, s.simple.GetMute()?))) };
        let Ok((volume, muted)) = current else {
            failed += 1;
            continue;
        };
        let current = Level { volume, muted: muted.as_bool() };
        if let Some(base) = ledger.reconcile(&s.instance_id, current, None)
            && write_level(s, base).is_err()
        {
            failed += 1;
        }
    }
    if failed > 0 {
        log::warn!("{failed} lowered session(s) could not be restored");
    }
}

fn write_level(session: &Session, level: Level) -> WinResult<()> {
    unsafe {
        session.simple.SetMasterVolume(level.volume, &OWN_EVENT_CONTEXT)?;
        session.simple.SetMute(level.muted, &OWN_EVENT_CONTEXT)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.release_reductions();
        self.sessions.clear();
        self.output = None;
        self.input = None;
        self.extra_endpoints.clear();
        unsafe {
            let _ = self.enumerator.UnregisterEndpointNotificationCallback(&self.device_notifier);
        }
    }
}

fn list_devices(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> WinResult<Vec<DeviceInfo>> {
    let collection = unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)? };
    let count = unsafe { collection.GetCount()? };
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let device = unsafe { collection.Item(i)? };
        let id = take_pwstr(unsafe { device.GetId()? });
        let name = device_name(&device).unwrap_or_else(|| "Unknown device".into());
        out.push(DeviceInfo { id, name, is_default: false });
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    Ok(out)
}

fn default_device_id(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> WinResult<Option<String>> {
    match unsafe { enumerator.GetDefaultAudioEndpoint(flow, eConsole) } {
        Ok(device) => Ok(Some(take_pwstr(unsafe { device.GetId()? }))),
        Err(e) if is_not_found(e.code()) => Ok(None),
        Err(e) => Err(e),
    }
}

fn device_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let s = take_pwstr(PropVariantToStringAlloc(&value).ok()?);
        (!s.is_empty()).then_some(s)
    }
}

struct Endpoint {
    id: String,
    name: String,
    device: IMMDevice,
    volume: IAudioEndpointVolume,
    callback: windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolumeCallback,
    meter: Option<IAudioMeterInformation>,
}

impl Endpoint {
    fn set_balance(&self, balance: f32) -> Result<(), ApplyError> {
        let what = || format!("Couldn't change the balance of {}", self.name);
        unsafe {
            if self.volume.GetChannelCount().context(what)? != 2 {
                return Err(ApplyError::Message(format!("{} doesn't support stereo balance.", self.name)));
            }
            let master = self.volume.GetMasterVolumeLevelScalar().context(what)?;
            let (l, r) = channels_from_balance(balance);
            self.volume.SetChannelVolumeLevelScalar(0, master * l, &OWN_EVENT_CONTEXT).context(what)?;
            self.volume.SetChannelVolumeLevelScalar(1, master * r, &OWN_EVENT_CONTEXT).context(what)?;
        }
        Ok(())
    }
    fn open(enumerator: &IMMDeviceEnumerator, id: &str, tx: Sender<Msg>) -> WinResult<Self> {
        let device = unsafe { enumerator.GetDevice(&HSTRING::from(id))? };
        let volume: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let callback = EndpointVolumeNotifier::create(tx);
        unsafe { volume.RegisterControlChangeNotify(&callback)? };
        let meter = unsafe { device.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None) }.ok();
        let name = device_name(&device).unwrap_or_else(|| "Unknown device".into());
        Ok(Self { id: id.to_string(), name, device, volume, callback, meter })
    }

    fn refresh_name(&mut self, devices: &[DeviceInfo]) {
        if let Some(d) = devices.iter().find(|d| d.id == self.id) {
            self.name.clone_from(&d.name);
        }
    }

    fn state(&self) -> WinResult<EndpointState> {
        unsafe {
            let volume = self.volume.GetMasterVolumeLevelScalar()?;
            let muted = self.volume.GetMute()?.as_bool();
            let balance = if self.volume.GetChannelCount()? == 2 {
                let l = self.volume.GetChannelVolumeLevelScalar(0)?;
                let r = self.volume.GetChannelVolumeLevelScalar(1)?;
                Some(balance_from_channels(l, r))
            } else {
                None
            };
            Ok(EndpointState { id: self.id.clone(), name: self.name.clone(), volume, muted, balance })
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        unsafe {
            let _ = self.volume.UnregisterControlChangeNotify(&self.callback);
        }
    }
}

struct SessionSet {
    device_id: String,
    manager: IAudioSessionManager2,
    notifier: IAudioSessionNotification,
    sessions: Vec<Session>,
    processes: HashMap<u32, ProcessEntry>,
}

struct ProcessEntry {
    info: ProcessInfo,
    watch: Option<ProcessWatch>,
    started_at: Option<u64>,
}

impl SessionSet {
    fn open(device: &IMMDevice, device_id: String, tx: Sender<Msg>) -> WinResult<Self> {
        let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let notifier = SessionCreatedNotifier::create(tx.clone());
        unsafe { manager.RegisterSessionNotification(&notifier)? };
        let mut set = Self { device_id, manager, notifier, sessions: Vec::new(), processes: HashMap::new() };
        // Enumerating after registering is required for creation notifications to be delivered.
        let _ = set.sync(&tx)?;
        Ok(set)
    }

    fn has_exited(&self, pid: u32) -> bool {
        self.processes.get(&pid).and_then(|p| p.watch.as_ref()).is_some_and(ProcessWatch::has_exited)
    }

    fn sync(&mut self, tx: &Sender<Msg>) -> WinResult<Vec<u32>> {
        let mut exited_pids = Vec::new();
        for (&pid, entry) in &self.processes {
            if entry.watch.as_ref().is_some_and(ProcessWatch::has_exited) {
                exited_pids.push(pid);
            }
        }
        let enumerator = unsafe { self.manager.GetSessionEnumerator()? };
        let count = unsafe { enumerator.GetCount()? };
        let mut seen = HashSet::new();
        for i in 0..count {
            // One unreadable session must not hide every other one.
            let read = (|| -> WinResult<Option<(IAudioSessionControl2, String)>> {
                let control: IAudioSessionControl2 = unsafe { enumerator.GetSession(i)? }.cast()?;
                if unsafe { control.GetState()? } == AudioSessionStateExpired {
                    return Ok(None);
                }
                let instance_id = take_pwstr(unsafe { control.GetSessionInstanceIdentifier()? });
                Ok(Some((control, instance_id)))
            })();
            let (control, instance_id) = match read {
                Ok(Some(session)) => session,
                Ok(None) => continue,
                Err(e) if is_fatal(e.code()) => return Err(e),
                Err(e) => {
                    log::warn!("skipping unreadable audio session {i}: {e}");
                    continue;
                }
            };
            if self.sessions.iter().any(|s| s.instance_id == instance_id) {
                seen.insert(instance_id);
                continue;
            }
            match Session::open(control, instance_id.clone(), tx, &mut self.processes) {
                Ok(session) => {
                    if self.has_exited(session.pid) {
                        continue;
                    }
                    log::debug!("session added: {} ({:?})", session.name, session.key);
                    seen.insert(instance_id);
                    self.sessions.push(session);
                }
                Err(e) if is_fatal(e.code()) => return Err(e),
                Err(e) => log::warn!("couldn't open audio session: {e}"),
            }
        }
        for (&pid, entry) in &self.processes {
            if entry.watch.as_ref().is_some_and(ProcessWatch::has_exited) && !exited_pids.contains(&pid) {
                exited_pids.push(pid);
            }
        }
        let processes = &self.processes;
        self.sessions.retain(|s| {
            let exited = processes.get(&s.pid).and_then(|p| p.watch.as_ref()).is_some_and(ProcessWatch::has_exited);
            let keep = seen.contains(&s.instance_id) && !exited;
            if !keep {
                log::debug!("session removed: {}", s.name);
            }
            keep
        });
        let live: HashSet<u32> = self.sessions.iter().map(|s| s.pid).collect();
        self.processes.retain(|pid, _| live.contains(pid));
        Ok(exited_pids)
    }
}

impl Drop for SessionSet {
    fn drop(&mut self) {
        self.sessions.clear();
        unsafe {
            let _ = self.manager.UnregisterSessionNotification(&self.notifier);
        }
    }
}

struct Session {
    instance_id: String,
    control: IAudioSessionControl2,
    simple: ISimpleAudioVolume,
    channels: Option<IChannelAudioVolume>,
    meter: Option<IAudioMeterInformation>,
    events: IAudioSessionEvents,
    changed: Arc<AtomicBool>,
    pid: u32,
    key: AppKey,
    name: String,
    exe_path: Option<PathBuf>,
    aumid: Option<String>,
    icon_resource: Option<String>,
}

impl Session {
    fn open(
        control: IAudioSessionControl2,
        instance_id: String,
        tx: &Sender<Msg>,
        processes: &mut HashMap<u32, ProcessEntry>,
    ) -> WinResult<Self> {
        let simple: ISimpleAudioVolume = control.cast()?;
        let channels: Option<IChannelAudioVolume> = control.cast().ok();
        let meter: Option<IAudioMeterInformation> = control.cast().ok();
        let is_system = unsafe { control.IsSystemSoundsSession() } == S_OK;
        let pid = unsafe { control.GetProcessId() }.unwrap_or(0);
        let display = resolve_indirect(&take_pwstr(unsafe { control.GetDisplayName()? }));
        let icon_resource = Some(take_pwstr(unsafe { control.GetIconPath()? })).filter(|s| !s.trim().is_empty());

        let (key, name, exe_path, aumid) = if is_system {
            (AppKey::System, "System sounds".to_string(), None, None)
        } else {
            if processes.get(&pid).and_then(|p| p.watch.as_ref()).is_some_and(ProcessWatch::has_exited) {
                processes.remove(&pid);
            }
            let entry = processes.entry(pid).or_insert_with(|| match process::open(pid) {
                Some(handle) => {
                    let info = process::query(&handle);
                    let started_at = process::created_at(&handle);
                    let watch = ProcessWatch::new(handle, tx.clone());
                    ProcessEntry { info, watch, started_at }
                }
                None => ProcessEntry { info: ProcessInfo::default(), watch: None, started_at: None },
            });
            let info = &entry.info;
            let key = app_key(info, pid);
            let name = [
                Some(display).filter(|d| !d.is_empty() && !d.starts_with('@')),
                info.description.clone(),
                info.exe_path.as_deref().map(friendly_exe_name),
            ]
            .into_iter()
            .flatten()
            .find(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Process {pid}"));
            (key, name, info.exe_path.clone(), info.aumid.clone())
        };

        let changed = Arc::new(AtomicBool::new(false));
        let events = SessionEvents::create(tx.clone(), changed.clone());
        unsafe { control.RegisterAudioSessionNotification(&events)? };
        Ok(Self {
            instance_id,
            control,
            simple,
            channels,
            meter,
            events,
            changed,
            pid,
            key,
            name,
            exe_path,
            aumid,
            icon_resource,
        })
    }

    /// Current values, or `None` if the session has expired.
    fn facts(&self, device_id: &str) -> WinResult<Option<SessionFacts>> {
        unsafe {
            let state = self.control.GetState()?;
            if state == AudioSessionStateExpired {
                return Ok(None);
            }
            let volume = self.simple.GetMasterVolume()?;
            let muted = self.simple.GetMute()?.as_bool();
            let balance = match &self.channels {
                Some(ch) if ch.GetChannelCount()? == 2 => {
                    Some(balance_from_channels(ch.GetChannelVolume(0)?, ch.GetChannelVolume(1)?))
                }
                _ => None,
            };
            Ok(Some(SessionFacts {
                id: self.instance_id.clone(),
                pid: self.pid,
                device_id: device_id.to_owned(),
                key: self.key.clone(),
                name: self.name.clone(),
                exe_path: self.exe_path.clone(),
                aumid: self.aumid.clone(),
                icon_resource: self.icon_resource.clone(),
                volume,
                muted,
                active: state == AudioSessionStateActive,
                balance,
            }))
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            let _ = self.control.UnregisterAudioSessionNotification(&self.events);
        }
    }
}

/// How an app is identified: package, then executable path, then process ID.
fn app_key(info: &ProcessInfo, pid: u32) -> AppKey {
    if let Some(aumid) = &info.aumid {
        AppKey::Package(aumid.to_lowercase())
    } else if let Some(path) = &info.exe_path {
        AppKey::Exe(path.to_string_lossy().to_lowercase())
    } else {
        AppKey::Pid(pid)
    }
}

fn apps_lowered_in_background(reductions: &Reductions, apps: &[AppKey]) -> Vec<AppKey> {
    apps.iter().filter(|key| reductions.background_reduction(key).is_some()).cloned().collect()
}

/// Resolves `@%SystemRoot%\System32\AudioSrv.Dll,-202`-style indirect strings.
fn resolve_indirect(s: &str) -> String {
    if !s.starts_with('@') {
        return s.to_string();
    }
    let mut buf = [0u16; 512];
    match unsafe { SHLoadIndirectString(&HSTRING::from(s), &mut buf, None) } {
        Ok(()) => {
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..len])
        }
        Err(_) => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_volume_commands_per_target() {
        let mut cmds = Vec::new();
        let a = AppKey::Pid(1);
        let b = AppKey::Pid(2);
        push_coalesced(&mut cmds, Command::SetAppVolume { key: a.clone(), volume: 0.1 });
        push_coalesced(&mut cmds, Command::SetAppVolume { key: b.clone(), volume: 0.2 });
        push_coalesced(&mut cmds, Command::SetAppMute { key: a.clone(), muted: true });
        push_coalesced(&mut cmds, Command::SetAppVolume { key: a.clone(), volume: 0.3 });
        push_coalesced(&mut cmds, Command::SetEndpointVolume { flow: Flow::Output, volume: 0.4 });
        push_coalesced(&mut cmds, Command::SetEndpointVolume { flow: Flow::Output, volume: 0.5 });
        assert_eq!(
            cmds,
            vec![
                Command::SetAppVolume { key: b, volume: 0.2 },
                Command::SetAppMute { key: a.clone(), muted: true },
                Command::SetAppVolume { key: a, volume: 0.3 },
                Command::SetEndpointVolume { flow: Flow::Output, volume: 0.5 },
            ]
        );
    }

    #[test]
    fn a_burst_of_device_events_refreshes_once_and_late_events_are_not_lost() {
        let window = DEVICE_COALESCE;
        let t0 = Instant::now();
        let mut c = Coalesce::default();
        assert!(!c.take_due(t0), "nothing pending");
        for ms in [0, 10, 30, 45] {
            c.note(t0 + Duration::from_millis(ms), window);
        }
        assert_eq!(c.due, Some(t0 + window), "later events in the burst don't push the refresh out");
        assert!(!c.take_due(t0 + Duration::from_millis(49)));
        assert!(c.take_due(t0 + window), "one refresh for the whole burst");
        assert!(!c.take_due(t0 + window * 2), "and only one");

        // An event after the refresh started opens a new window instead of being dropped.
        c.note(t0 + window + Duration::from_millis(5), window);
        assert!(c.take_due(t0 + window * 2 + Duration::from_millis(5)));

        // A refresh for another reason (such as a default-device change) covers pending events.
        c.note(t0, window);
        c.cancel();
        assert!(!c.take_due(t0 + window));
    }

    #[test]
    fn peaks_take_each_apps_loudest_session_and_drop_apps_that_left() {
        let (a, b, c) = (AppKey::Pid(1), AppKey::Pid(2), AppKey::Pid(3));
        let mut apps = HashMap::new();
        let first = [(a.clone(), 0.2), (b.clone(), 0.5), (a.clone(), 0.7)];
        let mut reads = 0;
        merge_peaks(
            &mut apps,
            first.iter(),
            |s| &s.0,
            |s| {
                reads += 1;
                s.1
            },
        );
        assert_eq!(reads, 3, "every session is read exactly once");
        assert_eq!(apps, HashMap::from([(a.clone(), 0.7), (b.clone(), 0.5)]));

        let capacity = apps.capacity();
        merge_peaks(&mut apps, [(c.clone(), 0.1), (a.clone(), 0.0)].iter(), |s| &s.0, |s| s.1);
        assert_eq!(apps, HashMap::from([(a.clone(), 0.0), (c, 0.1)]), "b left; a went silent");
        assert_eq!(apps.capacity(), capacity, "the map is reused, not rebuilt");

        merge_peaks(&mut apps, [].iter() as std::slice::Iter<(AppKey, f32)>, |s| &s.0, |s| s.1);
        assert!(apps.is_empty());
    }

    #[test]
    fn profile_notices_name_the_profile_and_what_was_not_applied() {
        let apps = PresetOutcome { applied: 2, missing: 0, failed: Vec::new() };
        let full = ProfileOutcome { devices_switched: true, master: true, apps: Some(apps), notes: Vec::new() };
        assert_eq!(full.notice("Gaming"), (NoticeLevel::Info, "Profile Gaming applied to 2 app(s).".into()));
        let devices_only = ProfileOutcome { devices_switched: true, ..ProfileOutcome::default() };
        assert_eq!(devices_only.notice("Media"), (NoticeLevel::Info, "Profile Media applied.".into()));
        let missing =
            ProfileOutcome { master: true, notes: vec!["Headset isn't connected".into()], ..Default::default() };
        assert_eq!(
            missing.notice("Work"),
            (NoticeLevel::Warning, "Profile Work applied. Not applied: Headset isn't connected.".into())
        );
        assert_eq!(ProfileOutcome::default().notice("Empty").0, NoticeLevel::Warning);
    }

    #[test]
    fn solo_focus_and_microphone_reductions_combine_per_app() {
        let (game, chat, music) =
            (AppKey::Exe("game.exe".into()), AppKey::Exe("chat.exe".into()), AppKey::Exe("music.exe".into()));
        let half = Reduction { factor: 0.5, mute: false };
        let quiet = Reduction { factor: 0.2, mute: false };
        let mut r = Reductions {
            solo: None,
            solo_level: quiet,
            focus_rules: vec![(game.clone(), half)],
            foreground: None,
            mic: Some((Reduction { factor: 0.3, mute: false }, vec![music.clone()])),
            mic_active: false,
            ledger: Ledger::default(),
        };
        assert_eq!(r.wanted(&game), None, "nothing is lowered before the foreground app is known");
        r.foreground = Some(chat.clone());
        assert_eq!(r.wanted(&game), Some(half), "the game is in the background");
        r.foreground = Some(game.clone());
        assert_eq!(r.wanted(&game), None, "back in front");
        assert_eq!(r.wanted(&music), None, "the microphone is quiet");
        r.mic_active = true;
        assert_eq!(r.wanted(&music).map(|x| x.factor), Some(0.3));
        assert_eq!(r.wanted(&chat), None, "only the microphone targets are lowered");
        r.solo = Some(chat.clone());
        r.foreground = Some(chat.clone());
        assert_eq!(r.wanted(&game).map(|x| x.factor), Some(0.2), "Solo and the focus rule: the stronger applies");
        assert_eq!(r.wanted(&chat), None, "the soloed app isn't lowered");
        assert_eq!(r.wanted(&AppKey::System), None, "System sounds is left alone");
        assert_eq!(apps_lowered_in_background(&r, &[game.clone(), music]), vec![game]);
    }

    #[test]
    fn meter_difference_threshold() {
        let mut a = Meters::default();
        let b = Meters::default();
        assert!(!meters_differ(&a, &b));
        a.apps.insert(AppKey::Pid(1), 0.005);
        assert!(!meters_differ(&a, &b));
        a.apps.insert(AppKey::Pid(1), 0.2);
        assert!(meters_differ(&a, &b));
        assert!(meters_differ(&b, &a), "an app only in the second set counts too");
        a.apps.insert(AppKey::Pid(1), 0.0);
        assert!(!meters_differ(&a, &b), "a silent app equals a missing one");
        a.output = 0.5;
        assert!(meters_differ(&a, &b));
    }

    #[test]
    fn preset_execution_continues_after_nonfatal_app_error_and_stops_on_fatal_error() {
        use super::super::com::AUDCLNT_E_DEVICE_INVALIDATED;
        use super::super::preferences::{PresetEntry, VolumePreset};
        use windows::Win32::Foundation::E_ACCESSDENIED;

        let (a, b, c) = (AppKey::Exe("a.exe".into()), AppKey::Exe("b.exe".into()), AppKey::Exe("c.exe".into()));
        let preset = VolumePreset {
            name: "Work".into(),
            apply_on_start: false,
            entries: vec![
                PresetEntry { key: a.clone(), name: "Alpha".into(), volume: 0.3, muted: false, output: None },
                PresetEntry { key: b.clone(), name: "Beta".into(), volume: 0.5, muted: true, output: None },
                PresetEntry { key: c.clone(), name: "Gamma".into(), volume: 0.8, muted: false, output: None },
            ],
            ..Default::default()
        };
        let live = [a.clone(), b.clone()];
        let mut intervened = Vec::new();
        let outcome = execute_preset(
            preset.clone(),
            &live,
            |k| k,
            |k, _vol, _muted| {
                if *k == a { Err(windows::core::Error::from_hresult(E_ACCESSDENIED)) } else { Ok(()) }
            },
            |k| intervened.push(k.clone()),
        )
        .expect("non-fatal error on Alpha must still apply Beta");
        assert_eq!(
            outcome,
            PresetOutcome { applied: 1, missing: 1, failed: vec!["Alpha".into()] },
            "Beta must be applied and Gamma counted as missing despite Alpha failing"
        );
        assert_eq!(intervened, vec![b.clone()]);
        assert_eq!(
            outcome.notice(),
            (NoticeLevel::Error, "Profile applied to 1 app(s); couldn't change Alpha.".into())
        );

        let fatal = execute_preset(
            preset,
            &live,
            |k| k,
            |_, _, _| Err(windows::core::Error::from_hresult(AUDCLNT_E_DEVICE_INVALIDATED)),
            |_| {},
        );
        assert!(matches!(fatal, Err(ApplyError::Windows(_, ref e)) if is_fatal(e.code())));
    }

    #[test]
    fn start_rules_execution_orders_processes_and_handles_partial_and_fatal_failures() {
        use super::super::com::AUDCLNT_E_DEVICE_INVALIDATED;
        use super::super::preferences::{PresetEntry, StartTracker, VolumePreset};
        use windows::Win32::Foundation::E_FAIL;

        let app = AppKey::Exe(r"c:\apps\player.exe".into());
        let rules = vec![VolumePreset {
            name: "Auto".into(),
            apply_on_start: true,
            entries: vec![PresetEntry {
                key: app.clone(),
                name: "Player".into(),
                volume: 0.4,
                muted: false,
                output: None,
            }],
            ..Default::default()
        }];
        let mut tracker = StartTracker::new(100);
        // Newer process (160, pid 20) appears before older process (150, pid 10) in the input slice,
        // and two distinct processes share timestamp 150 (pid 10 and pid 15).
        let candidates = vec![
            StartCandidate { id: "s-new", key: app.clone(), pid: 20, created: Some(160), untouched: true },
            StartCandidate { id: "s-old-b", key: app.clone(), pid: 15, created: Some(150), untouched: true },
            StartCandidate { id: "s-old-a", key: app.clone(), pid: 10, created: Some(150), untouched: true },
        ];
        let mut applied_ids = Vec::new();
        let mut notices = Vec::new();
        execute_start_rules(
            &rules,
            &mut tracker,
            candidates,
            |id, _key, vol, muted| {
                assert_eq!((vol, muted), (0.4, false));
                applied_ids.push(*id);
                if *id == "s-old-a" { Err(windows::core::Error::from_hresult(E_FAIL)) } else { Ok(()) }
            },
            |msg| notices.push(msg),
        )
        .expect("non-fatal session error is reported via notice without aborting");
        assert_eq!(
            applied_ids,
            vec!["s-old-a", "s-old-b", "s-new"],
            "processes must be ordered by (created, pid) and distinct PIDs with equal timestamps must both qualify"
        );
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("Player"));

        // A later refresh discovers an even older process (pid 5, created 120) whose audio session
        // appeared after pid 20 (created 160). It must still qualify once.
        let mut later_applied = Vec::new();
        execute_start_rules(
            &rules,
            &mut tracker,
            vec![
                StartCandidate { id: "s-delayed", key: app.clone(), pid: 5, created: Some(120), untouched: true },
                StartCandidate { id: "s-new-repeat", key: app.clone(), pid: 20, created: Some(160), untouched: true },
            ],
            |id, _key, _vol, _muted| {
                later_applied.push(*id);
                Ok(())
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(later_applied, vec!["s-delayed"]);

        // A fatal COM error aborts and returns Err so the engine rebuilds.
        let fatal_candidates =
            vec![StartCandidate { id: "s-fatal", key: app.clone(), pid: 30, created: Some(200), untouched: true }];
        let err = execute_start_rules(
            &rules,
            &mut tracker,
            fatal_candidates,
            |_, _, _, _| Err(windows::core::Error::from_hresult(AUDCLNT_E_DEVICE_INVALIDATED)),
            |_| {},
        )
        .unwrap_err();
        assert!(is_fatal(err.code()));

        // Even after exceeding tracker capacity with 1,024 additional live processes,
        // the previously failed pid 10 (created 150) recreating a session at 100% unmuted
        // must remain ineligible.
        for offset in 0..1024u32 {
            let _ = tracker.observe_process(&app, 1000 + offset, 300 + u64::from(offset), true);
        }
        let mut retried_after_capacity = Vec::new();
        execute_start_rules(
            &rules,
            &mut tracker,
            vec![StartCandidate { id: "s-old-a-recreated", key: app, pid: 10, created: Some(150), untouched: true }],
            |id, _key, _vol, _muted| {
                retried_after_capacity.push(*id);
                Ok(())
            },
            |_| {},
        )
        .unwrap();
        assert!(
            retried_after_capacity.is_empty(),
            "previously failed or considered live process must not re-qualify on recreated session after capacity"
        );
    }
}
