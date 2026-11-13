//! Temporary level reductions (Solo and ducking) that are undone only when the
//! level is still what Glass Mixer wrote.
//!
//! For each session the ledger keeps the level it had before the first
//! reduction (the base) and the level last written. If the current level
//! differs from the written one, the user or another app changed it: the
//! session is released, and nothing is written to it again until every
//! reduction on it has ended. A user's change always wins.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub volume: f32,
    pub muted: bool,
}

impl Level {
    /// Equal within what a read-back of a written float can differ by.
    fn matches(self, other: Level) -> bool {
        self.muted == other.muted && (self.volume - other.volume).abs() < 0.002
    }
}

/// How much a session is lowered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reduction {
    /// Multiplier for the base volume, 0.0..=1.0.
    pub factor: f32,
    pub mute: bool,
}

impl Reduction {
    /// Two reductions on one session: the stronger of each part applies.
    pub fn combine(self, other: Reduction) -> Reduction {
        Reduction { factor: self.factor.min(other.factor), mute: self.mute || other.mute }
    }

    fn apply(self, base: Level) -> Level {
        Level { volume: (base.volume * self.factor.clamp(0.0, 1.0)).clamp(0.0, 1.0), muted: base.muted || self.mute }
    }
}

#[derive(Debug)]
struct Entry {
    base: Level,
    written: Level,
    released: bool,
}

/// Reductions in effect, by session instance ID.
#[derive(Debug, Default)]
pub struct Ledger {
    entries: HashMap<String, Entry>,
}

impl Ledger {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, session: &str) -> bool {
        self.entries.contains_key(session)
    }

    /// Brings one session in line with the reduction that should apply to it
    /// now (`None` when none does). Plans a write; call `record_written` after
    /// each successful Windows write. A failed write can then be retried.
    pub fn reconcile(&mut self, session: &str, current: Level, wanted: Option<Reduction>) -> Option<Level> {
        let Some(entry) = self.entries.get_mut(session) else {
            let target = wanted?.apply(current);
            self.entries.insert(session.to_owned(), Entry { base: current, written: current, released: false });
            return (!target.matches(current)).then_some(target);
        };
        if !entry.released && !current.matches(entry.written) {
            entry.released = true;
        }
        match wanted {
            Some(reduction) if !entry.released => {
                let target = reduction.apply(entry.base);
                if target.matches(entry.written) { None } else { Some(target) }
            }
            Some(_) => None,
            None => {
                if entry.released || entry.base.matches(current) {
                    self.entries.remove(session);
                    None
                } else {
                    Some(entry.base)
                }
            }
        }
    }

    /// Records only the fields Windows accepted, including a partially applied
    /// volume/mute pair. That partial write is not a user intervention.
    pub fn record_written(&mut self, session: &str, level: Level) {
        if let Some(entry) = self.entries.get_mut(session) {
            entry.written = level;
        }
    }

    pub fn finish_restore(&mut self, session: &str) {
        self.entries.remove(session);
    }

    /// Forgets sessions that no longer exist.
    pub fn retain(&mut self, live: impl Fn(&str) -> bool) {
        self.entries.retain(|session, _| live(session));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOLO: Reduction = Reduction { factor: 0.2, mute: false };
    const MUTE: Reduction = Reduction { factor: 1.0, mute: true };

    fn level(volume: f32) -> Level {
        Level { volume, muted: false }
    }

    fn apply(ledger: &mut Ledger, session: &str, current: Level, wanted: Option<Reduction>) -> Option<Level> {
        let target = ledger.reconcile(session, current, wanted);
        if let Some(level) = target {
            ledger.record_written(session, level);
            if wanted.is_none() {
                ledger.finish_restore(session);
            }
        }
        target
    }

    #[test]
    fn a_reduction_is_written_once_and_undone_when_it_ends() {
        let mut ledger = Ledger::default();
        let lowered = apply(&mut ledger, "s", level(0.5), Some(SOLO)).expect("lowered");
        assert!((lowered.volume - 0.1).abs() < 1e-6);
        assert_eq!(apply(&mut ledger, "s", lowered, Some(SOLO)), None, "no repeated writes");
        assert_eq!(apply(&mut ledger, "s", lowered, None), Some(level(0.5)), "restored to the level before");
        assert!(ledger.is_empty());
        assert_eq!(apply(&mut ledger, "s", level(0.5), None), None, "nothing to do without a reduction");
    }

    #[test]
    fn a_change_made_while_reduced_is_kept_and_never_overwritten() {
        let mut ledger = Ledger::default();
        apply(&mut ledger, "s", level(0.5), Some(SOLO));
        // The user raises the lowered app to 70 %.
        assert_eq!(apply(&mut ledger, "s", level(0.7), Some(SOLO)), None, "the user's level isn't lowered again");
        assert_eq!(apply(&mut ledger, "s", level(0.7), None), None, "and isn't replaced by the old level at the end");
        assert!(ledger.is_empty());
        // A later reduction starts from the user's level.
        let lowered = apply(&mut ledger, "s", level(0.7), Some(SOLO)).unwrap();
        assert!((lowered.volume - 0.14).abs() < 1e-6);
    }

    #[test]
    fn unmuting_a_muted_session_releases_it() {
        let mut ledger = Ledger::default();
        let muted = apply(&mut ledger, "s", level(0.4), Some(MUTE)).unwrap();
        assert_eq!(muted, Level { volume: 0.4, muted: true });
        assert_eq!(apply(&mut ledger, "s", level(0.4), None), None, "the user unmuted it; leave it as it is");
    }

    #[test]
    fn a_session_already_muted_stays_muted_after_the_reduction() {
        let mut ledger = Ledger::default();
        let base = Level { volume: 0.6, muted: true };
        let lowered = apply(&mut ledger, "s", base, Some(SOLO)).unwrap();
        assert!(lowered.muted);
        assert_eq!(apply(&mut ledger, "s", lowered, None), Some(base));
    }

    #[test]
    fn combined_reductions_apply_the_strongest_and_follow_changes() {
        let mut ledger = Ledger::default();
        let duck = Reduction { factor: 0.5, mute: false };
        let first = apply(&mut ledger, "s", level(0.8), Some(duck)).unwrap();
        assert!((first.volume - 0.4).abs() < 1e-6);
        // Solo starts as well: the stronger factor applies to the original level.
        let both = apply(&mut ledger, "s", first, Some(duck.combine(SOLO))).unwrap();
        assert!((both.volume - 0.16).abs() < 1e-6);
        // Solo ends, ducking continues.
        let back = apply(&mut ledger, "s", both, Some(duck)).unwrap();
        assert!((back.volume - 0.4).abs() < 1e-6);
        assert_eq!(apply(&mut ledger, "s", back, None), Some(level(0.8)));
        assert_eq!(SOLO.combine(MUTE), Reduction { factor: 0.2, mute: true });
    }

    #[test]
    fn sessions_are_independent_and_ended_sessions_are_forgotten() {
        let mut ledger = Ledger::default();
        apply(&mut ledger, "a", level(1.0), Some(SOLO));
        apply(&mut ledger, "b", level(0.5), Some(SOLO));
        ledger.retain(|s| s == "b");
        assert!(!ledger.contains("a") && ledger.contains("b"));
        assert_eq!(apply(&mut ledger, "a", level(0.2), None), None, "a session that ended is not restored later");
    }

    #[test]
    fn a_reduction_that_changes_nothing_still_tracks_the_session() {
        let mut ledger = Ledger::default();
        assert_eq!(apply(&mut ledger, "s", level(0.0), Some(SOLO)), None, "0 % stays 0 %");
        assert!(ledger.contains("s"));
        assert_eq!(apply(&mut ledger, "s", level(0.0), None), None);
        assert!(ledger.is_empty());
    }

    #[test]
    fn a_failed_restore_keeps_the_base_for_a_retry() {
        let mut ledger = Ledger::default();
        let base = level(0.8);
        let lowered = apply(&mut ledger, "s", base, Some(SOLO)).unwrap();
        assert_eq!(ledger.reconcile("s", lowered, None), Some(base));
        assert!(ledger.contains("s"), "planning a restore must not forget the base before Windows accepts it");
        assert_eq!(ledger.reconcile("s", lowered, None), Some(base));
        ledger.record_written("s", base);
        ledger.finish_restore("s");
        assert!(ledger.is_empty());
    }

    #[test]
    fn failed_and_partial_writes_retry_without_overwriting_user_changes() {
        let mut ledger = Ledger::default();
        let base = level(0.8);
        let reduction = SOLO.combine(MUTE);
        let target = ledger.reconcile("s", base, Some(reduction)).unwrap();
        assert_eq!(ledger.reconcile("s", base, Some(reduction)), Some(target), "a failed first write retries");
        let partial = Level { volume: target.volume, muted: false };
        ledger.record_written("s", partial);
        assert_eq!(
            ledger.reconcile("s", partial, Some(reduction)),
            Some(target),
            "a failed mute does not release the session"
        );
        ledger.record_written("s", target);
        assert_eq!(ledger.reconcile("s", target, None), Some(base));
        let partial_restore = Level { volume: base.volume, muted: true };
        ledger.record_written("s", partial_restore);
        assert_eq!(ledger.reconcile("s", partial_restore, None), Some(base), "a partially restored level retries");
        assert_eq!(ledger.reconcile("s", level(0.3), None), None, "an actual external change still wins");
        assert!(ledger.is_empty());
    }
}
