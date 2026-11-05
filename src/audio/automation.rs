//! Opt-in automatic reductions: lowering apps while they aren't in the
//! foreground, and while the microphone picks up speech. Both feed the same
//! [`super::attenuation::Ledger`] as Solo, so a level the user changes is
//! never overwritten.

use std::time::{Duration, Instant};

/// When the microphone counts as active. Speech has to stay above the
/// threshold for `attack` to start ducking and below it for `release` to end
/// it, so short noises and pauses between words don't make levels flutter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MicGateConfig {
    /// Peak level, 0.0..=1.0.
    pub threshold: f32,
    pub attack: Duration,
    pub release: Duration,
}

#[derive(Debug, Default)]
pub struct MicGate {
    active: bool,
    /// Since when the level has been on the other side of the threshold.
    crossing_since: Option<Instant>,
}

impl MicGate {
    pub fn active(&self) -> bool {
        self.active
    }

    /// Feeds one peak reading; returns whether the active state changed.
    pub fn update(&mut self, config: &MicGateConfig, peak: f32, now: Instant) -> bool {
        let wants_active = peak >= config.threshold;
        if wants_active == self.active {
            self.crossing_since = None;
            return false;
        }
        let since = *self.crossing_since.get_or_insert(now);
        let hold = if wants_active { config.attack } else { config.release };
        if now.duration_since(since) >= hold {
            self.active = wants_active;
            self.crossing_since = None;
            return true;
        }
        false
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The foreground app as last reported, applied only once focus has stayed
/// put for `settle`, so alt-tabbing through windows doesn't bounce levels.
#[derive(Debug)]
pub struct FocusSettle<K> {
    applied: Option<K>,
    pending: Option<(K, Instant)>,
}

impl<K> Default for FocusSettle<K> {
    fn default() -> Self {
        Self { applied: None, pending: None }
    }
}

impl<K: Clone + PartialEq> FocusSettle<K> {
    pub fn applied(&self) -> Option<&K> {
        self.applied.as_ref()
    }

    pub fn report(&mut self, key: K, now: Instant) {
        if self.applied.as_ref() == Some(&key) {
            self.pending = None;
        } else if self.pending.as_ref().is_none_or(|(pending, _)| *pending != key) {
            self.pending = Some((key, now));
        }
    }

    /// When the pending report becomes effective.
    pub fn due(&self, settle: Duration) -> Option<Instant> {
        self.pending.as_ref().map(|(_, at)| *at + settle)
    }

    /// Applies the pending report if it has settled; returns whether the
    /// applied foreground changed.
    pub fn settle(&mut self, settle: Duration, now: Instant) -> bool {
        match &self.pending {
            Some((key, at)) if now.duration_since(*at) >= settle => {
                self.applied = Some(key.clone());
                self.pending = None;
                true
            }
            _ => false,
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GATE: MicGateConfig =
        MicGateConfig { threshold: 0.1, attack: Duration::from_millis(100), release: Duration::from_millis(500) };

    fn ms(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn speech_must_last_the_attack_time_to_start_ducking() {
        let (t0, mut gate) = (Instant::now(), MicGate::default());
        assert!(!gate.update(&GATE, 0.5, ms(t0, 0)));
        assert!(!gate.update(&GATE, 0.5, ms(t0, 60)));
        assert!(!gate.update(&GATE, 0.02, ms(t0, 80)), "a short click doesn't count");
        assert!(!gate.update(&GATE, 0.5, ms(t0, 100)), "the attack time starts over");
        assert!(gate.update(&GATE, 0.5, ms(t0, 200)));
        assert!(gate.active());
    }

    #[test]
    fn pauses_shorter_than_the_release_time_keep_ducking() {
        let (t0, mut gate) = (Instant::now(), MicGate::default());
        gate.update(&GATE, 0.5, ms(t0, 0));
        gate.update(&GATE, 0.5, ms(t0, 100));
        assert!(gate.active());
        assert!(!gate.update(&GATE, 0.0, ms(t0, 200)));
        assert!(!gate.update(&GATE, 0.0, ms(t0, 600)), "a pause between words");
        assert!(!gate.update(&GATE, 0.3, ms(t0, 650)), "speech resumes; release starts over");
        assert!(!gate.update(&GATE, 0.0, ms(t0, 700)));
        assert!(gate.update(&GATE, 0.0, ms(t0, 1200)), "silence for the release time ends ducking");
        assert!(!gate.active());
        gate.update(&GATE, 0.5, ms(t0, 1300));
        gate.reset();
        assert!(!gate.active());
    }

    #[test]
    fn focus_changes_apply_only_after_settling() {
        let settle = Duration::from_millis(250);
        let (t0, mut focus) = (Instant::now(), FocusSettle::default());
        focus.report("game", ms(t0, 0));
        assert!(!focus.settle(settle, ms(t0, 100)));
        assert_eq!(focus.due(settle), Some(ms(t0, 250)));
        assert!(focus.settle(settle, ms(t0, 250)));
        assert_eq!(focus.applied(), Some(&"game"));

        // Alt-tabbing through two windows and back changes nothing.
        focus.report("browser", ms(t0, 300));
        focus.report("chat", ms(t0, 350));
        focus.report("game", ms(t0, 400));
        assert_eq!(focus.due(settle), None, "back where it was: nothing pending");
        assert!(!focus.settle(settle, ms(t0, 1000)));
        assert_eq!(focus.applied(), Some(&"game"));

        // A repeated report doesn't push the deadline out.
        focus.report("browser", ms(t0, 1000));
        focus.report("browser", ms(t0, 1200));
        assert!(focus.settle(settle, ms(t0, 1250)));
        assert_eq!(focus.applied(), Some(&"browser"));
        focus.clear();
        assert_eq!(focus.applied(), None);
    }
}
