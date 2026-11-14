//! Engine thread loop, command coalescing, automation polling, and retry scheduling.

use super::error::ApplyError;
use super::{Engine, open_settings};
use crate::audio::attenuation::Reduction;
use crate::audio::automation::{FocusSettle, MicGate};
use crate::audio::com::{ComGuard, is_fatal};
use crate::audio::model::{AppKey, AudioSnapshot, Meters, ServiceStatus};
use crate::audio::power::PowerWatch;
use crate::audio::preferences::VolumePreset;
use crate::audio::properties::{MONO_SETTINGS_URI, SOUND_SETTINGS_URI};
use crate::audio::{Command, MicDucking, Msg, NoticeLevel, Shared};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) const METER_INTERVAL: Duration = Duration::from_millis(33);
const RESUME_SETTLE: Duration = Duration::from_millis(1500);
/// Device add/remove/state/property notifications arrive in bursts (a Bluetooth
/// reconnect sends several); they are merged into one re-enumeration.
pub(super) const DEVICE_COALESCE: Duration = Duration::from_millis(50);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Focus has to stay on an app this long before focus rules react.
const FOCUS_SETTLE: Duration = Duration::from_millis(250);
/// How often the microphone level is read while microphone ducking is on.
const MIC_INTERVAL: Duration = Duration::from_millis(50);

/// A deadline set by the first event of a burst. Later events in the window
/// don't extend it, so a steady stream still refreshes every window.
#[derive(Default)]
pub(super) struct Coalesce {
    pub(super) due: Option<Instant>,
}

impl Coalesce {
    pub(super) fn note(&mut self, now: Instant, window: Duration) {
        self.due.get_or_insert(now + window);
    }

    /// Whether the window has elapsed; resets it if so.
    pub(super) fn take_due(&mut self, now: Instant) -> bool {
        let due = self.due.is_some_and(|t| now >= t);
        if due {
            self.due = None;
        }
        due
    }

    /// A refresh that already covers the pending events.
    pub(super) fn cancel(&mut self) {
        self.due = None;
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct Dirty {
    pub(super) rebuild: bool,
    pub(super) devices: bool,
    pub(super) sessions: bool,
    pub(super) values: bool,
}

impl Dirty {
    fn any(&self) -> bool {
        self.rebuild || self.devices || self.sessions || self.values
    }
}

pub(crate) fn run(rx: Receiver<Msg>, tx: Sender<Msg>, shared: Arc<Shared>) {
    let _com = ComGuard::multithreaded();
    let _power = PowerWatch::register(tx.clone());
    let mut runner = Runner::new(tx, shared);

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

/// When to run another reduction pass after a nonfatal read or write failed. It is
/// independent of the structural retry (`retry_at`): a failed reduction must not
/// withhold a healthy snapshot, and a healthy one must not hide a failed reduction.
struct ReductionRetry {
    at: Option<Instant>,
    backoff: Duration,
}

impl Default for ReductionRetry {
    fn default() -> Self {
        Self { at: None, backoff: Duration::from_millis(250) }
    }
}

impl ReductionRetry {
    /// Records the outcome of a pass: schedules the next one with a growing,
    /// capped delay while failures remain, and starts over once none do.
    fn note(&mut self, pending: bool, now: Instant) {
        if pending {
            self.at = Some(now + self.backoff);
            self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
        } else {
            *self = Self::default();
        }
    }

    fn take_due(&mut self, now: Instant) -> bool {
        if self.at.is_some_and(|at| now >= at) {
            self.at = None;
            true
        } else {
            false
        }
    }
}

pub(super) struct Runner {
    pub(super) engine: Option<Engine>,
    tx: Sender<Msg>,
    pub(super) shared: Arc<Shared>,
    dirty: Dirty,
    retry_at: Option<Instant>,
    backoff: Duration,
    reduction_retry: ReductionRetry,
    delayed_rebuild: Option<Instant>,
    device_events: Coalesce,
    pub(super) last_meter_poll: Instant,
    pub(super) last_meters: Arc<Meters>,
    /// Reused for every poll, so a steady set of sessions polls without allocating.
    pub(super) meter_scratch: Meters,
    was_unavailable: bool,
    pub(super) devices_loaded: bool,
    preset_rules: Vec<VolumePreset>,
    pinned_outputs: Vec<String>,
    solo: Option<AppKey>,
    solo_level: Reduction,
    focus_rules: Vec<(AppKey, Reduction)>,
    focus: FocusSettle<u32>,
    mic: Option<MicDucking>,
    mic_gate: MicGate,
    last_mic_poll: Instant,
}

impl Runner {
    pub(super) fn new(tx: Sender<Msg>, shared: Arc<Shared>) -> Self {
        Self {
            engine: None,
            tx,
            shared,
            dirty: Dirty::default(),
            retry_at: Some(Instant::now()),
            backoff: Duration::from_millis(250),
            reduction_retry: ReductionRetry::default(),
            delayed_rebuild: None,
            device_events: Coalesce::default(),
            last_meter_poll: Instant::now(),
            last_meters: Arc::default(),
            meter_scratch: Meters::default(),
            was_unavailable: false,
            devices_loaded: false,
            preset_rules: Vec::new(),
            pinned_outputs: Vec::new(),
            solo: None,
            solo_level: Reduction { factor: 0.2, mute: false },
            focus_rules: Vec::new(),
            focus: FocusSettle::default(),
            mic: None,
            mic_gate: MicGate::default(),
            last_mic_poll: Instant::now(),
        }
    }

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

    pub(super) fn apply(&mut self, cmd: Command) {
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
                // Kept as a PID so it survives an engine outage; the settled PID is
                // resolved to an app by the current engine in `sync_automation`.
                self.focus.report(*pid, Instant::now());
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
                if matches!(cmd, Command::ToggleSolo { .. } | Command::EndSolo) {
                    self.solo.clone_from(&engine.reductions.solo);
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
            let uri = if cmd == Command::OpenSoundSettings { SOUND_SETTINGS_URI } else { MONO_SETTINGS_URI };
            if let Err(e) = open_settings(uri) {
                self.shared.push_notice(NoticeLevel::Error, e);
            }
            return;
        }
        if cmd == Command::LoadDevices {
            self.devices_loaded = true;
            if self.engine.is_none() {
                return;
            }
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
        if self.reduction_retry.take_due(now) {
            self.dirty.values = true;
        }
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
            self.reduction_retry = ReductionRetry::default();
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
                    engine.reductions.solo.clone_from(&self.solo);
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
        let engine_solo = engine.reductions.solo.clone();
        let reduction_retry_pending = engine.reduction_retry_pending;
        match result {
            Ok(snapshot) => {
                self.solo = engine_solo;
                self.reduction_retry.note(reduction_retry_pending, now);
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
                self.reduction_retry = ReductionRetry::default();
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
        if let Some(t) = self.reduction_retry.at
            && self.engine.is_some()
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

    /// Hands the current automation state to the engine and re-reads levels.
    fn sync_automation(&mut self) {
        let Some(engine) = self.engine.as_mut() else { return };
        let foreground = self.focus.applied().map(|pid| engine.key_for_pid(*pid));
        let r = &mut engine.reductions;
        r.focus_rules.clone_from(&self.focus_rules);
        r.foreground = foreground;
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
pub(super) fn push_coalesced(commands: &mut Vec<Command>, cmd: Command) {
    let same_target = |a: &Command, b: &Command| match (a, b) {
        (Command::SetEndpointVolume { flow: x, .. }, Command::SetEndpointVolume { flow: y, .. }) => x == y,
        (Command::SetAppVolume { key: x, .. }, Command::SetAppVolume { key: y, .. })
        | (Command::SetAppBalance { key: x, .. }, Command::SetAppBalance { key: y, .. }) => x == y,
        (Command::SetDeviceVolume { id: x, .. }, Command::SetDeviceVolume { id: y, .. })
        | (Command::SetDeviceBalance { id: x, .. }, Command::SetDeviceBalance { id: y, .. })
        | (Command::SetStreamVolume { id: x, .. }, Command::SetStreamVolume { id: y, .. }) => x == y,
        _ => false,
    };
    commands.retain(|c| !same_target(c, &cmd));
    commands.push(cmd);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_reports_are_kept_without_an_engine() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut runner = Runner::new(tx, Arc::new(Shared::default()));
        runner.apply(Command::SetFocusRules(vec![(AppKey::Pid(42), Reduction { factor: 0.3, mute: false })]));
        runner.apply(Command::ForegroundChanged { pid: 42 });
        assert!(runner.engine.is_none());
        assert!(runner.focus.due(FOCUS_SETTLE).is_some(), "outage must not drop foreground");

        runner.apply(Command::ForegroundChanged { pid: 99 });
        assert!(runner.focus.settle(FOCUS_SETTLE, Instant::now() + FOCUS_SETTLE));
        assert_eq!(runner.focus.applied(), Some(&99), "the latest report wins");
        runner.apply(Command::SetFocusRules(Vec::new()));
        assert!(runner.focus.applied().is_none(), "disabling rules clears focus");
    }

    #[test]
    fn reduction_retry_wakes_without_messages_and_resets_after_success() {
        let now = Instant::now();
        let mut retry = ReductionRetry::default();
        retry.note(true, now);
        assert!(!retry.take_due(now + Duration::from_millis(249)));
        assert!(retry.take_due(now + Duration::from_millis(250)));
        retry.note(true, now + Duration::from_millis(250));
        assert_eq!(retry.at, Some(now + Duration::from_millis(750)));
        retry.note(false, now + Duration::from_millis(300));
        assert!(retry.at.is_none());
        retry.note(true, now + Duration::from_millis(400));
        assert_eq!(retry.at, Some(now + Duration::from_millis(650)));
    }

    #[test]
    fn reduction_retry_backoff_never_exceeds_the_cap() {
        let now = Instant::now();
        let mut retry = ReductionRetry::default();
        for _ in 0..20 {
            retry.note(true, now);
            assert!(retry.at.is_some_and(|at| at <= now + MAX_BACKOFF));
        }
        retry.note(false, now);
        assert!(retry.at.is_none());
    }

    #[test]
    fn reduction_retry_deadline_is_ignored_while_the_engine_is_absent() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut runner = Runner::new(tx, Arc::new(Shared::default()));
        runner.retry_at = None;
        assert!(runner.next_timeout().is_none());
        runner.reduction_retry.at = Some(Instant::now());
        assert!(runner.next_timeout().is_none(), "no engine, no retry wake");
    }
}
