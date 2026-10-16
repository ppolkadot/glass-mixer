use crate::{
    audio::{AudioHandle, AudioSnapshot, Command, preferences::VolumePreset},
    settings::Settings,
};

#[derive(Default)]
pub struct PresetEditor {
    name: String,
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
        ui.label("Preset name");
        ui.text_edit_singleline(&mut editor.name).on_hover_text("Name for a new preset");
    });
    let can_save = !editor.name.trim().is_empty() && !snapshot.apps.is_empty() && settings.presets.len() < 128;
    if ui.add_enabled(can_save, egui::Button::new("Save current app volumes")).clicked() {
        let preset = VolumePreset::capture(&editor.name, &snapshot.apps);
        if settings.presets.iter().any(|p| p.name.eq_ignore_ascii_case(&preset.name)) {
            errors.push("A preset with that name already exists. Choose another name.".into());
        } else if preset.entries.is_empty() {
            errors.push("No apps with stable identities can be saved yet.".into());
        } else {
            settings.presets.push(preset);
            editor.name.clear();
        }
    }
    ui.label("Automatic rules are off by default. They only apply once to a newly launched process after enabling the rule. Already-running apps and observed user changes are skipped.");
    let mut delete = None;
    let mut enable = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (index, preset) in settings.presets.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                ui.separator();
                ui.label(egui::RichText::new(&preset.name).strong());
                ui.horizontal(|ui| {
                    let apply = ui.button("Apply now");
                    apply.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Apply {} now", preset.name))
                    });
                    if apply.clicked() {
                        audio.send(Command::ApplyPreset(preset.clone()));
                    }
                    let remove = ui.button("Delete preset");
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
                let auto = ui.checkbox(&mut on, "Apply once when saved apps start");
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
                    ui.label("Automatic rule enabled — uncheck to stop applying on future starts.");
                }
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
            });
        }
    });
    if let Some(i) = enable {
        let conflict = settings.presets.iter().enumerate().any(|(j, p)| {
            j != i
                && p.apply_on_start
                && p.entries.iter().any(|e| settings.presets[i].entries.iter().any(|other| other.key == e.key))
        });
        if conflict {
            errors.push("Another automatic preset includes the same app. Disable that rule first.".into());
        } else {
            settings.presets[i].apply_on_start = true;
        }
    }
    if let Some(i) = delete {
        settings.presets.remove(i);
    }
}
