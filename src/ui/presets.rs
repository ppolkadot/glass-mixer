use crate::{
    audio::{AudioHandle, AudioSnapshot, Command, preferences::VolumePreset},
    settings::Settings,
};

#[derive(Default)]
pub struct PresetEditor {
    name: String,
}

/// Preset names are unique ignoring case.
fn name_taken(presets: &[VolumePreset], name: &str) -> bool {
    presets.iter().any(|p| p.name.eq_ignore_ascii_case(name))
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

pub fn show(
    ui: &mut egui::Ui,
    editor: &mut PresetEditor,
    settings: &mut Settings,
    snapshot: &AudioSnapshot,
    audio: &AudioHandle,
    errors: &mut Vec<String>,
) {
    ui.heading("Volume presets");
    ui.label("Save real app volumes and mutes here explicitly. Device volumes and app balance are not included.");
    ui.horizontal(|ui| {
        let label = ui.label("Preset name");
        crate::ui::widgets::scroll_on_focus(ui.text_edit_singleline(&mut editor.name))
            .labelled_by(label.id)
            .on_hover_text("Name for a new preset");
    });
    let blocked = if editor.name.trim().is_empty() {
        Some("Enter a preset name to save.")
    } else if snapshot.apps.is_empty() {
        Some("No apps have audio sessions to save yet.")
    } else if settings.presets.len() >= 128 {
        Some("The preset limit (128) has been reached. Delete one to save another.")
    } else {
        None
    };
    let save = crate::ui::widgets::scroll_on_focus(
        ui.add_enabled(blocked.is_none(), egui::Button::new("Save current app volumes")),
    );
    if let Some(reason) = blocked {
        ui.add(egui::Label::new(egui::RichText::new(reason).small()).wrap());
    }
    if save.on_disabled_hover_text(blocked.unwrap_or_default()).clicked() {
        let preset = VolumePreset::capture(&editor.name, &snapshot.apps);
        if name_taken(&settings.presets, &preset.name) {
            errors.push("A preset with that name already exists. Choose another name.".into());
        } else if preset.entries.is_empty() {
            errors.push("No apps with stable identities can be saved yet.".into());
        } else {
            settings.presets.push(preset);
            editor.name.clear();
        }
    }
    ui.label("Automatic rules are off by default. They apply once to a newly launched process after enabling the rule, and only when its initial audio session is still at 100% and unmuted (so a non-default or Windows-restored level is not overwritten; use Apply now to override). Already-running apps and observed user changes are skipped.");
    let mut delete = None;
    let mut enable = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (index, preset) in settings.presets.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                ui.separator();
                ui.label(egui::RichText::new(&preset.name).strong());
                ui.horizontal_wrapped(|ui| {
                    let apply = crate::ui::widgets::scroll_on_focus(ui.button("Apply now"));
                    apply.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Apply {} now", preset.name))
                    });
                    if apply.clicked() {
                        audio.send(Command::ApplyPreset(preset.clone()));
                    }
                    let remove = crate::ui::widgets::scroll_on_focus(ui.button("Delete preset"));
                    remove.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            format!("Delete preset {}", preset.name),
                        )
                    });
                    if remove.clicked() {
                        delete = Some(index);
                    }
                });
                let mut on = preset.apply_on_start;
                let auto =
                    crate::ui::widgets::scroll_on_focus(ui.checkbox(&mut on, "Apply once when saved apps start"));
                auto.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::Checkbox,
                        true,
                        on,
                        format!("Apply {} once when saved apps start", preset.name),
                    )
                });
                if auto.changed() {
                    if on {
                        enable = Some(index);
                    } else {
                        preset.apply_on_start = false;
                    }
                }
                if preset.apply_on_start {
                    ui.label("Automatic rule enabled — applies once to new launches starting at 100% and unmuted (use Apply now to override remembered levels). Uncheck to stop.");
                }
                let collapsed =
                    egui::CollapsingHeader::new(format!("{} saved apps", preset.entries.len())).show(ui, |ui| {
                        for entry in &preset.entries {
                            ui.label(format!(
                                "{}: {}%{}",
                                entry.name,
                                crate::audio::to_percent(entry.volume),
                                if entry.muted { ", muted" } else { "" }
                            ));
                        }
                    });
                crate::ui::widgets::scroll_on_focus(collapsed.header_response);
            });
        }
    });
    if let Some(i) = enable {
        if auto_rule_conflict(&settings.presets, i) {
            errors.push("Another automatic preset includes the same app. Disable that rule first.".into());
        } else {
            settings.presets[i].apply_on_start = true;
        }
    }
    if let Some(i) = delete {
        settings.presets.remove(i);
    }
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
            entries: vec![PresetEntry { key: AppKey::System, name: "System sounds".into(), volume: 0.4, muted: false }],
        });
        let (mut editor, audio, mut errors) = (PresetEditor::default(), AudioHandle::detached(), Vec::new());
        let frame = harness.run(width, 900.0, |ui| show(ui, &mut editor, &mut settings, snapshot, &audio, &mut errors));
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
        assert!(names.contains(&"Preset name"), "the name field must be named: {names:?}");
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
                .map(|key| PresetEntry { key, name: "x".into(), volume: 0.5, muted: false })
                .collect(),
        }
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
        let frame =
            harness.run(400.0, 500.0, |ui| show(ui, &mut editor, &mut settings, &snapshot, &audio, &mut errors));
        let save = frame.widgets.iter().find(|w| w.name == "Save current app volumes").expect("Save button");
        assert!(save.disabled);
        assert!(
            frame.widgets.iter().any(|w| w.name.contains("Enter a preset name")),
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
                        }],
                    });
                }
                let snapshot = AudioSnapshot { status: ServiceStatus::Ok, ..Default::default() };
                let (mut editor, audio, mut errors) = (PresetEditor::default(), AudioHandle::detached(), Vec::new());

                let _ = harness.run(width, height, |ui| {
                    show(ui, &mut editor, &mut settings, &snapshot, &audio, &mut errors);
                });

                let mut reached_last = false;
                for _ in 0..40 {
                    let frame = harness.send_key(width, height, egui::Key::Tab, egui::Modifiers::NONE, |ui| {
                        show(ui, &mut editor, &mut settings, &snapshot, &audio, &mut errors);
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
                                show(ui, &mut editor, &mut settings, &snapshot, &audio, &mut errors);
                            });
                            assert!(settings.presets[7].apply_on_start);
                            reached_last = true;
                            break;
                        }
                    }
                }
                assert!(reached_last, "Tab traversal in Presets must reach the bottom preset");

                let mut reached_first = false;
                for _ in 0..40 {
                    let frame = harness.send_key(width, height, egui::Key::Tab, egui::Modifiers::SHIFT, |ui| {
                        show(ui, &mut editor, &mut settings, &snapshot, &audio, &mut errors);
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
