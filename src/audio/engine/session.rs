//! Per-output audio session sets, session wrappers, and process tracking.

use super::WinResult;
use crate::audio::Msg;
use crate::audio::callbacks::{SessionCreatedNotifier, SessionEvents};
use crate::audio::com::{is_fatal, take_pwstr};
use crate::audio::model::{AppKey, SessionFacts, balance_from_channels, friendly_exe_name};
use crate::audio::process::{self, ProcessInfo, ProcessWatch};
use crossbeam_channel::Sender;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionEvents,
    IAudioSessionManager2, IAudioSessionNotification, IChannelAudioVolume, IMMDevice, ISimpleAudioVolume,
};
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::UI::Shell::SHLoadIndirectString;
use windows::core::{HSTRING, Interface};

pub(super) struct SessionSet {
    pub(super) device_id: String,
    manager: IAudioSessionManager2,
    notifier: IAudioSessionNotification,
    pub(super) sessions: Vec<Session>,
    pub(super) processes: HashMap<u32, ProcessEntry>,
}

pub(super) struct ProcessEntry {
    info: ProcessInfo,
    watch: Option<ProcessWatch>,
    pub(super) started_at: Option<u64>,
}

impl SessionSet {
    pub(super) fn open(device: &IMMDevice, device_id: String, tx: Sender<Msg>) -> WinResult<Self> {
        let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let notifier = SessionCreatedNotifier::create(tx.clone());
        unsafe { manager.RegisterSessionNotification(&notifier)? };
        let mut set = Self { device_id, manager, notifier, sessions: Vec::new(), processes: HashMap::new() };
        // Enumerating after registering is required for creation notifications to be delivered.
        let _ = set.sync(&tx)?;
        Ok(set)
    }

    pub(super) fn has_exited(&self, pid: u32) -> bool {
        self.processes.get(&pid).and_then(|p| p.watch.as_ref()).is_some_and(ProcessWatch::has_exited)
    }

    pub(super) fn sync(&mut self, tx: &Sender<Msg>) -> WinResult<Vec<u32>> {
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

pub(super) struct Session {
    pub(super) instance_id: String,
    control: IAudioSessionControl2,
    pub(super) simple: ISimpleAudioVolume,
    pub(super) channels: Option<IChannelAudioVolume>,
    pub(super) meter: Option<IAudioMeterInformation>,
    events: IAudioSessionEvents,
    pub(super) changed: Arc<AtomicBool>,
    pub(super) pid: u32,
    pub(super) key: AppKey,
    pub(super) name: String,
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
    pub(super) fn facts(&self, device_id: &str) -> WinResult<Option<SessionFacts>> {
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
pub(super) fn app_key(info: &ProcessInfo, pid: u32) -> AppKey {
    if let Some(aumid) = &info.aumid {
        AppKey::Package(aumid.to_lowercase())
    } else if let Some(path) = &info.exe_path {
        AppKey::Exe(path.to_string_lossy().to_lowercase())
    } else {
        AppKey::Pid(pid)
    }
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
