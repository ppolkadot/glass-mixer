//! Platform-independent audio state: snapshot types published to the UI and the
//! pure logic (session grouping, balance math, formatting) that is unit-tested.

use std::{collections::HashMap, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flow {
    Output,
    Input,
}

/// Stable identity of an application in the mixer. Sessions sharing a key are
/// shown as one row.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AppKey {
    /// The Windows "System sounds" session.
    System,
    /// A packaged (MSIX/Store) app, by Application User Model ID.
    Package(String),
    /// A desktop app, by lower-cased executable path.
    Exe(String),
    /// A process whose executable could not be determined.
    Pid(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EndpointState {
    pub id: String,
    pub name: String,
    /// Scalar volume, 0.0..=1.0.
    pub volume: f32,
    pub muted: bool,
    /// -1.0 (left) ..= 1.0 (right); `None` when the endpoint isn't stereo.
    pub balance: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    /// The session is currently rendering audio.
    Playing,
    /// The session exists but is not rendering.
    Idle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppEntry {
    pub key: AppKey,
    /// Best name known to the audio engine (session name, file description or exe name).
    pub name: String,
    pub exe_path: Option<PathBuf>,
    pub aumid: Option<String>,
    /// `IAudioSessionControl::GetIconPath`, when the app provides one.
    pub icon_resource: Option<String>,
    pub volume: f32,
    pub muted: bool,
    pub activity: Activity,
    pub session_count: usize,
    pub balance: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum ServiceStatus {
    #[default]
    Starting,
    Ok,
    /// The Windows audio service could not be reached; the engine keeps retrying.
    Unavailable(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioSnapshot {
    pub status: ServiceStatus,
    pub output: Option<EndpointState>,
    pub input: Option<EndpointState>,
    pub outputs: Vec<DeviceInfo>,
    pub inputs: Vec<DeviceInfo>,
    pub apps: Vec<AppEntry>,
    /// Whether the (undocumented) default-device switching interface is available.
    pub can_switch_default: bool,
}

impl AudioSnapshot {
    pub fn endpoint(&self, flow: Flow) -> Option<&EndpointState> {
        match flow {
            Flow::Output => self.output.as_ref(),
            Flow::Input => self.input.as_ref(),
        }
    }

    pub fn devices(&self, flow: Flow) -> &[DeviceInfo] {
        match flow {
            Flow::Output => &self.outputs,
            Flow::Input => &self.inputs,
        }
    }
}

/// Peak levels, 0.0..=1.0. Published separately from the snapshot because they
/// change continuously while audio plays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meters {
    pub output: f32,
    pub input: f32,
    pub apps: HashMap<AppKey, f32>,
}

/// Per-session facts gathered by the engine, before grouping into [`AppEntry`] rows.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionFacts {
    pub key: AppKey,
    pub name: String,
    pub exe_path: Option<PathBuf>,
    pub aumid: Option<String>,
    pub icon_resource: Option<String>,
    pub volume: f32,
    pub muted: bool,
    pub active: bool,
    pub balance: Option<f32>,
}

/// Groups sessions by application. The result is sorted: System sounds first,
/// then by name (case-insensitive), so rows don't jump around as state changes.
///
/// For a group, the displayed volume is the loudest session's volume, and the
/// group is "muted" only if every session is muted. Commands are applied to all
/// sessions of the group, so they converge after the first change.
pub fn group_sessions(sessions: &[SessionFacts]) -> Vec<AppEntry> {
    let mut groups: Vec<AppEntry> = Vec::new();
    let mut index: HashMap<&AppKey, usize> = HashMap::new();
    for s in sessions {
        if let Some(&i) = index.get(&s.key) {
            let g = &mut groups[i];
            g.volume = g.volume.max(s.volume);
            g.muted &= s.muted;
            if s.active {
                g.activity = Activity::Playing;
            }
            g.session_count += 1;
            if g.balance.is_none() {
                g.balance = s.balance;
            }
            if g.icon_resource.is_none() {
                g.icon_resource.clone_from(&s.icon_resource);
            }
            if g.name.is_empty() {
                g.name.clone_from(&s.name);
            }
        } else {
            index.insert(&s.key, groups.len());
            groups.push(AppEntry {
                key: s.key.clone(),
                name: s.name.clone(),
                exe_path: s.exe_path.clone(),
                aumid: s.aumid.clone(),
                icon_resource: s.icon_resource.clone(),
                volume: s.volume,
                muted: s.muted,
                activity: if s.active { Activity::Playing } else { Activity::Idle },
                session_count: 1,
                balance: s.balance,
            });
        }
    }
    groups.sort_by(|a, b| {
        let sys = |e: &AppEntry| e.key != AppKey::System;
        sys(a)
            .cmp(&sys(b))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.key.cmp(&b.key))
    });
    groups
}

/// Converts left/right channel levels into a balance in -1.0..=1.0.
pub fn balance_from_channels(left: f32, right: f32) -> f32 {
    let (l, r) = (left.max(0.0), right.max(0.0));
    if l <= f32::EPSILON && r <= f32::EPSILON {
        return 0.0;
    }
    if (l - r).abs() < 1e-4 {
        0.0
    } else if l > r {
        -(1.0 - r / l)
    } else {
        1.0 - l / r
    }
}

/// Converts a balance into per-channel multipliers whose maximum is 1.0.
pub fn channels_from_balance(balance: f32) -> (f32, f32) {
    let b = balance.clamp(-1.0, 1.0);
    let left = 1.0 - b.max(0.0);
    let right = 1.0 + b.min(0.0);
    (left, right)
}

pub fn to_percent(volume: f32) -> u32 {
    (volume.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// Case-insensitive substring match used by the app filter.
pub fn matches_filter(name: &str, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty() || name.to_lowercase().contains(&filter.to_lowercase())
}

/// A readable fallback name from an executable path: `C:\..\spotify.exe` -> `Spotify`.
pub fn friendly_exe_name(path: &std::path::Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut chars = stem.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Tray tooltip text. Windows truncates tooltips at 127 UTF-16 units.
pub fn tray_tooltip(snapshot: &AudioSnapshot) -> String {
    let text = match (&snapshot.status, &snapshot.output) {
        (ServiceStatus::Unavailable(_), _) => "Glass Mixer — audio service unavailable".to_string(),
        (_, None) => "Glass Mixer — no output device".to_string(),
        (_, Some(out)) if out.muted => format!("Glass Mixer — Muted · {}", out.name),
        (_, Some(out)) => format!("Glass Mixer — {}% · {}", to_percent(out.volume), out.name),
    };
    truncate_utf16(&text, 127)
}

fn truncate_utf16(text: &str, max_units: usize) -> String {
    if text.encode_utf16().count() <= max_units {
        return text.to_string();
    }
    let mut out = String::new();
    let mut units = 0;
    for c in text.chars() {
        let n = c.len_utf16();
        if units + n > max_units - 1 {
            break;
        }
        units += n;
        out.push(c);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(key: AppKey, name: &str, volume: f32, muted: bool, active: bool) -> SessionFacts {
        SessionFacts {
            key,
            name: name.into(),
            exe_path: None,
            aumid: None,
            icon_resource: None,
            volume,
            muted,
            active,
            balance: None,
        }
    }

    #[test]
    fn groups_sessions_of_the_same_app() {
        let firefox = AppKey::Exe(r"c:\program files\mozilla firefox\firefox.exe".into());
        let sessions = vec![
            facts(firefox.clone(), "Firefox", 0.4, true, false),
            facts(AppKey::Exe(r"c:\spotify.exe".into()), "Spotify", 0.8, false, true),
            facts(firefox.clone(), "Firefox", 0.7, false, true),
            facts(AppKey::System, "System sounds", 1.0, false, false),
        ];
        let apps = group_sessions(&sessions);
        assert_eq!(apps.len(), 3);
        assert_eq!(apps[0].key, AppKey::System, "system sounds first");
        assert_eq!(apps[1].name, "Firefox");
        assert_eq!(apps[1].session_count, 2);
        assert_eq!(apps[1].volume, 0.7, "loudest session wins");
        assert!(!apps[1].muted, "muted only if all sessions are muted");
        assert_eq!(apps[1].activity, Activity::Playing);
        assert_eq!(apps[2].name, "Spotify");
    }

    #[test]
    fn group_is_muted_when_all_sessions_are() {
        let k = AppKey::Pid(42);
        let apps = group_sessions(&[facts(k.clone(), "x", 0.5, true, false), facts(k, "x", 0.5, true, false)]);
        assert!(apps[0].muted);
        assert_eq!(apps[0].activity, Activity::Idle);
    }

    #[test]
    fn ordering_is_stable_and_case_insensitive() {
        let apps = group_sessions(&[
            facts(AppKey::Pid(1), "zoom", 1.0, false, false),
            facts(AppKey::Pid(2), "Audacity", 1.0, false, false),
            facts(AppKey::Pid(3), "brave", 1.0, false, false),
        ]);
        let names: Vec<_> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Audacity", "brave", "zoom"]);
    }

    #[test]
    fn balance_round_trips() {
        for b in [-1.0, -0.5, -0.1, 0.0, 0.25, 0.8, 1.0] {
            let (l, r) = channels_from_balance(b);
            assert!(l <= 1.0 && r <= 1.0 && (l == 1.0 || r == 1.0));
            let back = balance_from_channels(l, r);
            assert!((back - b).abs() < 1e-5, "{b} -> ({l},{r}) -> {back}");
        }
        assert_eq!(balance_from_channels(0.0, 0.0), 0.0);
        assert_eq!(balance_from_channels(0.5, 0.5), 0.0);
        assert!((balance_from_channels(0.6, 0.3) + 0.5).abs() < 1e-5);
    }

    #[test]
    fn percent_rounding() {
        assert_eq!(to_percent(0.0), 0);
        assert_eq!(to_percent(0.494), 49);
        assert_eq!(to_percent(0.505), 51);
        assert_eq!(to_percent(1.2), 100);
        assert_eq!(to_percent(-0.1), 0);
    }

    #[test]
    fn filter_matching() {
        assert!(matches_filter("Spotify", ""));
        assert!(matches_filter("Spotify", "  "));
        assert!(matches_filter("Spotify", "spot"));
        assert!(matches_filter("Microsoft Teams", "TEAMS"));
        assert!(!matches_filter("Spotify", "zoom"));
    }

    #[test]
    fn friendly_names() {
        assert_eq!(friendly_exe_name(std::path::Path::new(r"C:\Apps\spotify.exe")), "Spotify");
        assert_eq!(friendly_exe_name(std::path::Path::new(r"C:\x\Discord.exe")), "Discord");
        assert_eq!(friendly_exe_name(std::path::Path::new("")), "");
    }

    #[test]
    fn tooltip_formats() {
        let mut snap = AudioSnapshot {
            status: ServiceStatus::Ok,
            output: Some(EndpointState {
                id: "x".into(),
                name: "Speakers".into(),
                volume: 0.456,
                muted: false,
                balance: None,
            }),
            ..Default::default()
        };
        assert_eq!(tray_tooltip(&snap), "Glass Mixer — 46% · Speakers");
        snap.output.as_mut().unwrap().muted = true;
        assert_eq!(tray_tooltip(&snap), "Glass Mixer — Muted · Speakers");
        snap.output = None;
        assert_eq!(tray_tooltip(&snap), "Glass Mixer — no output device");
        snap.output =
            Some(EndpointState { id: "x".into(), name: "N".repeat(300), volume: 1.0, muted: false, balance: None });
        let t = tray_tooltip(&snap);
        assert!(t.encode_utf16().count() <= 127);
        assert!(t.ends_with('…'));
    }
}
