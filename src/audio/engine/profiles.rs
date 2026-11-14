//! Profile application, manual presets, and automatic start-rule execution.

use super::error::{ApplyError, Context};
use super::runner::Dirty;
use super::session::Session;
use super::{Engine, WinResult, current_filetime};
use crate::audio::callbacks::OWN_EVENT_CONTEXT;
use crate::audio::com::is_fatal;
use crate::audio::model::{AppKey, Flow};
use crate::audio::preferences::{StartTracker, VolumePreset, plan_preset};
use crate::audio::{NoticeLevel, Shared};
use std::collections::HashMap;
use std::sync::atomic::Ordering;

impl Engine {
    pub(super) fn set_preset_rules(&mut self, presets: Vec<VolumePreset>) {
        self.preset_rules = presets.into_iter().filter(|p| p.apply_on_start).collect();
        self.start_tracker.rearm(current_filetime());
    }

    /// Applies a whole profile: default devices first (app levels are kept per
    /// device, so the sessions are re-read on the new one), then the master
    /// level, then the app levels.
    pub(super) fn apply_profile(
        &mut self,
        profile: &VolumePreset,
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
            if let Some(choice) = device.device()
                && !self.outputs.iter().any(|d| d.id.eq_ignore_ascii_case(&choice.id))
            {
                outcome.notes.push(format!("{} isn't connected for {}", choice.name, entry.name));
            } else {
                match self.route_app(&entry.key, device.device().map(|d| d.id.as_str())) {
                    Ok(None) => {}
                    Ok(Some(warning)) => outcome.notes.push(format!("{}'s device ({warning})", entry.name)),
                    Err(e) => outcome.notes.push(format!("{}'s device ({e})", entry.name)),
                }
            }
        }
        Ok(outcome)
    }

    fn apply_preset(&mut self, preset: VolumePreset) -> Result<PresetOutcome, ApplyError> {
        let mut intervened = Vec::new();
        let outcome = {
            let sessions: Vec<&Session> = self.all_sessions().collect();
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

    pub(super) fn mark_key_intervened(&mut self, key: &AppKey) {
        for set in &self.sessions {
            for session in set.sessions.iter().filter(|s| &s.key == key) {
                if let Some(created) = set.processes.get(&session.pid).and_then(|p| p.started_at) {
                    self.start_tracker.note_process_intervention(key, session.pid, created);
                }
            }
        }
    }

    pub(super) fn apply_start_rules(&mut self, shared: &Shared) -> WinResult<()> {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PresetOutcome {
    pub(super) applied: usize,
    pub(super) missing: usize,
    pub(super) failed: Vec<String>,
}

impl PresetOutcome {
    pub(super) fn notice(&self) -> (NoticeLevel, String) {
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
pub(super) struct ProfileOutcome {
    pub(super) devices_switched: bool,
    pub(super) master: bool,
    pub(super) apps: Option<PresetOutcome>,
    /// Parts that could not be applied, in plain words.
    pub(super) notes: Vec<String>,
}

impl ProfileOutcome {
    pub(super) fn notice(&self, name: &str) -> (NoticeLevel, String) {
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

pub(super) fn execute_preset<S>(
    preset: VolumePreset,
    sessions: &[S],
    key_of: impl Fn(&S) -> &AppKey,
    mut apply_one: impl FnMut(&S, f32, bool) -> WinResult<()>,
    mut on_intervened: impl FnMut(&AppKey),
) -> Result<PresetOutcome, ApplyError> {
    let Some(preset) = preset.sanitize() else {
        return Err(ApplyError::Message("Preset has no valid name.".into()));
    };
    let plan = plan_preset(&preset, |key| sessions.iter().any(|s| key_of(s) == key));
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

pub(super) struct StartCandidate<I> {
    pub(super) id: I,
    pub(super) key: AppKey,
    pub(super) pid: u32,
    pub(super) created: Option<u64>,
    pub(super) untouched: bool,
}

pub(super) fn execute_start_rules<I>(
    preset_rules: &[VolumePreset],
    start_tracker: &mut StartTracker,
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
