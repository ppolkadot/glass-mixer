//! Explicit user-owned app metadata and presets. No live Windows state cache.
use super::{Activity, AppEntry, AppKey};
use serde::{Deserialize, Deserializer, Serialize};
use std::{collections::HashSet, path::PathBuf};

pub fn normalized_key(key: &AppKey) -> Option<AppKey> {
    match key {
        AppKey::Exe(path) if !path.trim().is_empty() => Some(AppKey::Exe(path.to_lowercase())),
        AppKey::Package(id) if !id.trim().is_empty() => Some(AppKey::Package(id.to_lowercase())),
        AppKey::System => Some(AppKey::System),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PinnedApp {
    pub key: AppKey,
    pub name: String,
    pub exe_path: Option<PathBuf>,
    pub aumid: Option<String>,
}

impl PinnedApp {
    pub fn from_app(app: &AppEntry) -> Option<Self> {
        let key = normalized_key(&app.key)?;
        if key == AppKey::System {
            return None;
        }
        Some(Self { key, name: app.name.clone(), exe_path: app.exe_path.clone(), aumid: app.aumid.clone() })
    }

    pub fn placeholder(&self) -> AppEntry {
        AppEntry {
            key: self.key.clone(),
            name: self.name.clone(),
            exe_path: self.exe_path.clone(),
            aumid: self.aumid.clone(),
            icon_resource: None,
            volume: 0.0,
            muted: false,
            activity: Activity::Idle,
            session_count: 0,
            balance: None,
        }
    }
}

pub fn normalize_pins(pins: Vec<PinnedApp>) -> Vec<PinnedApp> {
    let mut seen = HashSet::new();
    pins.into_iter()
        .filter_map(|mut pin| {
            pin.key = normalized_key(&pin.key)?;
            if pin.key == AppKey::System || !seen.insert(pin.key.clone()) {
                return None;
            }
            match &pin.key {
                AppKey::Exe(path) => {
                    pin.exe_path = Some(PathBuf::from(path));
                    pin.aumid = None;
                }
                AppKey::Package(id) if pin.aumid.is_none() => {
                    pin.aumid = Some(id.clone());
                }
                _ => {}
            }
            pin.name = pin.name.chars().take(160).collect();
            if pin.name.trim().is_empty() {
                pin.name = "Pinned app".into();
            }
            Some(pin)
        })
        .take(128)
        .collect()
}

/// System sounds, pins in saved order, then unpinned live rows in engine order.
/// A zero session_count is only a UI placeholder and must never expose a value.
pub fn rows_with_pins(apps: &[AppEntry], pins: &[PinnedApp]) -> Vec<AppEntry> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    if let Some(system) = apps.iter().find(|a| a.key == AppKey::System) {
        seen.insert(system.key.clone());
        rows.push(system.clone());
    }
    for pin in pins {
        if seen.insert(pin.key.clone()) {
            rows.push(
                apps.iter()
                    .find(|a| normalized_key(&a.key).as_ref() == Some(&pin.key))
                    .cloned()
                    .unwrap_or_else(|| pin.placeholder()),
            );
        }
    }
    for app in apps {
        if seen.insert(normalized_key(&app.key).unwrap_or_else(|| app.key.clone())) {
            rows.push(app.clone());
        }
    }
    rows
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresetEntry {
    pub key: AppKey,
    pub name: String,
    pub volume: f32,
    pub muted: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VolumePreset {
    pub name: String,
    #[serde(default, deserialize_with = "read_entries")]
    pub entries: Vec<PresetEntry>,
    #[serde(default, deserialize_with = "read_auto_rule")]
    pub apply_on_start: bool,
}

fn read_entries<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<PresetEntry>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .take(128)
        .filter_map(|item| serde_json::from_value(item.clone()).ok())
        .collect())
}

fn read_auto_rule<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(value.as_bool().unwrap_or(false))
}

impl VolumePreset {
    pub fn capture(name: &str, apps: &[AppEntry]) -> Self {
        Self {
            name: name.trim().chars().take(80).collect(),
            apply_on_start: false,
            entries: apps
                .iter()
                .filter_map(|a| {
                    Some(PresetEntry {
                        key: normalized_key(&a.key)?,
                        name: a.name.clone(),
                        volume: a.volume,
                        muted: a.muted,
                    })
                })
                .collect(),
        }
    }
    pub fn sanitize(mut self) -> Option<Self> {
        self.name = self.name.trim().chars().take(80).collect();
        if self.name.is_empty() {
            return None;
        }
        let mut seen = HashSet::new();
        self.entries = self
            .entries
            .into_iter()
            .filter_map(|mut e| {
                e.key = normalized_key(&e.key)?;
                if !e.volume.is_finite() || !(0.0..=1.0).contains(&e.volume) || !seen.insert(e.key.clone()) {
                    return None;
                }
                e.name = e.name.chars().take(160).collect();
                Some(e)
            })
            .take(128)
            .collect();
        Some(self)
    }
}

/// Automatic rules are deliberately conservative: only a new process after
/// arming, a first observation and no observed user volume/mute change qualify.
pub fn may_apply_at_start(created: Option<u64>, armed: u64, last_seen: Option<u64>, untouched: bool) -> bool {
    untouched && created.is_some_and(|t| t > armed && last_seen.is_none_or(|last| t > last))
}

/// Remembers which process instance of each app has already been considered, so an
/// automatic rule applies at most once per new process.
#[derive(Debug, Default)]
pub struct StartTracker {
    armed_at: u64,
    last_started: std::collections::HashMap<AppKey, u64>,
}

impl StartTracker {
    pub fn new(armed_at: u64) -> Self {
        Self { armed_at, last_started: Default::default() }
    }

    /// Starts over: only processes created after `armed_at` qualify from now on.
    pub fn rearm(&mut self, armed_at: u64) {
        self.armed_at = armed_at;
        self.last_started.clear();
    }

    /// The user (or a preset) set this process's levels, so it never qualifies afterwards.
    pub fn note_intervention(&mut self, key: &AppKey, created: u64) {
        let last = self.last_started.entry(key.clone()).or_insert(created);
        *last = (*last).max(created);
    }

    /// Whether the rule may apply to the process of `key` created at `created`.
    /// Records the process as seen either way. Call in ascending `created` order
    /// when several processes of one app appear at once.
    pub fn observe(&mut self, key: &AppKey, created: u64, untouched: bool) -> bool {
        let previous = self.last_started.get(key).copied();
        if previous.is_none_or(|last| created > last) {
            self.last_started.insert(key.clone(), created);
        }
        may_apply_at_start(Some(created), self.armed_at, previous, untouched)
    }
}

/// Which saved entries a preset can apply to right now.
#[derive(Debug, PartialEq)]
pub struct PresetPlan<'a> {
    pub present: Vec<&'a PresetEntry>,
    /// Entries for apps that have no live session.
    pub missing: usize,
}

pub fn plan_preset<'a>(preset: &'a VolumePreset, is_live: impl Fn(&AppKey) -> bool) -> PresetPlan<'a> {
    let (present, absent): (Vec<_>, Vec<_>) = preset.entries.iter().partition(|entry| is_live(&entry.key));
    PresetPlan { present, missing: absent.len() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_pins_collapse_and_missing_apps_have_no_sessions() {
        let pin =
            PinnedApp { key: AppKey::Exe("C:\\Apps\\A.exe".into()), name: "A".into(), exe_path: None, aumid: None };
        let pins = normalize_pins(vec![pin.clone(), pin]);
        assert_eq!(pins.len(), 1);
        let rows = rows_with_pins(&[], &pins);
        assert_eq!(rows[0].session_count, 0);
        let json = serde_json::to_string(&pins).unwrap();
        assert!(!json.contains("volume") && !json.contains("muted"));
        let mut live = pins[0].placeholder();
        live.session_count = 1;
        live.volume = 0.42;
        assert_eq!(rows_with_pins(&[live.clone()], &pins), vec![live]);
    }
    #[test]
    fn presets_are_explicit_and_invalid_values_do_not_apply() {
        let preset = VolumePreset {
            name: " Test ".into(),
            apply_on_start: false,
            entries: vec![
                PresetEntry { key: AppKey::Pid(2), name: "x".into(), volume: 0.5, muted: false },
                PresetEntry { key: AppKey::System, name: "sys".into(), volume: 2.0, muted: false },
            ],
        }
        .sanitize()
        .unwrap();
        assert!(preset.entries.is_empty());
        assert!(!preset.apply_on_start);
        assert_eq!(preset.name, "Test");
    }
    #[test]
    fn automatic_rules_never_reapply_to_existing_or_modified_processes() {
        assert!(may_apply_at_start(Some(101), 100, None, true));
        assert!(!may_apply_at_start(Some(99), 100, None, true));
        assert!(!may_apply_at_start(Some(101), 100, Some(101), true));
        assert!(!may_apply_at_start(Some(101), 100, None, false));
        assert!(!may_apply_at_start(None, 100, None, true));
    }

    fn exe(path: &str) -> AppKey {
        AppKey::Exe(path.into())
    }

    #[test]
    fn a_new_process_is_considered_exactly_once() {
        let mut tracker = StartTracker::new(100);
        let app = exe(r"c:\apps\player.exe");
        assert!(tracker.observe(&app, 150, true), "first sighting of a process started after arming");
        assert!(!tracker.observe(&app, 150, true), "the same process is never applied twice");
        assert!(tracker.observe(&app, 200, true), "a later launch of the same app is a new process");
        assert!(!tracker.observe(&app, 120, true), "an older process than one already seen is skipped");
    }

    #[test]
    fn processes_that_predate_arming_or_were_touched_never_qualify() {
        let mut tracker = StartTracker::new(100);
        let app = exe(r"c:\apps\player.exe");
        assert!(!tracker.observe(&app, 90, true), "already running when the rule was armed");
        assert!(!tracker.observe(&app, 160, false), "its levels were changed before the first look");
        assert!(!tracker.observe(&app, 160, true), "and a touched process stays skipped");
    }

    #[test]
    fn apps_are_tracked_independently() {
        let mut tracker = StartTracker::new(100);
        let (a, b) = (exe("a.exe"), AppKey::Package("pkg!app".into()));
        assert!(tracker.observe(&a, 150, true));
        assert!(tracker.observe(&b, 150, true), "another app with the same start time is not affected");
    }

    #[test]
    fn an_intervention_blocks_the_rule_for_that_process_only() {
        let mut tracker = StartTracker::new(100);
        let app = exe("a.exe");
        tracker.note_intervention(&app, 150);
        assert!(!tracker.observe(&app, 150, true), "a preset or user change was already made to this process");
        assert!(tracker.observe(&app, 160, true), "the next launch is a new process");
    }

    #[test]
    fn rearming_forgets_history_and_ignores_older_processes() {
        let mut tracker = StartTracker::new(100);
        let app = exe("a.exe");
        assert!(tracker.observe(&app, 150, true));
        tracker.rearm(300);
        assert!(!tracker.observe(&app, 250, true), "started before the new arming time");
        assert!(tracker.observe(&app, 350, true), "started after it, history cleared");
    }

    #[test]
    fn launch_order_decides_which_new_process_is_seen_first() {
        // The engine sorts same-pass processes oldest first. Judged in that order both qualify...
        let mut ordered = StartTracker::new(100);
        let app = exe("a.exe");
        assert!(ordered.observe(&app, 150, true));
        assert!(ordered.observe(&app, 160, true));
        // ...but judging the newer one first would hide the older one, which is why order matters.
        let mut reversed = StartTracker::new(100);
        assert!(reversed.observe(&app, 160, true));
        assert!(!reversed.observe(&app, 150, true));
    }

    #[test]
    fn a_preset_plan_separates_running_from_missing_apps() {
        let entry = |key: AppKey, name: &str| PresetEntry { key, name: name.into(), volume: 0.5, muted: false };
        let preset = VolumePreset {
            name: "Mix".into(),
            apply_on_start: false,
            entries: vec![
                entry(AppKey::System, "System sounds"),
                entry(exe("running.exe"), "Running"),
                entry(exe("gone.exe"), "Gone"),
            ],
        };
        let live = [AppKey::System, exe("running.exe")];
        let plan = plan_preset(&preset, |key| live.contains(key));
        assert_eq!(plan.present.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["System sounds", "Running"]);
        assert_eq!(plan.missing, 1);
        assert_eq!(plan_preset(&preset, |_| false).missing, 3);
        assert!(plan_preset(&preset, |_| false).present.is_empty());
    }
}
