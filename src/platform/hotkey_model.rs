//! Pure data types and chord validation for global shortcuts.

use serde::{Deserialize, Serialize};

use crate::audio::AppKey;

/// An action requested by a shortcut that needs the UI thread's state (the
/// window, or the saved profiles). The UI thread performs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    ToggleWindow,
    ActivateProfile(String),
}

/// A binding that is saved but not active, with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedBinding {
    pub action: HotkeyAction,
    pub target: Option<HotkeyTarget>,
    pub chord: String,
    pub reason: String,
}

impl FailedBinding {
    pub fn message(&self) -> String {
        format!("{} ({}): {}", self.slot().label(), self.chord, self.reason)
    }

    pub(crate) fn slot(&self) -> Slot {
        Slot { action: self.action, target: self.target.clone() }
    }
}

/// Requested settings and active registrations can differ after a conflict.
#[derive(Clone, Debug, Default)]
pub struct HotkeyStatus {
    pub active: Vec<HotkeyBinding>,
    pub failed: Vec<FailedBinding>,
    /// Service-level problems, such as a stopped worker thread.
    pub errors: Vec<String>,
    pub pending: bool,
}

impl HotkeyStatus {
    /// Every problem as one line, for notices.
    pub fn messages(&self) -> Vec<String> {
        self.failed.iter().map(FailedBinding::message).chain(self.errors.iter().cloned()).collect()
    }
}

/// What a shortcut does. The `App*` actions and `ActivateProfile` need a
/// [`HotkeyTarget`]; the others must not have one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HotkeyAction {
    ToggleOutputMute,
    ToggleInputMute,
    VolumeUp,
    VolumeDown,
    ToggleWindow,
    CycleOutputDevice,
    AppMute,
    AppVolumeUp,
    AppVolumeDown,
    AppSolo,
    ActivateProfile,
}

impl HotkeyAction {
    /// The actions without a target, in the order the editor lists them.
    pub const ALL: [Self; 6] = [
        Self::ToggleOutputMute,
        Self::ToggleInputMute,
        Self::VolumeUp,
        Self::VolumeDown,
        Self::ToggleWindow,
        Self::CycleOutputDevice,
    ];
    pub const FOR_APP: [Self; 4] = [Self::AppMute, Self::AppVolumeUp, Self::AppVolumeDown, Self::AppSolo];

    pub fn label(self) -> &'static str {
        match self {
            Self::ToggleOutputMute => "Toggle output mute",
            Self::ToggleInputMute => "Toggle microphone mute",
            Self::VolumeUp => "Volume up",
            Self::VolumeDown => "Volume down",
            Self::ToggleWindow => "Show / hide mixer",
            Self::CycleOutputDevice => "Next pinned output device",
            Self::AppMute => "Mute / unmute",
            Self::AppVolumeUp => "Volume up",
            Self::AppVolumeDown => "Volume down",
            Self::AppSolo => "Solo on / off",
            Self::ActivateProfile => "Activate",
        }
    }

    fn needs_app(self) -> bool {
        Self::FOR_APP.contains(&self)
    }
}

/// The app or profile a targeted shortcut acts on. The name is for display;
/// apps are matched by key and profiles by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyTarget {
    App { key: AppKey, name: String },
    Profile { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyBinding {
    pub action: HotkeyAction,
    pub chord: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<HotkeyTarget>,
}

/// One assignable shortcut: an action, and the app or profile it targets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Slot {
    pub action: HotkeyAction,
    pub target: Option<HotkeyTarget>,
}

impl Slot {
    pub fn label(&self) -> String {
        match &self.target {
            None => self.action.label().to_owned(),
            Some(HotkeyTarget::App { name, .. }) => format!("{name}: {}", self.action.label()),
            Some(HotkeyTarget::Profile { name }) => format!("Activate profile {name}"),
        }
    }

    pub(crate) fn of(binding: &HotkeyBinding) -> Self {
        Self { action: binding.action, target: binding.target.clone() }
    }

    pub(crate) fn is_well_formed(&self) -> bool {
        match &self.target {
            None => HotkeyAction::ALL.contains(&self.action),
            Some(HotkeyTarget::App { .. }) => self.action.needs_app(),
            Some(HotkeyTarget::Profile { .. }) => self.action == HotkeyAction::ActivateProfile,
        }
    }
}

/// Every slot the editor offers: the global actions, four per pinned app, one
/// per profile, and any saved binding whose app or profile is gone (so it can
/// still be cleared).
pub fn slots(
    pins: &[crate::audio::preferences::PinnedApp],
    app_names: &[crate::audio::preferences::AppName],
    profiles: &[crate::audio::preferences::VolumePreset],
    bindings: &[HotkeyBinding],
) -> Vec<Slot> {
    let mut slots: Vec<Slot> = HotkeyAction::ALL.iter().map(|&action| Slot { action, target: None }).collect();
    for pin in pins {
        for action in HotkeyAction::FOR_APP {
            let name = crate::audio::preferences::custom_name(app_names, &pin.key).unwrap_or(&pin.name);
            let target = HotkeyTarget::App { key: pin.key.clone(), name: name.to_owned() };
            slots.push(Slot { action, target: Some(target) });
        }
    }
    for profile in profiles {
        slots.push(Slot {
            action: HotkeyAction::ActivateProfile,
            target: Some(HotkeyTarget::Profile { name: profile.name.clone() }),
        });
    }
    for binding in bindings {
        let slot = Slot::of(binding);
        let known = slots.iter().any(|s| same_slot(s, &slot));
        if !known && slot.is_well_formed() {
            slots.push(slot);
        }
    }
    slots
}

/// Apps are the same target whatever their display name; profiles compare by name ignoring case.
pub(crate) fn same_slot(a: &Slot, b: &Slot) -> bool {
    a.action == b.action
        && match (&a.target, &b.target) {
            (None, None) => true,
            (Some(HotkeyTarget::App { key: x, .. }), Some(HotkeyTarget::App { key: y, .. })) => x == y,
            (Some(HotkeyTarget::Profile { name: x }), Some(HotkeyTarget::Profile { name: y })) => {
                x.eq_ignore_ascii_case(y)
            }
            _ => false,
        }
}

pub(crate) struct Validated {
    pub(crate) valid: Vec<(HotkeyBinding, Chord)>,
    pub(crate) rejected: Vec<FailedBinding>,
}

/// Validates each binding on its own. When two bindings share a slot or a
/// chord, the earlier one wins and the later one is rejected.
pub(crate) fn validate_bindings(bindings: &[HotkeyBinding]) -> Validated {
    let mut valid: Vec<(HotkeyBinding, Chord)> = Vec::new();
    let mut rejected = Vec::new();
    for binding in bindings {
        let reject = |reason: String| FailedBinding {
            action: binding.action,
            target: binding.target.clone(),
            chord: binding.chord.clone(),
            reason,
        };
        let slot = Slot::of(binding);
        if !slot.is_well_formed() {
            rejected.push(reject("this action needs a matching app or profile.".into()));
            continue;
        }
        if valid.iter().any(|(other, _)| same_slot(&Slot::of(other), &slot)) {
            rejected.push(reject("choose only one shortcut for this action.".into()));
            continue;
        }
        match parse_chord(&binding.chord) {
            Ok(chord) => {
                if let Some((other, _)) = valid.iter().find(|(_, existing)| *existing == chord) {
                    rejected.push(reject(format!(
                        "{} is also assigned to {}. Choose a different chord.",
                        binding.chord,
                        Slot::of(other).label()
                    )));
                } else {
                    valid.push((binding.clone(), chord));
                }
            }
            Err(error) => rejected.push(reject(error)),
        }
    }
    Validated { valid, rejected }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Chord {
    pub(crate) modifiers: u32,
    pub(crate) key: u32,
}

pub(crate) fn parse_chord(value: &str) -> Result<Chord, String> {
    let mut modifiers = 0;
    let mut key = None;
    for part in value.split('+') {
        let part = part.trim().to_ascii_uppercase();
        let modifier = match part.as_str() {
            "CTRL" | "CONTROL" => 2,
            "ALT" => 1,
            "SHIFT" => 4,
            _ => 0,
        };
        if modifier != 0 {
            if modifiers & modifier != 0 || key.is_some() {
                return Err("Put each modifier once, before the key (for example Ctrl+Alt+M).".into());
            }
            modifiers |= modifier;
            continue;
        }
        if key.is_some() {
            return Err("Choose exactly one key after the modifiers.".into());
        }
        let code = match part.as_str() {
            "LEFT" => 0x25,
            "UP" => 0x26,
            "RIGHT" => 0x27,
            "DOWN" => 0x28,
            "SPACE" => 0x20,
            "ENTER" | "RETURN" => 0x0D,
            "TAB" => 0x09,
            "ESC" | "ESCAPE" => 0x1B,
            "HOME" => 0x24,
            "END" => 0x23,
            "PAGEUP" | "PGUP" => 0x21,
            "PAGEDOWN" | "PGDN" => 0x22,
            "INSERT" | "INS" => 0x2D,
            "DELETE" | "DEL" => 0x2E,
            "BACKSPACE" => 0x08,
            _ if part.len() == 1 && part.as_bytes()[0].is_ascii_alphanumeric() => part.as_bytes()[0] as u32,
            _ => {
                let function = part.strip_prefix('F').and_then(|n| n.parse::<u32>().ok());
                match function {
                    Some(n) if (1..=24).contains(&n) && part == format!("F{n}") => 0x6F + n,
                    _ => return Err("Use a letter, digit, F1–F24, arrow, or navigation key.".into()),
                }
            }
        };
        key = Some(code);
    }
    let key = key.ok_or_else(|| "Add a key after the modifiers (for example Ctrl+Alt+M).".to_owned())?;
    if modifiers == 0 {
        return Err("Include Ctrl, Alt, or Shift so ordinary typing stays available.".into());
    }
    if key == 0x7B {
        return Err("F12 is reserved by Windows; choose another key.".into());
    }
    if modifiers & 3 == 3 && key == 0x2E {
        return Err("Ctrl+Alt+Delete is reserved by Windows; choose another chord.".into());
    }
    Ok(Chord { modifiers, key })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(action: HotkeyAction, chord: &str) -> HotkeyBinding {
        HotkeyBinding { action, chord: chord.into(), target: None }
    }

    fn app_target(path: &str, name: &str) -> Option<HotkeyTarget> {
        Some(HotkeyTarget::App { key: AppKey::Exe(path.into()), name: name.into() })
    }

    fn targeted(action: HotkeyAction, chord: &str, target: Option<HotkeyTarget>) -> HotkeyBinding {
        HotkeyBinding { action, chord: chord.into(), target }
    }

    #[test]
    fn duplicate_chords_are_rejected_even_with_different_spelling() {
        let checked = validate_bindings(&[
            binding(HotkeyAction::VolumeUp, "Ctrl+Alt+A"),
            binding(HotkeyAction::VolumeDown, " alt + control + a "),
        ]);
        assert_eq!(checked.valid.len(), 1);
        assert_eq!(checked.valid[0].0.action, HotkeyAction::VolumeUp, "the earlier binding wins");
        assert_eq!(checked.rejected.len(), 1);
        assert_eq!(checked.rejected[0].action, HotkeyAction::VolumeDown);
    }

    #[test]
    fn one_invalid_binding_does_not_reject_the_valid_ones() {
        let checked = validate_bindings(&[
            binding(HotkeyAction::ToggleWindow, "Ctrl+Alt+F23"),
            binding(HotkeyAction::VolumeUp, "Ctrl+Alt+garbage"),
            binding(HotkeyAction::VolumeDown, "Ctrl+Alt+F22"),
        ]);
        let valid: Vec<_> = checked.valid.iter().map(|(b, _)| b.action).collect();
        assert_eq!(valid, [HotkeyAction::ToggleWindow, HotkeyAction::VolumeDown]);
        assert_eq!(checked.rejected.len(), 1);
        assert_eq!(checked.rejected[0].action, HotkeyAction::VolumeUp);
        assert!(checked.rejected[0].message().starts_with("Volume up (Ctrl+Alt+garbage): "));
    }

    #[test]
    fn duplicate_actions_keep_the_first_and_report_the_rest() {
        let checked = validate_bindings(&[
            binding(HotkeyAction::VolumeUp, "Ctrl+F1"),
            binding(HotkeyAction::VolumeUp, "Ctrl+F2"),
            binding(HotkeyAction::VolumeDown, "garbage"),
        ]);
        assert_eq!(checked.valid.len(), 1);
        assert_eq!(checked.rejected.len(), 2);
    }

    #[test]
    fn empty_configuration_disables_every_hotkey() {
        let checked = validate_bindings(&[]);
        assert!(checked.valid.is_empty() && checked.rejected.is_empty());
    }

    #[test]
    fn parses_modifiers_case_whitespace_and_key_aliases() {
        assert_eq!(parse_chord(" shift + CTRL + a ").unwrap(), Chord { modifiers: 6, key: 65 });
        assert_eq!(parse_chord("Alt+Left").unwrap(), Chord { modifiers: 1, key: 0x25 });
        assert_eq!(parse_chord("Ctrl+F24").unwrap(), Chord { modifiers: 2, key: 0x87 });
        assert_eq!(parse_chord("Ctrl+0").unwrap(), Chord { modifiers: 2, key: 48 });
        assert_eq!(parse_chord("Ctrl+PageDown"), parse_chord("Ctrl+PgDn"));
    }

    #[test]
    fn rejects_ambiguous_reserved_or_incomplete_chords() {
        for invalid in [
            "",
            "A",
            "Ctrl",
            "Ctrl+",
            "+A",
            "Ctrl+Ctrl+A",
            "Ctrl+A+B",
            "Win+A",
            "Ctrl+F0",
            "Ctrl+F25",
            "Ctrl+F12",
            "Ctrl+Ä",
            "Ctrl++",
            "Ctrl+F01",
            "Ctrl+Alt+Delete",
        ] {
            assert!(parse_chord(invalid).is_err(), "accepted invalid chord: {invalid}");
        }
    }

    #[test]
    fn targeted_bindings_are_one_per_app_and_need_a_matching_target() {
        let a = app_target("a.exe", "A");
        let checked = validate_bindings(&[
            targeted(HotkeyAction::AppMute, "Ctrl+Alt+1", a.clone()),
            targeted(HotkeyAction::AppMute, "Ctrl+Alt+2", app_target("b.exe", "B")),
            targeted(HotkeyAction::AppMute, "Ctrl+Alt+3", app_target("a.exe", "A renamed")),
            targeted(HotkeyAction::AppMute, "Ctrl+Alt+4", None),
            targeted(HotkeyAction::VolumeUp, "Ctrl+Alt+5", a.clone()),
            targeted(HotkeyAction::ActivateProfile, "Ctrl+Alt+6", a),
        ]);
        let valid: Vec<_> = checked.valid.iter().map(|(b, _)| b.chord.as_str()).collect();
        assert_eq!(valid, ["Ctrl+Alt+1", "Ctrl+Alt+2"], "the same action for two apps is fine");
        let reasons: Vec<_> = checked.rejected.iter().map(|f| (f.chord.as_str(), f.reason.as_str())).collect();
        assert_eq!(reasons[0], ("Ctrl+Alt+3", "choose only one shortcut for this action."), "same app, new name");
        assert!(reasons[1..].iter().all(|(_, r)| r.contains("matching app or profile")), "{reasons:?}");
        assert!(checked.rejected[0].message().starts_with("A renamed: Mute / unmute (Ctrl+Alt+3)"));
    }

    #[test]
    fn bindings_without_a_target_keep_their_saved_format() {
        let old: HotkeyBinding = serde_json::from_str(r#"{"action":"VolumeUp","chord":"Ctrl+Alt+F23"}"#).unwrap();
        assert_eq!(old, binding(HotkeyAction::VolumeUp, "Ctrl+Alt+F23"));
        assert_eq!(serde_json::to_string(&old).unwrap(), r#"{"action":"VolumeUp","chord":"Ctrl+Alt+F23"}"#);
        let app = targeted(HotkeyAction::AppSolo, "Ctrl+Alt+S", app_target(r"c:\x.exe", "X"));
        let round: HotkeyBinding = serde_json::from_str(&serde_json::to_string(&app).unwrap()).unwrap();
        assert_eq!(round, app);
    }

    #[test]
    fn the_editor_offers_pinned_apps_profiles_and_orphaned_bindings() {
        use crate::audio::preferences::{PinnedApp, VolumePreset};
        let pin = PinnedApp { key: AppKey::Exe("a.exe".into()), name: "A".into(), exe_path: None, aumid: None };
        let profile =
            VolumePreset { name: "Work".into(), entries: Vec::new(), apply_on_start: false, ..Default::default() };
        let orphan = targeted(HotkeyAction::AppSolo, "Ctrl+Alt+S", app_target("gone.exe", "Gone"));
        let all = slots(&[pin], &[], &[profile], std::slice::from_ref(&orphan));
        let labels: Vec<_> = all.iter().map(Slot::label).collect();
        assert_eq!(labels.len(), HotkeyAction::ALL.len() + 4 + 1 + 1);
        assert!(labels.contains(&"A: Solo on / off".to_owned()));
        assert!(labels.contains(&"Activate profile Work".to_owned()));
        assert!(labels.contains(&"Gone: Solo on / off".to_owned()), "an unpinned app's shortcut can still be cleared");
    }
}
