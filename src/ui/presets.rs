//! The Profiles view: save the current state as a profile, then apply, rename,
//! duplicate, trim or delete it. Profiles are stored as `presets` in settings.

use super::{
    Look,
    typography::{self, Role},
    widgets::{self, ButtonKind},
};
use crate::{
    audio::{
        AudioHandle, AudioSnapshot, Command,
        preferences::{CaptureParts, VolumePreset},
    },
    platform::hotkeys::HotkeyTarget,
    settings::Settings,
};

pub struct PresetEditor {
    name: String,
    parts: CaptureParts,
    /// The profile being renamed (by its current name, which is unique, so
    /// inserting or importing profiles can't move the draft to another one) and
    /// the new name typed so far.
    renaming: Option<(String, String)>,
}

#[cfg(test)]
impl PresetEditor {
    pub fn start_rename_for_test(&mut self, name: &str) {
        self.renaming = Some((name.to_owned(), format!("{name} 2")));
    }
}

impl Default for PresetEditor {
    fn default() -> Self {
        Self { name: String::new(), parts: CaptureParts { apps: true, ..CaptureParts::default() }, renaming: None }
    }
}

/// Profile names are unique ignoring case.
fn name_taken(presets: &[VolumePreset], name: &str) -> bool {
    presets.iter().any(|p| p.name.eq_ignore_ascii_case(name))
}

/// "Gaming copy", then "Gaming copy 2", and so on, keeping within the 80-character limit.
fn copy_name(presets: &[VolumePreset], name: &str) -> String {
    let base: String = name.chars().take(70).collect();
    let mut candidate = format!("{base} copy");
    let mut n = 2;
    while name_taken(presets, &candidate) {
        candidate = format!("{base} copy {n}");
        n += 1;
    }
    candidate
}

/// Whether enabling the automatic rule of `presets[index]` would overlap another
/// enabled rule: two rules for one app could fight over its levels.
fn auto_rule_conflict(presets: &[VolumePreset], index: usize) -> bool {
    presets.iter().enumerate().any(|(j, p)| {
        j != index
            && p.apply_on_start
            && p.entries.iter().any(|e| presets[index].entries.iter().any(|other| other.key == e.key))
    })
}

/// Shortcuts follow a renamed profile and are removed with a deleted one.
fn retarget_shortcuts(settings: &mut Settings, old: &str, new: Option<&str>) {
    let is_old =
        |t: &Option<HotkeyTarget>| matches!(t, Some(HotkeyTarget::Profile { name }) if name.eq_ignore_ascii_case(old));
    match new {
        Some(new) => {
            for binding in settings.hotkeys.iter_mut().filter(|b| is_old(&b.target)) {
                binding.target = Some(HotkeyTarget::Profile { name: new.to_owned() });
            }
        }
        None => settings.hotkeys.retain(|b| !is_old(&b.target)),
    }
}

fn named_button(ui: &mut egui::Ui, look: &Look, text: &str, name: &str, kind: ButtonKind) -> egui::Response {
    let response = widgets::button(ui, look, text, kind);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), name));
    response
}

enum Action {
    Apply(usize),
    StartRename(usize),
    Rename(usize, String),
    Duplicate(usize),
    Delete(usize),
    EnableRule(usize),
    DisableRule(usize),
    RemoveEntry(usize, usize),
    RemoveMaster(usize),
    RemoveOutput(usize),
    RemoveInput(usize),
}

pub fn show(
    ui: &mut egui::Ui,
    look: &Look,
    editor: &mut PresetEditor,
    settings: &mut Settings,
    snapshot: &AudioSnapshot,
    audio: &AudioHandle,
    errors: &mut Vec<String>,
) {
    // One scroll area for the whole view, so keyboard focus can reach every profile.
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        contents(ui, look, editor, settings, snapshot, audio, errors);
    });
}

fn contents(
    ui: &mut egui::Ui,
    look: &Look,
    editor: &mut PresetEditor,
    settings: &mut Settings,
    snapshot: &AudioSnapshot,
    audio: &AudioHandle,
    errors: &mut Vec<String>,
) {
    let p = &look.p;
    ui.label(typography::rich("Profiles", Role::LargeTitle, look).color(p.text));
    ui.add(
        egui::Label::new(
            typography::rich(
                "A profile stores only what you save in it: app volumes and mutes, and optionally the master level and default devices. Apply it here or with a shortcut (Settings > Global shortcuts).",
                Role::Footnote,
                look,
            )
            .color(p.text_muted),
        )
        .wrap(),
    );
    ui.add_space(look.m.gap);
    // A device is saved on an app's entry, so it can't be saved without the apps.
    if !editor.parts.apps {
        editor.parts.app_devices = false;
    }
    let preview = VolumePreset::capture_profile(&editor.name, snapshot, editor.parts);
    let blocked = if editor.name.trim().is_empty() {
        Some("Enter a profile name to save.")
    } else if settings.presets.len() >= 128 {
        Some("The profile limit (128) has been reached. Delete one to save another.")
    } else if preview.is_empty() {
        Some("Nothing to save yet: choose what to include, or start an app that plays audio.")
    } else {
        None
    };
    widgets::card(look).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(typography::rich("Save current levels", Role::CalloutStrong, look).color(p.text));
        ui.horizontal(|ui| {
            let label = ui.label(typography::rich("Profile name", Role::Body, look).color(p.text));
            widgets::scroll_on_focus(
                ui.add(
                    egui::TextEdit::singleline(&mut editor.name)
                        .desired_width((ui.available_width() - 4.0).clamp(80.0, 220.0)),
                ),
            )
            .labelled_by(label.id)
            .on_hover_text("Name for a new profile");
        });
        let parts = &mut editor.parts;
        widgets::scroll_on_focus(widgets::check_row(ui, look, &mut parts.apps, "App volumes and mutes"));
        let apps_on = parts.apps;
        ui.add_enabled_ui(apps_on, |ui| {
            widgets::scroll_on_focus(widgets::check_row(ui, look, &mut parts.app_devices, "App devices"))
                .on_disabled_hover_text(
                    "App devices are saved with app volumes and mutes. Turn those on to include them.",
                );
        });
        if !apps_on {
            ui.add(
                egui::Label::new(
                    typography::rich(
                        "App devices are saved with app volumes and mutes. Turn those on to include them.",
                        Role::Footnote,
                        look,
                    )
                    .color(p.text_muted),
                )
                .wrap(),
            );
        }
        for (on, text) in [
            (&mut parts.master, "Master volume"),
            (&mut parts.output_device, "Default output"),
            (&mut parts.input_device, "Default input"),
        ] {
            widgets::scroll_on_focus(widgets::check_row(ui, look, on, text));
        }
        let save = ui
            .add_enabled_ui(blocked.is_none(), |ui| {
                widgets::button(ui, look, "Save current state", ButtonKind::Primary)
            })
            .inner;
        if let Some(reason) = blocked {
            ui.add(egui::Label::new(typography::rich(reason, Role::Footnote, look).color(p.text_muted)).wrap());
        }
        if save.on_disabled_hover_text(blocked.unwrap_or_default()).clicked() {
            if name_taken(&settings.presets, &preview.name) {
                errors.push("A profile with that name already exists. Choose another name.".into());
            } else {
                settings.presets.push(preview);
                editor.name.clear();
            }
        }
    });
    ui.add_space(look.m.gap);
    ui.add(
        egui::Label::new(
            typography::rich(
                "Automatic rules are off by default. They apply a profile's app levels once to a newly launched process after enabling the rule, and only when its initial audio session is still at 100% and unmuted (so a non-default or Windows-restored level is not overwritten; use Apply now to override). Already-running apps and observed user changes are skipped. Devices and the master level change only when you apply a profile.",
                Role::Footnote,
                look,
            )
            .color(p.text_muted),
        )
        .wrap(),
    );
    ui.add_space(look.m.gap);

    // A rename draft for a profile that is gone (replaced by an import) is dropped.
    if let Some((target, _)) = &editor.renaming
        && !settings.presets.iter().any(|p| p.name.eq_ignore_ascii_case(target))
    {
        editor.renaming = None;
    }
    let mut action = None;
    for (index, preset) in settings.presets.iter().enumerate() {
        ui.push_id(index, |ui| profile_card(ui, look, editor, index, preset, &mut action));
    }
    let Some(action) = action else { return };
    match action {
        Action::Apply(i) => audio.send(Command::ApplyPreset(settings.presets[i].clone())),
        Action::StartRename(i) => {
            let name = settings.presets[i].name.clone();
            editor.renaming = Some((name.clone(), name));
        }
        Action::Rename(i, new) => {
            let new: String = new.trim().chars().take(80).collect();
            let old = settings.presets[i].name.clone();
            if new.is_empty() {
                errors.push("A profile needs a name.".into());
            } else if !new.eq_ignore_ascii_case(&old) && name_taken(&settings.presets, &new) {
                errors.push("A profile with that name already exists. Choose another name.".into());
            } else {
                settings.presets[i].name.clone_from(&new);
                retarget_shortcuts(settings, &old, Some(&new));
                editor.renaming = None;
            }
        }
        Action::Duplicate(i) => {
            if settings.presets.len() >= 128 {
                errors.push("The profile limit (128) has been reached. Delete one to add another.".into());
            } else {
                let mut copy = settings.presets[i].clone();
                copy.name = copy_name(&settings.presets, &copy.name);
                // Two identical automatic rules would always conflict.
                copy.apply_on_start = false;
                settings.presets.insert(i + 1, copy);
            }
        }
        Action::Delete(i) => {
            let removed = settings.presets.remove(i);
            retarget_shortcuts(settings, &removed.name, None);
            editor.renaming = None;
        }
        Action::EnableRule(i) => {
            if auto_rule_conflict(&settings.presets, i) {
                errors.push("Another automatic profile includes the same app. Disable that rule first.".into());
            } else {
                settings.presets[i].apply_on_start = true;
            }
        }
        Action::DisableRule(i) => settings.presets[i].apply_on_start = false,
        Action::RemoveEntry(i, e) => {
            settings.presets[i].entries.remove(e);
            if settings.presets[i].entries.is_empty() {
                settings.presets[i].apply_on_start = false;
            }
        }
        Action::RemoveMaster(i) => settings.presets[i].master = None,
        Action::RemoveOutput(i) => settings.presets[i].output_device = None,
        Action::RemoveInput(i) => settings.presets[i].input_device = None,
    }
}

fn profile_card(
    ui: &mut egui::Ui,
    look: &Look,
    editor: &mut PresetEditor,
    index: usize,
    preset: &VolumePreset,
    action: &mut Option<Action>,
) {
    let name = &preset.name;
    widgets::card(look).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let mut cancel = false;
        match &mut editor.renaming {
            Some((target, draft)) if target.eq_ignore_ascii_case(name) => {
                ui.horizontal_wrapped(|ui| {
                    let field = widgets::scroll_on_focus(
                        ui.add(
                            egui::TextEdit::singleline(draft)
                                .desired_width((ui.available_width() - 4.0).clamp(80.0, 220.0)),
                        ),
                    );
                    widgets::set_name(ui, &field, &format!("New name for {name}"));
                    if named_button(
                        ui,
                        look,
                        "Save name",
                        &format!("Save the new name for {name}"),
                        ButtonKind::Primary,
                    )
                    .clicked()
                    {
                        *action = Some(Action::Rename(index, draft.clone()));
                    }
                    cancel = widgets::button(ui, look, "Cancel", ButtonKind::Secondary).clicked();
                });
            }
            _ => {
                ui.label(typography::rich(name, Role::CalloutStrong, look).color(look.p.text));
            }
        }
        if cancel {
            editor.renaming = None;
        }
        ui.horizontal_wrapped(|ui| {
            if named_button(ui, look, "Apply now", &format!("Apply {name} now"), ButtonKind::Secondary).clicked() {
                *action = Some(Action::Apply(index));
            }
            if named_button(ui, look, "Rename", &format!("Rename profile {name}"), ButtonKind::Secondary).clicked() {
                *action = Some(Action::StartRename(index));
            }
            if named_button(ui, look, "Duplicate", &format!("Duplicate profile {name}"), ButtonKind::Secondary).clicked()
            {
                *action = Some(Action::Duplicate(index));
            }
            if named_button(ui, look, "Delete", &format!("Delete profile {name}"), ButtonKind::Destructive).clicked() {
                *action = Some(Action::Delete(index));
            }
        });
        if !preset.entries.is_empty() {
            let mut on = preset.apply_on_start;
            let auto = widgets::scroll_on_focus(widgets::check_row(
                ui,
                look,
                &mut on,
                "Apply once when saved apps start",
            ));
            auto.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Checkbox,
                    true,
                    on,
                    format!("Apply {name} once when saved apps start"),
                )
            });
            if auto.changed() {
                *action = Some(if on { Action::EnableRule(index) } else { Action::DisableRule(index) });
            }
            if preset.apply_on_start {
                ui.add(
                    egui::Label::new(
                        typography::rich(
                            "Automatic rule enabled — applies once to new launches starting at 100% and unmuted (use Apply now to override remembered levels). Uncheck to stop.",
                            Role::Footnote,
                            look,
                        )
                        .color(look.p.text_muted),
                    )
                    .wrap(),
                );
            }
        }
        let parts = preset.entries.len()
            + usize::from(preset.master.is_some())
            + usize::from(preset.output_device.is_some())
            + usize::from(preset.input_device.is_some());
        let collapsed = widgets::disclosure(ui, look, &format!("{parts} saved setting(s)"), |ui| {
            let mut row = |ui: &mut egui::Ui, text: String, remove: Action| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(typography::rich(&text, Role::Callout, look).color(look.p.text));
                    if named_button(ui, look, "Remove", &format!("Remove {text} from {name}"), ButtonKind::Secondary)
                        .clicked()
                    {
                        *action = Some(remove);
                    }
                });
            };
            if let Some(master) = preset.master {
                let level = if master.muted {
                    "muted".to_owned()
                } else {
                    format!("{}%", crate::audio::to_percent(master.volume))
                };
                row(ui, format!("Master volume: {level}"), Action::RemoveMaster(index));
            }
            if let Some(device) = &preset.output_device {
                row(ui, format!("Default output: {}", device.name), Action::RemoveOutput(index));
            }
            if let Some(device) = &preset.input_device {
                row(ui, format!("Default input: {}", device.name), Action::RemoveInput(index));
            }
            for (e, entry) in preset.entries.iter().enumerate() {
                let level = format!(
                    "{}: {}%{}{}",
                    entry.name,
                    crate::audio::to_percent(entry.volume),
                    if entry.muted { ", muted" } else { "" },
                    entry.output.as_ref().map_or(String::new(), |d| format!(", on {}", d.name()))
                );
                row(ui, level, Action::RemoveEntry(index, e));
            }
        });
        widgets::scroll_on_focus(collapsed.header_response);
    });
    ui.add_space(look.m.gap);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::preferences::{PresetEntry, VolumePreset};
    use crate::audio::{AppKey, ServiceStatus};
    use crate::settings::{Density, MIN_WIDTH};
    use crate::ui::test_support::{Harness, is_interactive, past_right_edge, unnamed};

    fn render(snapshot: &AudioSnapshot) -> (f32, crate::ui::test_support::Frame) {
        let harness = Harness::new(Density::Comfortable);
        let width = MIN_WIDTH - 2.0 * harness.look.m.pad;
        let mut settings = Settings::default();
        settings.presets.push(VolumePreset {
            name: "A Preset With A Fairly Long Name For Evening Listening".into(),
            apply_on_start: true,
            entries: vec![PresetEntry {
                key: AppKey::System,
                name: "System sounds".into(),
                volume: 0.4,
                muted: false,
                output: None,
            }],
            master: Some(crate::audio::preferences::MasterLevel { volume: 0.5, muted: false }),
            output_device: Some(crate::audio::preferences::DeviceChoice {
                id: "{out}".into(),
                name: "Speakers (High Definition Audio Device With A Long Name)".into(),
            }),
            ..Default::default()
        });
        let look = harness.look;
        let (mut editor, audio, mut errors) = (PresetEditor::default(), AudioHandle::detached(), Vec::new());
        let frame =
            harness.run(width, 900.0, |ui| show(ui, &look, &mut editor, &mut settings, snapshot, &audio, &mut errors));
        (width, frame)
    }

    #[test]
    fn presets_view_fits_and_every_control_is_named() {
        let snapshot = AudioSnapshot { status: ServiceStatus::Ok, ..Default::default() };
        let (width, frame) = render(&snapshot);
        assert!(unnamed(&frame.widgets).is_empty(), "unnamed controls: {:?}", unnamed(&frame.widgets));
        let clipped: Vec<_> = past_right_edge(&frame.widgets, width).iter().map(|w| w.name.clone()).collect();
        assert!(clipped.is_empty(), "controls past the right edge ({width}): {clipped:?}");
        assert!(frame.content_right <= width + 0.5, "content reaches {}", frame.content_right);
        let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        assert!(names.contains(&"Profile name"), "the name field must be named: {names:?}");
        let name = "A Preset With A Fairly Long Name For Evening Listening";
        for action in ["Rename profile", "Duplicate profile", "Delete profile"] {
            assert!(names.contains(&format!("{action} {name}").as_str()), "missing {action}: {names:?}");
        }
        assert!(names.contains(&"3 saved setting(s)"), "{names:?}");
        assert!(
            frame.widgets.iter().any(|w| w.name.contains("100% and unmuted")),
            "automatic rule restriction must be explained in the Presets UI"
        );
    }

    fn preset(name: &str, apps: Vec<AppKey>, auto: bool) -> VolumePreset {
        VolumePreset {
            name: name.into(),
            apply_on_start: auto,
            entries: apps
                .into_iter()
                .map(|key| PresetEntry { key, name: "x".into(), volume: 0.5, muted: false, output: None })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn copies_get_unique_names_and_shortcuts_follow_renames_and_deletions() {
        use crate::platform::hotkeys::{HotkeyAction, HotkeyBinding};
        let presets = [preset("Gaming", vec![], false), preset("gaming copy", vec![], false)];
        assert_eq!(copy_name(&presets, "Gaming"), "Gaming copy 2");
        assert!(copy_name(&presets, &"x".repeat(200)).chars().count() <= 80);

        let shortcut = |name: &str| HotkeyBinding {
            action: HotkeyAction::ActivateProfile,
            chord: "Ctrl+Alt+G".into(),
            target: Some(HotkeyTarget::Profile { name: name.into() }),
        };
        let mut settings = Settings { hotkeys: vec![shortcut("Gaming"), shortcut("Work")], ..Default::default() };
        retarget_shortcuts(&mut settings, "gaming", Some("Games"));
        assert_eq!(settings.hotkeys, vec![shortcut("Games"), shortcut("Work")]);
        retarget_shortcuts(&mut settings, "Work", None);
        assert_eq!(settings.hotkeys, vec![shortcut("Games")]);
    }

    /// The names of the profiles whose rename field is on screen.
    fn rename_fields(settings: &mut Settings, editor: &mut PresetEditor) -> Vec<String> {
        let harness = Harness::new(Density::Comfortable);
        let (audio, mut errors, snapshot) = (AudioHandle::detached(), Vec::new(), AudioSnapshot::default());
        let frame =
            harness.run(500.0, 900.0, |ui| show(ui, &harness.look, editor, settings, &snapshot, &audio, &mut errors));
        frame.widgets.iter().filter_map(|w| w.name.strip_prefix("New name for ")).map(str::to_owned).collect()
    }

    #[test]
    fn a_rename_follows_its_profile_when_another_is_inserted_before_it() {
        let mut settings =
            Settings { presets: vec![preset("A", vec![], false), preset("B", vec![], false)], ..Default::default() };
        let mut editor = PresetEditor { renaming: Some(("B".into(), "B2".into())), ..Default::default() };
        assert_eq!(rename_fields(&mut settings, &mut editor), ["B"]);

        // Duplicating A inserts "A copy" ahead of B, moving B to index 2.
        let copy = preset("A copy", vec![], false);
        settings.presets.insert(1, copy);
        assert_eq!(settings.presets[2].name, "B");
        assert_eq!(rename_fields(&mut settings, &mut editor), ["B"], "still B, not the new copy");
        assert_eq!(editor.renaming.as_ref().map(|(_, draft)| draft.as_str()), Some("B2"));

        // If the profile is replaced (an import), the stale draft is dropped.
        settings.presets = vec![preset("C", vec![], false)];
        assert!(rename_fields(&mut settings, &mut editor).is_empty());
        assert!(editor.renaming.is_none());
    }

    #[test]
    fn app_devices_need_app_volumes_and_say_so() {
        let harness = Harness::new(Density::Comfortable);
        let mut settings = Settings::default();
        let snapshot = AudioSnapshot::default();
        let mut editor = PresetEditor {
            parts: CaptureParts { apps: false, app_devices: true, ..CaptureParts::default() },
            ..Default::default()
        };
        let audio = AudioHandle::detached();
        let mut errors = Vec::new();
        let frame = harness.run(500.0, 900.0, |ui| {
            show(ui, &harness.look, &mut editor, &mut settings, &snapshot, &audio, &mut errors)
        });
        assert!(!editor.parts.app_devices, "a device-only selection is cleared");
        let devices = frame.widgets.iter().find(|w| w.name == "App devices").expect("the App devices option");
        assert!(devices.disabled);
        assert!(
            frame.widgets.iter().any(|w| w.name.contains("saved with app volumes and mutes")),
            "the dependency must be explained"
        );
    }

    #[test]
    fn preset_names_are_unique_ignoring_case() {
        let presets = [preset("Gaming", vec![], false)];
        assert!(name_taken(&presets, "gaming"));
        assert!(name_taken(&presets, "GAMING"));
        assert!(!name_taken(&presets, "Gaming 2"));
    }

    #[test]
    fn two_automatic_rules_may_not_share_an_app() {
        let (a, b, c) = (AppKey::Exe("a.exe".into()), AppKey::Exe("b.exe".into()), AppKey::Exe("c.exe".into()));
        let presets = vec![
            preset("One", vec![a.clone(), b.clone()], true),
            preset("Two", vec![b.clone()], false),
            preset("Three", vec![c], false),
            preset("Four", vec![a.clone()], true),
        ];
        assert!(auto_rule_conflict(&presets, 1), "Two shares b.exe with the enabled rule One");
        assert!(!auto_rule_conflict(&presets, 2), "Three shares nothing");
        assert!(auto_rule_conflict(&presets, 0), "One conflicts with the enabled Four on a.exe");
        // A preset never conflicts with itself, and disabled presets never conflict.
        assert!(!auto_rule_conflict(&[preset("Solo", vec![a.clone()], true)], 0));
        assert!(!auto_rule_conflict(&[preset("Off", vec![a.clone()], false), preset("Also", vec![a], false)], 1));
    }

    #[test]
    fn a_disabled_save_button_says_why() {
        // No apps and no name: the button is disabled, and the reason is visible text.
        let harness = Harness::new(Density::Comfortable);
        let mut settings = Settings::default();
        let snapshot = AudioSnapshot::default();
        let (mut editor, audio, mut errors) = (PresetEditor::default(), AudioHandle::detached(), Vec::new());
        let frame = harness.run(400.0, 500.0, |ui| {
            show(ui, &harness.look, &mut editor, &mut settings, &snapshot, &audio, &mut errors)
        });
        let save = frame.widgets.iter().find(|w| w.name == "Save current state").expect("Save button");
        assert!(save.disabled);
        assert!(
            frame.widgets.iter().any(|w| w.name.contains("Enter a profile name")),
            "the reason is not shown: {:?}",
            frame.widgets.iter().map(|w| &w.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn keyboard_tab_shift_tab_and_space_keep_focus_visible_in_presets() {
        for density in [Density::Comfortable, Density::Compact] {
            for (window_width, height) in [(MIN_WIDTH, 440.0), (480.0, 720.0)] {
                let harness = Harness::new(density);
                let width = window_width - 2.0 * harness.look.m.pad;
                let mut settings = Settings::default();
                for i in 0..8 {
                    settings.presets.push(VolumePreset {
                        name: format!("Preset Number {i}"),
                        apply_on_start: false,
                        entries: vec![PresetEntry {
                            key: AppKey::Exe(format!(r"c:\apps\p_{i}.exe")),
                            name: format!("App {i}"),
                            volume: 0.5,
                            muted: false,
                            output: None,
                        }],
                        ..Default::default()
                    });
                }
                let snapshot = AudioSnapshot { status: ServiceStatus::Ok, ..Default::default() };
                let (mut editor, audio, mut errors) = (PresetEditor::default(), AudioHandle::detached(), Vec::new());

                let _ = harness.run(width, height, |ui| {
                    show(ui, &harness.look, &mut editor, &mut settings, &snapshot, &audio, &mut errors);
                });

                let mut reached_last = false;
                for _ in 0..80 {
                    let frame = harness.send_key(width, height, egui::Key::Tab, egui::Modifiers::NONE, |ui| {
                        show(ui, &harness.look, &mut editor, &mut settings, &snapshot, &audio, &mut errors);
                    });
                    if let Some(widget) = frame.widgets.iter().find(|w| w.focused && is_interactive(w.role)) {
                        if let Some(b) = widget.bounds {
                            assert!(
                                b.min.y >= -1.0 && b.max.y <= height + 1.0,
                                "Presets focused control '{}' off-screen at {b:?} in {width}x{height} ({density:?})",
                                widget.name
                            );
                        }
                        if widget.name == "Apply Preset Number 7 once when saved apps start" {
                            // Toggle automatic rule with Space.
                            let _ = harness.send_key(width, height, egui::Key::Space, egui::Modifiers::NONE, |ui| {
                                show(ui, &harness.look, &mut editor, &mut settings, &snapshot, &audio, &mut errors);
                            });
                            assert!(settings.presets[7].apply_on_start);
                            reached_last = true;
                            break;
                        }
                    }
                }
                assert!(reached_last, "Tab traversal in Presets must reach the bottom preset");

                let mut reached_first = false;
                for _ in 0..80 {
                    let frame = harness.send_key(width, height, egui::Key::Tab, egui::Modifiers::SHIFT, |ui| {
                        show(ui, &harness.look, &mut editor, &mut settings, &snapshot, &audio, &mut errors);
                    });
                    if let Some(widget) = frame.widgets.iter().find(|w| w.focused && is_interactive(w.role)) {
                        if let Some(b) = widget.bounds {
                            assert!(
                                b.min.y >= -1.0 && b.max.y <= height + 1.0,
                                "Presets Shift+Tab focused control '{}' off-screen at {b:?} in {width}x{height} ({density:?})",
                                widget.name
                            );
                        }
                        if widget.name == "Apply Preset Number 0 now" {
                            reached_first = true;
                            break;
                        }
                    }
                }
                assert!(reached_first, "Shift+Tab in Presets must return to the top preset");
            }
        }
    }
}
