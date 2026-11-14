//! The audio engine thread: owns every Core Audio object, applies commands and
//! publishes snapshots re-read from Windows.

mod endpoint;
mod error;
mod meters;
mod profiles;
mod reductions;
mod routing;
mod runner;
mod session;

pub(crate) use runner::run;

use super::app_routing::AppRouting;
use super::attenuation::{Ledger, Reduction};
use super::callbacks::{DeviceNotifier, OWN_EVENT_CONTEXT};
use super::com::is_fatal;
use super::model::*;
use super::policy_config::PolicyConfig;
use super::preferences::{StartTracker, VolumePreset};
use super::process;
use super::properties::{DeviceProperties, read_device_properties};
use super::{Command, Msg, NoticeLevel, Shared};
use crossbeam_channel::Sender;
use endpoint::{Endpoint, default_device_id, list_devices};
use error::{ApplyError, Context};
use reductions::{Reductions, apps_lowered_in_background, release_in};
use runner::Dirty;
use session::{Session, SessionSet, app_key};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, IMMNotificationClient, MMDeviceEnumerator, eCapture, eRender};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::HSTRING;

/// How often process markers are checked for exits that were never observed,
/// piggybacked on session refreshes (there is no timer).
const RECLAIM_INTERVAL: Duration = Duration::from_secs(300);
/// The same check while the tracker is large or saturated.
const RECLAIM_RETRY: Duration = Duration::from_secs(10);

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

fn iter_sessions(sets: &[SessionSet]) -> impl Iterator<Item = &Session> + Clone {
    sets.iter().flat_map(|set| set.sessions.iter())
}

pub(super) struct Engine {
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
    properties: HashMap<String, DeviceProperties>,
    preset_rules: Vec<VolumePreset>,
    start_tracker: StartTracker,
    last_reclaim: Instant,
    pinned_outputs: Vec<String>,
    /// Set by `cycle_output`, so the resulting switch is announced.
    announce_output_until: Option<Instant>,
    reductions: Reductions,
    /// Set by the last reduction pass when a nonfatal read or write failed and
    /// another pass is needed; the runner schedules it.
    reduction_retry_pending: bool,
    /// Set by the last snapshot when a session couldn't be read for a nonfatal
    /// reason that may pass; the runner schedules another read with a backoff
    /// instead of this engine re-queuing itself in a tight loop.
    session_retry_pending: bool,
    routing: Option<AppRouting>,
    /// Each process's device preference as last read from (or written to)
    /// Windows. Read once per process: Windows doesn't notify changes.
    routes: HashMap<(u32, Option<u64>), Route>,
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
            start_tracker: StartTracker::new(current_filetime()),
            last_reclaim: Instant::now(),
            pinned_outputs: Vec::new(),
            announce_output_until: None,
            routing,
            routes: HashMap::new(),
            reduction_retry_pending: false,
            session_retry_pending: false,
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

    fn all_sessions(&self) -> impl Iterator<Item = &Session> + Clone {
        iter_sessions(&self.sessions)
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
                        let props = read_device_properties(&ep.device, flow == Flow::Output);
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
        self.session_retry_pending = false;
        let mut facts = Vec::new();
        let mut needs_sync = false;
        let mut unreadable = false;
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
                    // Re-enumerating can't fix a session that is still listed but
                    // unreadable, so retry on the runner's backoff, not by message.
                    Err(e) => {
                        log::debug!("skipping session {}: {e}", s.instance_id);
                        unreadable = true;
                    }
                }
            }
        }
        self.session_retry_pending = unreadable;
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

    /// The app identity of a process, as its sessions would have it.
    fn key_for_pid(&self, pid: u32) -> AppKey {
        if let Some(s) = self.all_sessions().find(|s| s.pid == pid) {
            return s.key.clone();
        }
        let info = process::open(pid).map(|handle| process::query(&handle)).unwrap_or_default();
        app_key(&info, pid)
    }

    /// Applies a command. Returns whether device lists must be re-read.
    fn apply(&mut self, cmd: &Command, shared: &Shared) -> Result<bool, ApplyError> {
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
            Command::ToggleOutputMute => {
                let ep = self.endpoint(Flow::Output)?;
                let what = || format!("Couldn't change mute for {}", ep.name);
                let muted = unsafe { ep.volume.GetMute() }.context(what)?.as_bool();
                unsafe { ep.volume.SetMute(!muted, ctx) }.context(what)?;
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
                if let Some(warning) = self.route_app(key, device.as_deref())? {
                    shared.push_notice(NoticeLevel::Warning, warning);
                }
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
                    .all_sessions()
                    .filter(|s| &s.key == key)
                    .all(|s| unsafe { s.simple.GetMute() }.is_ok_and(|m| m.as_bool()));
                self.for_app(key, "mute", |s| unsafe { s.simple.SetMute(!all_muted, ctx) })?;
                self.mark_key_intervened(key);
            }
            Command::AdjustAppVolume { key, delta } => {
                let current = self
                    .all_sessions()
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

    fn stream(&self, id: &str) -> Result<&Session, ApplyError> {
        self.all_sessions()
            .find(|s| s.instance_id == id)
            .ok_or_else(|| ApplyError::Message("That audio stream has ended; the list has been refreshed.".into()))
    }

    fn app_name(&self, key: &AppKey) -> Option<String> {
        self.all_sessions().find(|s| &s.key == key).map(|s| s.name.clone())
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
        let sessions: Vec<&Session> = self.all_sessions().filter(|s| &s.key == key).collect();
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

#[cfg(test)]
mod tests {
    use super::meters::{merge_peaks, meters_differ};
    use super::profiles::{PresetOutcome, ProfileOutcome, StartCandidate, execute_preset, execute_start_rules};
    use super::reductions::{apps_lowered_in_background, execute_reductions};
    use super::runner::{Coalesce, DEVICE_COALESCE, Runner, push_coalesced};
    use super::*;
    use crate::audio::attenuation::Level;
    use std::sync::Arc;

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
        push_coalesced(&mut cmds, Command::SetStreamVolume { id: "s1".into(), volume: 0.1 });
        push_coalesced(&mut cmds, Command::SetStreamVolume { id: "s2".into(), volume: 0.2 });
        push_coalesced(&mut cmds, Command::SetStreamVolume { id: "s1".into(), volume: 0.8 });
        assert_eq!(
            cmds,
            vec![
                Command::SetAppVolume { key: b, volume: 0.2 },
                Command::SetAppMute { key: a.clone(), muted: true },
                Command::SetAppVolume { key: a, volume: 0.3 },
                Command::SetEndpointVolume { flow: Flow::Output, volume: 0.5 },
                Command::SetStreamVolume { id: "s2".into(), volume: 0.2 },
                Command::SetStreamVolume { id: "s1".into(), volume: 0.8 },
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

    #[test]
    fn reductions_skip_nonfatal_session_errors_and_abort_on_fatal() {
        use super::super::com::AUDCLNT_E_DEVICE_INVALIDATED;
        use windows::Win32::Foundation::E_ACCESSDENIED;

        struct Stub {
            id: &'static str,
            name: &'static str,
            key: AppKey,
            read_err: Option<windows::core::HRESULT>,
            write_err: Option<windows::core::HRESULT>,
        }

        let (a, b, c) = (AppKey::Exe("a.exe".into()), AppKey::Exe("b.exe".into()), AppKey::Exe("c.exe".into()));
        let mut r = Reductions {
            solo: Some(AppKey::Exe("solo.exe".into())),
            solo_level: Reduction { factor: 0.2, mute: false },
            focus_rules: Vec::new(),
            foreground: None,
            mic: None,
            mic_active: false,
            ledger: Ledger::default(),
        };
        let stubs = [
            Stub { id: "s1", name: "BadRead", key: a, read_err: Some(E_ACCESSDENIED), write_err: None },
            Stub { id: "s2", name: "BadWrite", key: b, read_err: None, write_err: Some(E_ACCESSDENIED) },
            Stub { id: "s3", name: "Healthy", key: c, read_err: None, write_err: None },
        ];
        let mut applied = Vec::new();
        execute_reductions(
            &mut r,
            stubs.iter().map(|s| (s, s.id, s.name, &s.key)),
            |s| match s.read_err {
                Some(hr) => Err(windows::core::Error::from_hresult(hr)),
                None => Ok(Level { volume: 1.0, muted: false }),
            },
            |s, v| match s.write_err {
                Some(hr) => Err(windows::core::Error::from_hresult(hr)),
                None => {
                    applied.push((s.id, v));
                    Ok(())
                }
            },
            |_s, _m| Ok(()),
        )
        .expect("non-fatal errors must skip and continue");
        assert_eq!(applied, vec![("s3", 0.2)]);

        let fatal_read = [Stub {
            id: "sf1",
            name: "FatalRead",
            key: AppKey::Exe("f1.exe".into()),
            read_err: Some(AUDCLNT_E_DEVICE_INVALIDATED),
            write_err: None,
        }];
        let err = execute_reductions(
            &mut r,
            fatal_read.iter().map(|s| (s, s.id, s.name, &s.key)),
            |s| Err(windows::core::Error::from_hresult(s.read_err.unwrap())),
            |_, _| Ok(()),
            |_, _| Ok(()),
        )
        .unwrap_err();
        assert!(is_fatal(err.code()));

        let fatal_write = [Stub {
            id: "sf2",
            name: "FatalWrite",
            key: AppKey::Exe("f2.exe".into()),
            read_err: None,
            write_err: Some(AUDCLNT_E_DEVICE_INVALIDATED),
        }];
        let err = execute_reductions(
            &mut r,
            fatal_write.iter().map(|s| (s, s.id, s.name, &s.key)),
            |_| Ok(Level { volume: 1.0, muted: false }),
            |s, _| Err(windows::core::Error::from_hresult(s.write_err.unwrap())),
            |_, _| Ok(()),
        )
        .unwrap_err();
        assert!(is_fatal(err.code()));
    }

    #[test]
    fn failed_restoration_remains_pending_until_success_or_user_intervention() {
        use crate::audio::attenuation::Level;
        use std::cell::Cell;
        use windows::Win32::Foundation::E_ACCESSDENIED;

        let key = AppKey::Exe("music.exe".into());
        let session = ();
        let base = Level { volume: 0.8, muted: false };
        let lowered = Level { volume: 0.2, muted: true };
        let current = Cell::new(lowered);
        let fail_mute = Cell::new(true);
        let mut reductions = Reductions {
            solo: None,
            solo_level: Reduction { factor: 0.25, mute: true },
            focus_rules: Vec::new(),
            foreground: None,
            mic: None,
            mic_active: false,
            ledger: Ledger::default(),
        };
        let _ = reductions.ledger.reconcile("s", base, Some(Reduction { factor: 0.25, mute: true }));
        reductions.ledger.record_written("s", lowered);
        let run_pass = |r: &mut Reductions| {
            execute_reductions(
                r,
                [(&session, "s", "Music", &key)].into_iter(),
                |_| Ok(current.get()),
                |_, volume| {
                    current.set(Level { volume, ..current.get() });
                    Ok(())
                },
                |_, muted| {
                    if fail_mute.replace(false) {
                        Err(windows::core::Error::from_hresult(E_ACCESSDENIED))
                    } else {
                        current.set(Level { muted, ..current.get() });
                        Ok(())
                    }
                },
            )
        };
        assert!(run_pass(&mut reductions).unwrap());
        assert!(reductions.ledger.contains("s"));
        assert!(!run_pass(&mut reductions).unwrap());
        assert_eq!(current.get(), base);
        assert!(reductions.ledger.is_empty());

        let _ = reductions.ledger.reconcile("s", base, Some(Reduction { factor: 0.25, mute: true }));
        reductions.ledger.record_written("s", lowered);
        current.set(lowered);
        fail_mute.set(true);
        assert!(run_pass(&mut reductions).unwrap());
        let user = Level { volume: 0.3, muted: false };
        current.set(user);
        assert!(!run_pass(&mut reductions).unwrap());
        assert_eq!(current.get(), user);
        assert!(reductions.ledger.is_empty());
    }

    #[test]
    fn load_devices_without_engine_records_flag_without_error_notice() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let shared = Arc::new(Shared::default());
        let mut runner = Runner::new(tx, shared.clone());
        runner.apply(Command::LoadDevices);
        assert!(runner.devices_loaded);
        assert!(shared.take_notices().is_empty());
    }
}
