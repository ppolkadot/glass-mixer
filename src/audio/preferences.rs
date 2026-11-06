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
            balance: crate::audio::BalanceState::Unavailable,
            streams: Vec::new(),
            route: crate::audio::Route::SystemDefault,
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

/// An output device the user pinned for cycling. The name is shown while the
/// device is disconnected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedDevice {
    pub id: String,
    pub name: String,
}

/// Drops empty and duplicate IDs (case-insensitive, as Windows treats them) and caps the list.
pub fn normalize_pinned_devices(pins: Vec<PinnedDevice>) -> Vec<PinnedDevice> {
    let mut seen = HashSet::new();
    pins.into_iter()
        .filter(|pin| !pin.id.trim().is_empty() && seen.insert(pin.id.to_lowercase()))
        .map(|mut pin| {
            pin.name = pin.name.chars().take(160).collect();
            pin
        })
        .take(32)
        .collect()
}

/// A name the user gave an app, shown instead of the one Windows reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppName {
    pub key: AppKey,
    pub name: String,
}

const MAX_APP_NAME: usize = 60;
const MAX_APP_NAMES: usize = 128;

/// Keeps one non-empty name per stable app identity (transient apps have none).
pub fn normalize_app_names(names: Vec<AppName>) -> Vec<AppName> {
    let mut seen = HashSet::new();
    names
        .into_iter()
        .filter_map(|mut entry| {
            entry.key = normalized_key(&entry.key)?;
            entry.name = entry.name.trim().chars().take(MAX_APP_NAME).collect();
            (!entry.name.is_empty() && seen.insert(entry.key.clone())).then_some(entry)
        })
        .take(MAX_APP_NAMES)
        .collect()
}

/// The name the user gave `key`, if any.
pub fn custom_name<'a>(names: &'a [AppName], key: &AppKey) -> Option<&'a str> {
    let key = normalized_key(key)?;
    names.iter().find(|n| n.key == key).map(|n| n.name.as_str())
}

/// Sets the name of an app; an empty name removes it. Returns whether anything changed.
pub fn set_custom_name(names: &mut Vec<AppName>, key: &AppKey, name: &str) -> bool {
    let Some(key) = normalized_key(key) else { return false };
    let name: String = name.trim().chars().take(MAX_APP_NAME).collect();
    match names.iter().position(|n| n.key == key) {
        Some(i) if name.is_empty() => {
            names.remove(i);
            true
        }
        Some(i) if names[i].name != name => {
            names[i].name = name;
            true
        }
        Some(_) => false,
        None if name.is_empty() || names.len() >= MAX_APP_NAMES => false,
        None => {
            names.push(AppName { key, name });
            true
        }
    }
}

/// Swaps `items[index]` with its neighbour: `delta` is -1 (earlier) or 1 (later).
/// Returns whether it moved; the ends of the list stay put.
pub fn move_item<T>(items: &mut [T], index: usize, delta: isize) -> bool {
    if delta.abs() != 1 || index >= items.len() {
        return false;
    }
    match index.checked_add_signed(delta).filter(|&target| target < items.len()) {
        Some(target) => {
            items.swap(index, target);
            true
        }
        None => false,
    }
}

/// What cycling the default output through the pinned devices does.
#[derive(Debug, PartialEq)]
pub enum CycleOutcome<'a> {
    /// Make this device the default.
    Switch(&'a super::DeviceInfo),
    /// The only connected pinned device is already the default.
    AlreadyDefault(&'a super::DeviceInfo),
    NothingPinned,
    /// Devices are pinned, but none of them is connected.
    NoneConnected,
}

/// The next connected pinned output after the current default, in pin order.
/// When the default isn't pinned, the first connected pinned device is next.
pub fn next_pinned_output<'a>(pinned: &[String], outputs: &'a [super::DeviceInfo]) -> CycleOutcome<'a> {
    if pinned.is_empty() {
        return CycleOutcome::NothingPinned;
    }
    let connected: Vec<&super::DeviceInfo> =
        pinned.iter().filter_map(|id| outputs.iter().find(|d| d.id.eq_ignore_ascii_case(id))).collect();
    let Some(first) = connected.first() else {
        return CycleOutcome::NoneConnected;
    };
    match connected.iter().position(|d| d.is_default) {
        Some(i) if connected.len() == 1 => CycleOutcome::AlreadyDefault(connected[i]),
        Some(i) => CycleOutcome::Switch(connected[(i + 1) % connected.len()]),
        None => CycleOutcome::Switch(first),
    }
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
    /// The output device to set for the app; `None` leaves its device alone.
    #[serde(default, deserialize_with = "read_optional", skip_serializing_if = "Option::is_none")]
    pub output: Option<DeviceChoice>,
}

/// The master output level a profile sets.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MasterLevel {
    pub volume: f32,
    pub muted: bool,
}

/// A default device a profile selects. The name is shown while it is disconnected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceChoice {
    pub id: String,
    pub name: String,
}

/// A saved profile (called a preset before 0.5): app levels, and optionally the
/// master level and default devices. Only explicit choices are stored, never
/// live state. Older files have only the first three fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VolumePreset {
    pub name: String,
    #[serde(default, deserialize_with = "read_entries")]
    pub entries: Vec<PresetEntry>,
    #[serde(default, deserialize_with = "read_auto_rule")]
    pub apply_on_start: bool,
    #[serde(default, deserialize_with = "read_optional", skip_serializing_if = "Option::is_none")]
    pub master: Option<MasterLevel>,
    #[serde(default, deserialize_with = "read_optional", skip_serializing_if = "Option::is_none")]
    pub output_device: Option<DeviceChoice>,
    #[serde(default, deserialize_with = "read_optional", skip_serializing_if = "Option::is_none")]
    pub input_device: Option<DeviceChoice>,
}

/// Which parts of the current state a new profile captures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureParts {
    pub apps: bool,
    /// Each saved app's own output device, when it isn't the system default.
    pub app_devices: bool,
    pub master: bool,
    pub output_device: bool,
    pub input_device: bool,
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

/// An invalid optional part is dropped instead of discarding the whole profile.
fn read_optional<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).ok())
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
                        output: None,
                    })
                })
                .collect(),
            ..Default::default()
        }
    }
    /// A profile from the current state; `parts` chooses what it includes.
    pub fn capture_profile(name: &str, snapshot: &super::AudioSnapshot, parts: CaptureParts) -> Self {
        let device = |flow: super::Flow| {
            snapshot.endpoint(flow).map(|ep| DeviceChoice { id: ep.id.clone(), name: ep.name.clone() })
        };
        let mut profile = if parts.apps { Self::capture(name, &snapshot.apps) } else { Self::capture(name, &[]) };
        if parts.apps && parts.app_devices {
            for entry in &mut profile.entries {
                let route = snapshot.apps.iter().find(|a| normalized_key(&a.key).as_ref() == Some(&entry.key));
                if let Some(super::Route::Device(id)) = route.map(|a| &a.route) {
                    let name = snapshot.outputs.iter().find(|d| &d.id == id).map_or("Saved device", |d| &d.name);
                    entry.output = Some(DeviceChoice { id: id.clone(), name: name.to_owned() });
                }
            }
        }
        if parts.master {
            profile.master = snapshot.output.as_ref().map(|ep| MasterLevel { volume: ep.volume, muted: ep.muted });
        }
        if parts.output_device {
            profile.output_device = device(super::Flow::Output);
        }
        if parts.input_device {
            profile.input_device = device(super::Flow::Input);
        }
        profile
    }

    /// Whether applying it would change anything.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.master.is_none() && self.output_device.is_none() && self.input_device.is_none()
    }

    pub fn sanitize(mut self) -> Option<Self> {
        self.name = self.name.trim().chars().take(80).collect();
        if self.name.is_empty() {
            return None;
        }
        self.master = self.master.filter(|m| m.volume.is_finite() && (0.0..=1.0).contains(&m.volume));
        let valid_device = |d: &DeviceChoice| !d.id.trim().is_empty();
        self.output_device = self.output_device.filter(valid_device);
        self.input_device = self.input_device.filter(valid_device);
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

const MAX_TRACKED_PROCESSES: usize = 1024;

/// An automatic rule applies only to a process started after arming, on its
/// first observation, with no observed volume or mute change.
pub fn may_apply_at_start(created: Option<u64>, armed: u64, already_seen: bool, untouched: bool) -> bool {
    untouched && !already_seen && created.is_some_and(|t| t > armed)
}

/// Identifies a single OS process instance of an application.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProcessInstance {
    pub key: AppKey,
    pub pid: u32,
    pub created: u64,
}

/// Remembers which process instances were already considered or adjusted, so an
/// automatic rule applies at most once per process instance.
#[derive(Debug, Default)]
pub struct StartTracker {
    armed_at: u64,
    seen: std::collections::HashSet<ProcessInstance>,
    /// Set once more than `MAX_TRACKED_PROCESSES` live instances are tracked;
    /// unseen instances are then skipped. Instances skipped meanwhile were never
    /// recorded, so only [`StartTracker::reclaim`] (which also moves the arming
    /// time past them) or [`StartTracker::rearm`] may clear it.
    saturated: bool,
}

impl StartTracker {
    pub fn new(armed_at: u64) -> Self {
        Self { armed_at, seen: Default::default(), saturated: false }
    }

    /// Starts over: only processes created after `armed_at` qualify from now on.
    pub fn rearm(&mut self, armed_at: u64) {
        self.armed_at = armed_at;
        self.seen.clear();
        self.saturated = false;
    }

    fn record(&mut self, instance: ProcessInstance) -> bool {
        if instance.pid != 0 {
            self.seen.retain(|existing| !(existing.pid == instance.pid && existing.created != instance.created));
        }
        if self.seen.contains(&instance) {
            return true;
        }
        if self.saturated || self.seen.len() >= MAX_TRACKED_PROCESSES {
            self.saturated = true;
            return true;
        }
        self.seen.insert(instance);
        false
    }

    /// Removes markers for a process that has exited, so retained state stays bounded.
    pub fn forget_exited_pid(&mut self, pid: u32) {
        if pid != 0 {
            self.seen.retain(|existing| existing.pid != pid);
        }
    }

    /// Whether enough markers are held that [`StartTracker::reclaim`] is worth its cost.
    pub fn wants_reclaim(&self) -> bool {
        self.saturated || self.seen.len() >= MAX_TRACKED_PROCESSES / 2
    }

    /// Drops markers whose process instance is no longer running, including
    /// processes whose exit was never observed. `is_running(pid, created)` must
    /// answer `true` when it can't tell, so a live instance is never forgotten.
    ///
    /// If this frees capacity while saturated, the latch is cleared and the
    /// arming time moves to `now`: instances skipped while saturated started
    /// before `now`, so none of them can qualify later.
    pub fn reclaim(&mut self, now: u64, mut is_running: impl FnMut(u32, u64) -> bool) {
        self.seen.retain(|i| i.pid == 0 || is_running(i.pid, i.created));
        if self.saturated && self.seen.len() < MAX_TRACKED_PROCESSES {
            self.saturated = false;
            self.armed_at = self.armed_at.max(now);
        }
    }

    /// The user (or a preset) set this process instance's levels, so it never qualifies afterwards.
    pub fn note_process_intervention(&mut self, key: &AppKey, pid: u32, created: u64) {
        if created > self.armed_at {
            self.record(ProcessInstance { key: key.clone(), pid, created });
        }
    }

    /// Whether the rule may apply to the process instance `(key, pid, created)`.
    /// Records the process instance as seen if it started after arming.
    pub fn observe_process(&mut self, key: &AppKey, pid: u32, created: u64, untouched: bool) -> bool {
        if created <= self.armed_at {
            if pid != 0 {
                self.seen.retain(|existing| !(existing.pid == pid && existing.created != created));
            }
            return false;
        }
        let already_seen = self.record(ProcessInstance { key: key.clone(), pid, created });
        may_apply_at_start(Some(created), self.armed_at, already_seen, untouched)
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
                PresetEntry { key: AppKey::Pid(2), name: "x".into(), volume: 0.5, muted: false, output: None },
                PresetEntry { key: AppKey::System, name: "sys".into(), volume: 2.0, muted: false, output: None },
            ],
            ..Default::default()
        }
        .sanitize()
        .unwrap();
        assert!(preset.entries.is_empty());
        assert!(!preset.apply_on_start);
        assert_eq!(preset.name, "Test");
    }
    #[test]
    fn automatic_rules_never_reapply_to_existing_or_modified_processes() {
        assert!(may_apply_at_start(Some(101), 100, false, true));
        assert!(!may_apply_at_start(Some(99), 100, false, true));
        assert!(!may_apply_at_start(Some(101), 100, true, true));
        assert!(!may_apply_at_start(Some(101), 100, false, false));
        assert!(!may_apply_at_start(None, 100, false, true));
    }

    fn exe(path: &str) -> AppKey {
        AppKey::Exe(path.into())
    }

    #[test]
    fn a_new_process_is_considered_exactly_once() {
        let mut tracker = StartTracker::new(100);
        let app = exe(r"c:\apps\player.exe");
        assert!(tracker.observe_process(&app, 10, 150, true), "first sighting of a process started after arming");
        assert!(
            !tracker.observe_process(&app, 10, 150, true),
            "the same process is never applied twice even if it opens another session"
        );
        assert!(tracker.observe_process(&app, 20, 200, true), "a later launch of the same app is a new process");
        assert!(
            tracker.observe_process(&app, 30, 120, true),
            "an older process started after arming whose first audio session arrives later still qualifies"
        );
        assert!(!tracker.observe_process(&app, 30, 120, true), "and that older process is also considered only once");
    }

    #[test]
    fn processes_that_predate_arming_or_were_touched_never_qualify() {
        let mut tracker = StartTracker::new(100);
        let app = exe(r"c:\apps\player.exe");
        assert!(!tracker.observe_process(&app, 10, 90, true), "already running when the rule was armed");
        assert!(!tracker.observe_process(&app, 20, 160, false), "its levels were changed before the first look");
        assert!(!tracker.observe_process(&app, 20, 160, true), "and a touched process stays skipped");
    }

    #[test]
    fn apps_are_tracked_independently() {
        let mut tracker = StartTracker::new(100);
        let (a, b) = (exe("a.exe"), AppKey::Package("pkg!app".into()));
        assert!(tracker.observe_process(&a, 10, 150, true));
        assert!(tracker.observe_process(&b, 20, 150, true), "another app with the same start time is not affected");
    }

    #[test]
    fn an_intervention_blocks_the_rule_for_that_process_only() {
        let mut tracker = StartTracker::new(100);
        let app = exe("a.exe");
        tracker.note_process_intervention(&app, 20, 160);
        assert!(
            !tracker.observe_process(&app, 20, 160, true),
            "a preset or user change was already made to this process"
        );
        assert!(
            tracker.observe_process(&app, 10, 150, true),
            "an older sibling process that had not produced audio yet is not suppressed"
        );
        assert!(
            tracker.observe_process(&app, 30, 160, true),
            "a sibling process sharing the same timestamp with a different PID is not suppressed"
        );
    }

    #[test]
    fn rearming_forgets_history_and_ignores_older_processes() {
        let mut tracker = StartTracker::new(100);
        let app = exe("a.exe");
        assert!(tracker.observe_process(&app, 10, 150, true));
        tracker.rearm(300);
        assert!(!tracker.observe_process(&app, 10, 250, true), "started before the new arming time");
        assert!(tracker.observe_process(&app, 10, 350, true), "started after it, history cleared");
    }

    #[test]
    fn newer_and_older_processes_qualify_across_passes_and_handle_pid_reuse_and_equal_timestamps() {
        let app = exe("a.exe");
        // Newer-first across separate refreshes: B (160) produces audio before A (150).
        let mut newer_first = StartTracker::new(100);
        assert!(newer_first.observe_process(&app, 20, 160, true));
        assert!(newer_first.observe_process(&app, 10, 150, true));
        assert!(!newer_first.observe_process(&app, 20, 160, true));
        assert!(!newer_first.observe_process(&app, 10, 150, true));

        // Equal timestamps with different PIDs both qualify once.
        let mut same_time = StartTracker::new(100);
        assert!(same_time.observe_process(&app, 10, 150, true));
        assert!(same_time.observe_process(&app, 20, 150, true));
        assert!(!same_time.observe_process(&app, 10, 150, true));

        // PID reuse: PID 10 at t=150 exits and OS reuses PID 10 at t=220.
        assert!(same_time.observe_process(&app, 10, 220, true));
        assert!(!same_time.observe_process(&app, 10, 220, true));

        // Lifecycle eviction when a watched process exits.
        same_time.forget_exited_pid(20);
        assert!(!same_time.seen.iter().any(|i| i.pid == 20));
    }

    #[test]
    fn exceeding_capacity_never_evicts_live_markers_or_re_qualifies_running_processes() {
        let app = exe("a.exe");
        let mut tracker = StartTracker::new(100);

        // PID 1 starts at 101 with touched session (untouched = false).
        assert!(!tracker.observe_process(&app, 1, 101, false));
        // PID 2 is intervened on before audio observation.
        tracker.note_process_intervention(&app, 2, 102);
        // PID 3 qualifies on its initial session (e.g. applied or failed once).
        assert!(tracker.observe_process(&app, 3, 103, true));

        // Reclaiming confirmed exited processes before saturation keeps capacity open.
        assert!(tracker.observe_process(&app, 5000, 104, true));
        tracker.forget_exited_pid(5000);

        // Observe 1,024 additional distinct live process instances without exit reclamation.
        for offset in 0..(MAX_TRACKED_PROCESSES as u32) {
            let pid = 10 + offset;
            let created = 200 + u64::from(offset);
            let _ = tracker.observe_process(&app, pid, created, true);
        }

        // Revisiting the oldest live processes (touched, intervened, or recreated session)
        // must never re-qualify them.
        assert!(
            !tracker.observe_process(&app, 1, 101, true),
            "oldest live touched process must remain ineligible after capacity saturation"
        );
        assert!(
            !tracker.observe_process(&app, 2, 102, true),
            "oldest live intervened process must remain ineligible after capacity saturation"
        );
        assert!(
            !tracker.observe_process(&app, 3, 103, true),
            "oldest live process with recreated session must remain ineligible after capacity saturation"
        );
        // While saturated, unseen processes are also conservatively skipped.
        assert!(
            !tracker.observe_process(&app, 9999, 9999, true),
            "saturated tracker must conservatively refuse unseen processes rather than evicting live markers"
        );
    }

    #[test]
    fn saturation_stays_latched_after_exits_free_slots_until_rearm() {
        let app = exe("a.exe");
        let mut tracker = StartTracker::new(100);
        for offset in 0..(MAX_TRACKED_PROCESSES as u32) {
            assert!(tracker.observe_process(&app, 10 + offset, 200 + u64::from(offset), true));
        }
        // The next unseen instance exceeds capacity and latches saturation.
        assert!(!tracker.observe_process(&app, 9999, 5000, true), "the process that exceeds capacity is skipped");
        assert!(tracker.saturated);

        // Exits free slots, but the latch stays until rearming.
        for offset in 0..64 {
            tracker.forget_exited_pid(10 + offset);
        }
        assert!(tracker.saturated, "saturation is latched until rearming, not cleared by exit reclamation");
        assert!(!tracker.observe_process(&app, 8888, 6000, true), "new instances stay skipped while latched");

        tracker.rearm(7000);
        assert!(!tracker.saturated);
        assert!(tracker.observe_process(&app, 8888, 7001, true), "rearming clears the latch");
    }

    #[test]
    fn long_runs_of_launches_with_unobserved_exits_never_saturate() {
        let (app, helper) = (exe("player.exe"), exe("helper.exe"));
        let mut tracker = StartTracker::new(100);
        let mut live: HashSet<(u32, u64)> = HashSet::new();
        let mut applied = 0;
        for n in 0..20_000u32 {
            // PIDs cycle through a small range, so they are reused constantly.
            let (pid, created) = (4 + n % 700 * 4, 200 + u64::from(n));
            let key = if n % 3 == 0 { &helper } else { &app };
            if tracker.observe_process(key, pid, created, true) {
                applied += 1;
            }
            assert!(!tracker.observe_process(key, pid, created, true), "launch {n} considered twice");
            live.insert((pid, created));
            // Each process lives for a while, then exits without an exit notification.
            live.retain(|&(_, c)| c + 300 > created);
            if n % 50 == 0 && tracker.wants_reclaim() {
                tracker.reclaim(created, |pid, c| live.contains(&(pid, c)));
            }
            assert!(!tracker.saturated, "saturated after {n} launches");
        }
        assert_eq!(applied, 20_000, "every launch qualifies exactly once");
        assert!(tracker.seen.len() <= MAX_TRACKED_PROCESSES);
    }

    #[test]
    fn reclaiming_after_saturation_resumes_rules_without_requalifying_skipped_or_live_processes() {
        let app = exe("a.exe");
        let mut tracker = StartTracker::new(100);
        for offset in 0..(MAX_TRACKED_PROCESSES as u32) {
            assert!(tracker.observe_process(&app, 10 + offset, 200 + u64::from(offset), true));
        }
        assert!(!tracker.observe_process(&app, 9000, 5000, true), "skipped while saturated, so never recorded");
        assert!(tracker.saturated);

        // A reclaim that finds every process alive changes nothing.
        tracker.reclaim(5500, |_, _| true);
        assert!(tracker.saturated);
        assert_eq!(tracker.armed_at, 100);

        // 600 processes exited unnoticed; the skipped one still runs.
        let exited = |pid: u32| (10..610).contains(&pid);
        tracker.reclaim(6000, |pid, _| !exited(pid));
        assert!(!tracker.saturated, "freed capacity clears the latch");
        assert!(
            !tracker.observe_process(&app, 9000, 5000, true),
            "the process skipped while saturated must not qualify when its session appears later"
        );
        assert!(!tracker.observe_process(&app, 900, 1090, true), "a live tracked process stays ineligible");
        assert!(tracker.observe_process(&app, 9001, 6001, true), "a launch after recovery qualifies");
        assert!(!tracker.observe_process(&app, 9001, 6001, true));
    }

    #[test]
    fn reclaim_keeps_unknown_processes_and_handles_pid_reuse() {
        let app = exe("a.exe");
        let mut tracker = StartTracker::new(100);
        assert!(tracker.observe_process(&app, 10, 150, true));
        assert!(tracker.observe_process(&app, 20, 160, true));
        // Pid 10 is unknown (for example access denied), so it is reported as running.
        tracker.reclaim(170, |pid, _| pid == 10);
        assert!(!tracker.observe_process(&app, 10, 150, true), "an unknown process keeps its marker");
        assert!(tracker.seen.iter().all(|i| i.pid != 20));
        // Pid 20 was reused by a later process, which is a new instance.
        assert!(tracker.observe_process(&app, 20, 400, true));
        assert!(!tracker.observe_process(&app, 20, 400, true));
        assert_eq!(tracker.armed_at, 100, "reclaiming without saturation never moves the arming time");
    }

    fn output(id: &str, is_default: bool) -> super::super::DeviceInfo {
        super::super::DeviceInfo { id: id.into(), name: id.to_uppercase(), is_default }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cycling_with_no_or_disconnected_pins_changes_nothing() {
        let outputs = [output("speakers", true), output("hdmi", false)];
        assert_eq!(next_pinned_output(&[], &outputs), CycleOutcome::NothingPinned);
        assert_eq!(next_pinned_output(&ids(&["headset"]), &outputs), CycleOutcome::NoneConnected);
        assert_eq!(next_pinned_output(&ids(&["headset"]), &[]), CycleOutcome::NoneConnected);
    }

    #[test]
    fn cycling_with_one_pinned_device_selects_it_once() {
        let outputs = [output("speakers", true), output("headset", false)];
        assert_eq!(next_pinned_output(&ids(&["headset"]), &outputs), CycleOutcome::Switch(&outputs[1]));
        let now = [output("speakers", false), output("headset", true)];
        assert_eq!(next_pinned_output(&ids(&["headset"]), &now), CycleOutcome::AlreadyDefault(&now[1]));
    }

    #[test]
    fn cycling_follows_pin_order_and_skips_unpinned_and_disconnected_devices() {
        let pins = ids(&["headset", "cable", "speakers"]);
        // "cable" is disconnected; "hdmi" and "virtual" are not pinned.
        let mut outputs =
            vec![output("hdmi", false), output("speakers", false), output("virtual", false), output("headset", true)];
        let mut order = Vec::new();
        for _ in 0..4 {
            let CycleOutcome::Switch(next) = next_pinned_output(&pins, &outputs) else { panic!("expected a switch") };
            let id = next.id.clone();
            order.push(id.clone());
            for d in &mut outputs {
                d.is_default = d.id == id;
            }
        }
        assert_eq!(order, ["speakers", "headset", "speakers", "headset"], "deterministic pin order");

        // The default is not pinned: the first connected pinned device comes next.
        for d in &mut outputs {
            d.is_default = d.id == "hdmi";
        }
        assert_eq!(next_pinned_output(&pins, &outputs), CycleOutcome::Switch(&outputs[3]));

        // The cable is plugged in: it joins the cycle in its pinned position.
        outputs.push(output("cable", false));
        for d in &mut outputs {
            d.is_default = d.id == "headset";
        }
        let CycleOutcome::Switch(next) = next_pinned_output(&pins, &outputs) else { panic!() };
        assert_eq!(next.id, "cable");
    }

    #[test]
    fn custom_app_names_are_trimmed_unique_per_app_and_limited_to_stable_apps() {
        let (a, b) = (exe("C:\\Apps\\A.exe"), exe("c:\\apps\\b.exe"));
        let name = |key: &AppKey, text: &str| AppName { key: key.clone(), name: text.into() };
        let cleaned = normalize_app_names(vec![
            name(&a, "  Music  "),
            name(&exe("c:\\apps\\a.exe"), "Duplicate of the same app"),
            name(&b, "   "),
            name(&AppKey::Pid(4), "Transient"),
            name(&AppKey::System, &"x".repeat(200)),
        ]);
        assert_eq!(cleaned.len(), 2);
        assert_eq!((cleaned[0].key.clone(), cleaned[0].name.as_str()), (exe("c:\\apps\\a.exe"), "Music"));
        assert_eq!(cleaned[1].name.chars().count(), 60, "long names are cut");
        assert_eq!(custom_name(&cleaned, &a), Some("Music"), "found whatever the case of the path");
        assert_eq!(custom_name(&cleaned, &b), None);
        assert_eq!(custom_name(&cleaned, &AppKey::Pid(4)), None);
    }

    #[test]
    fn setting_a_name_adds_changes_clears_and_reports_whether_anything_changed() {
        let a = exe("c:\\apps\\a.exe");
        let mut names = Vec::new();
        assert!(!set_custom_name(&mut names, &a, "  "), "nothing to add");
        assert!(set_custom_name(&mut names, &a, " Music "));
        assert!(!set_custom_name(&mut names, &a, "Music"), "the same name changes nothing");
        assert!(set_custom_name(&mut names, &a, "Radio"));
        assert_eq!(custom_name(&names, &a), Some("Radio"));
        assert!(!set_custom_name(&mut names, &AppKey::Pid(9), "No"), "transient apps can't be named");
        assert!(set_custom_name(&mut names, &a, ""), "an empty name clears it");
        assert!(names.is_empty() && !set_custom_name(&mut names, &a, ""));
        for n in 0..MAX_APP_NAMES {
            assert!(set_custom_name(&mut names, &exe(&format!("c:\\apps\\{n}.exe")), "x"));
        }
        assert!(!set_custom_name(&mut names, &exe("c:\\apps\\one-too-many.exe"), "x"), "the list is bounded");
    }

    #[test]
    fn moving_an_item_swaps_with_its_neighbour_and_stops_at_the_ends() {
        let mut items = ["a", "b", "c"];
        assert!(move_item(&mut items, 1, -1));
        assert_eq!(items, ["b", "a", "c"]);
        assert!(move_item(&mut items, 1, 1));
        assert_eq!(items, ["b", "c", "a"]);
        assert!(!move_item(&mut items, 0, -1) && !move_item(&mut items, 2, 1), "the ends stay put");
        assert!(!move_item(&mut items, 9, 1) && !move_item(&mut items, 0, 2) && !move_item(&mut items, 0, 0));
        assert_eq!(items, ["b", "c", "a"], "refused moves change nothing");
    }

    #[test]
    fn pinned_devices_drop_duplicates_and_empty_ids() {
        let pin = |id: &str| PinnedDevice { id: id.into(), name: "x".repeat(300) };
        let pins = normalize_pinned_devices(vec![pin("{A}"), pin("{a}"), pin(" "), pin("{B}")]);
        assert_eq!(pins.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["{A}", "{B}"]);
        assert_eq!(pins[0].name.chars().count(), 160);
    }

    #[test]
    fn profiles_keep_the_preset_format_and_drop_only_invalid_parts() {
        let old: VolumePreset = serde_json::from_str(r#"{"name":"Old","entries":[],"apply_on_start":false}"#).unwrap();
        assert_eq!(old, VolumePreset { name: "Old".into(), ..Default::default() });
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            r#"{"name":"Old","entries":[],"apply_on_start":false}"#,
            "a profile without the new parts saves exactly as a preset did"
        );
        let text = r#"{"name":"New","master":{"volume":"loud"},"output_device":{"id":"{x}","name":"Headset"},
            "input_device":{"id":" ","name":"Blank"}}"#;
        let profile = serde_json::from_str::<VolumePreset>(text).unwrap().sanitize().unwrap();
        assert_eq!(profile.master, None, "a damaged part is dropped, not the profile");
        assert_eq!(profile.output_device.as_ref().map(|d| d.name.as_str()), Some("Headset"));
        assert_eq!(profile.input_device, None, "a device without an ID is dropped");
        let out_of_range = VolumePreset {
            name: "x".into(),
            master: Some(MasterLevel { volume: 3.0, muted: false }),
            ..Default::default()
        };
        assert_eq!(out_of_range.sanitize().unwrap().master, None);
    }

    #[test]
    fn a_profile_captures_only_the_chosen_parts() {
        use super::super::{AudioSnapshot, EndpointState};
        let endpoint = |id: &str, volume: f32| EndpointState {
            id: id.into(),
            name: id.to_uppercase(),
            volume,
            muted: false,
            balance: None,
        };
        let mut app = PinnedApp { key: exe("a.exe"), name: "A".into(), exe_path: None, aumid: None }.placeholder();
        app.session_count = 1;
        app.volume = 0.3;
        app.route = super::super::Route::Device("tv".into());
        let snapshot = AudioSnapshot {
            output: Some(endpoint("out", 0.7)),
            input: Some(endpoint("mic", 0.5)),
            apps: vec![app],
            outputs: vec![super::super::DeviceInfo { id: "tv".into(), name: "TV".into(), is_default: false }],
            ..Default::default()
        };
        let none = VolumePreset::capture_profile("x", &snapshot, CaptureParts::default());
        assert!(none.is_empty());
        let all = CaptureParts { apps: true, app_devices: true, master: true, output_device: true, input_device: true };
        let full = VolumePreset::capture_profile("Full", &snapshot, all);
        assert_eq!(full.entries.len(), 1);
        assert_eq!(full.entries[0].output, Some(DeviceChoice { id: "tv".into(), name: "TV".into() }));
        let levels_only =
            VolumePreset::capture_profile("L", &snapshot, CaptureParts { apps: true, ..Default::default() });
        assert_eq!(levels_only.entries[0].output, None, "an app's device is saved only when asked for");
        assert_eq!(full.master, Some(MasterLevel { volume: 0.7, muted: false }));
        assert_eq!(full.output_device.unwrap().id, "out");
        assert_eq!(full.input_device.unwrap().id, "mic");
        let devices =
            VolumePreset::capture_profile("D", &snapshot, CaptureParts { output_device: true, ..Default::default() });
        assert!(devices.entries.is_empty() && devices.master.is_none() && !devices.is_empty());
    }

    #[test]
    fn a_preset_plan_separates_running_from_missing_apps() {
        let entry =
            |key: AppKey, name: &str| PresetEntry { key, name: name.into(), volume: 0.5, muted: false, output: None };
        let preset = VolumePreset {
            name: "Mix".into(),
            apply_on_start: false,
            entries: vec![
                entry(AppKey::System, "System sounds"),
                entry(exe("running.exe"), "Running"),
                entry(exe("gone.exe"), "Gone"),
            ],
            ..Default::default()
        };
        let live = [AppKey::System, exe("running.exe")];
        let plan = plan_preset(&preset, |key| live.contains(key));
        assert_eq!(plan.present.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["System sounds", "Running"]);
        assert_eq!(plan.missing, 1);
        assert_eq!(plan_preset(&preset, |_| false).missing, 3);
        assert!(plan_preset(&preset, |_| false).present.is_empty());
    }
}
