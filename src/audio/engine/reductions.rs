//! Solo, focus-rule, and microphone-ducking reductions and ledger writes.

use super::error::ApplyError;
use super::session::{Session, SessionSet};
use super::{Engine, WinResult, iter_sessions};
use crate::audio::Shared;
use crate::audio::attenuation::{Ledger, Level, Reduction};
use crate::audio::callbacks::OWN_EVENT_CONTEXT;
use crate::audio::com::is_fatal;
use crate::audio::model::AppKey;

/// Solo and ducking state: what should be lowered, and the ledger of
/// what was written so it can be undone safely.
pub(super) struct Reductions {
    pub(super) solo: Option<AppKey>,
    pub(super) solo_level: Reduction,
    pub(super) focus_rules: Vec<(AppKey, Reduction)>,
    /// The settled foreground app, `None` until one is known.
    pub(super) foreground: Option<AppKey>,
    pub(super) mic: Option<(Reduction, Vec<AppKey>)>,
    pub(super) mic_active: bool,
    pub(super) ledger: Ledger,
}

impl Reductions {
    pub(super) fn wanted(&self, key: &AppKey) -> Option<Reduction> {
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
    pub(super) fn background_reduction(&self, key: &AppKey) -> Option<Reduction> {
        let foreground = self.foreground.as_ref()?;
        if foreground == key {
            return None;
        }
        self.focus_rules.iter().find(|(rule, _)| rule == key).map(|(_, reduction)| *reduction)
    }

    pub(super) fn active(&self) -> bool {
        self.solo.is_some()
            || !self.ledger.is_empty()
            || self.mic_active
            || (self.foreground.is_some() && !self.focus_rules.is_empty())
    }
}

impl Engine {
    pub(super) fn toggle_solo(&mut self, key: &AppKey) -> Result<Option<String>, ApplyError> {
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

    pub(super) fn end_solo(&mut self) -> Result<Option<String>, ApplyError> {
        let Some(key) = self.reductions.solo.take() else {
            return Ok(None);
        };
        log::info!("solo ended for {key:?}");
        let name = self.app_name(&key).unwrap_or_else(|| "the app".into());
        Ok(Some(format!("Solo ended for {name}")))
    }

    /// Writes the reductions that should apply now and undoes those that ended.
    pub(super) fn apply_reductions(&mut self, shared: &Shared) -> WinResult<()> {
        if let Some(key) = &self.reductions.solo
            && !self.all_sessions().any(|s| &s.key == key)
        {
            log::info!("solo ended: {key:?} has no audio session left");
            self.reductions.solo = None;
            shared.push_announcement("Solo ended: the app closed its audio.");
        }
        if !self.reductions.active() {
            return Ok(());
        }
        let sessions = iter_sessions(&self.sessions).map(|s| (s, s.instance_id.as_str(), s.name.as_str(), &s.key));
        execute_reductions(
            &mut self.reductions,
            sessions,
            |s| unsafe {
                s.simple.GetMasterVolume().and_then(|volume| Ok(Level { volume, muted: s.simple.GetMute()?.as_bool() }))
            },
            |s, v| unsafe { s.simple.SetMasterVolume(v, &OWN_EVENT_CONTEXT) },
            |s, m| unsafe { s.simple.SetMute(m, &OWN_EVENT_CONTEXT) },
        )
    }

    /// Puts back every level a reduction changed (when it is still the written
    /// one), before the sessions are released.
    pub(super) fn release_reductions(&mut self) {
        let sets: Vec<&SessionSet> = self.sessions.iter().collect();
        release_in(&mut self.reductions.ledger, &sets);
        self.reductions.ledger = Ledger::default();
    }
}

/// Restores the levels reductions changed in `sets` and forgets those sessions.
pub(super) fn release_in(ledger: &mut Ledger, sets: &[&SessionSet]) {
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
            && write_reduction(ledger, s, current, base, true).is_err()
        {
            failed += 1;
        }
    }
    if failed > 0 {
        log::warn!("{failed} lowered session(s) could not be restored");
    }
}

pub(super) fn execute_reductions<'a, S: 'a>(
    reductions: &mut Reductions,
    sessions: impl Iterator<Item = (&'a S, &'a str, &'a str, &'a AppKey)> + Clone,
    mut read_level: impl FnMut(&S) -> WinResult<Level>,
    mut set_volume: impl FnMut(&S, f32) -> WinResult<()>,
    mut set_mute: impl FnMut(&S, bool) -> WinResult<()>,
) -> WinResult<()> {
    for (s, instance_id, name, key) in sessions.clone() {
        let wanted = reductions.wanted(key);
        if wanted.is_none() && !reductions.ledger.contains(instance_id) {
            continue;
        }
        let current = match read_level(s) {
            Ok(level) => level,
            Err(e) if is_fatal(e.code()) => return Err(e),
            Err(e) => {
                log::debug!("couldn't read {name} for a reduction: {e}");
                continue;
            }
        };
        if let Some(target) = reductions.ledger.reconcile(instance_id, current, wanted)
            && let Err(e) = write_reduction_with(
                &mut reductions.ledger,
                instance_id,
                current,
                target,
                wanted.is_none(),
                |v| set_volume(s, v),
                |m| set_mute(s, m),
            )
        {
            if is_fatal(e.code()) {
                return Err(e);
            }
            log::warn!("couldn't change {name} for a reduction: {e}");
        }
    }
    reductions.ledger.retain(|id| sessions.clone().any(|(_, instance_id, _, _)| instance_id == id));
    Ok(())
}

fn write_reduction_with(
    ledger: &mut Ledger,
    instance_id: &str,
    current: Level,
    level: Level,
    restoring: bool,
    set_volume: impl FnOnce(f32) -> WinResult<()>,
    set_mute: impl FnOnce(bool) -> WinResult<()>,
) -> WinResult<()> {
    set_volume(level.volume)?;
    ledger.record_written(instance_id, Level { volume: level.volume, muted: current.muted });
    set_mute(level.muted)?;
    ledger.record_written(instance_id, level);
    if restoring {
        ledger.finish_restore(instance_id);
    }
    Ok(())
}

fn write_reduction(
    ledger: &mut Ledger,
    session: &Session,
    current: Level,
    level: Level,
    restoring: bool,
) -> WinResult<()> {
    write_reduction_with(
        ledger,
        &session.instance_id,
        current,
        level,
        restoring,
        |v| unsafe { session.simple.SetMasterVolume(v, &OWN_EVENT_CONTEXT) },
        |m| unsafe { session.simple.SetMute(m, &OWN_EVENT_CONTEXT) },
    )
}

pub(super) fn apps_lowered_in_background(reductions: &Reductions, apps: &[AppKey]) -> Vec<AppKey> {
    apps.iter().filter(|key| reductions.background_reduction(key).is_some()).cloned().collect()
}
