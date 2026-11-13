//! Global shortcut editor UI.

use crate::platform::{
    hotkey_model::{HotkeyBinding, HotkeyStatus, Slot, Validated, same_slot, validate_bindings},
    hotkeys::HotkeyService,
};

#[derive(Default)]
pub struct HotkeyEditor {
    drafts: std::collections::HashMap<Slot, String>,
    initialized: std::collections::HashSet<Slot>,
    error: Option<String>,
}

fn primary_chord_for_slot(
    slot: &Slot,
    bindings: &[HotkeyBinding],
    validated: &Validated,
    status: &HotkeyStatus,
) -> Option<String> {
    let in_slot = |b: &HotkeyBinding| same_slot(&Slot::of(b), slot);
    if let Some(active) =
        status.active.iter().find(|b| in_slot(b) && bindings.iter().any(|raw| in_slot(raw) && raw.chord == b.chord))
    {
        return Some(active.chord.clone());
    }
    if let Some((valid, _)) = validated.valid.iter().find(|(b, _)| in_slot(b)) {
        return Some(valid.chord.clone());
    }
    bindings.iter().find(|b| in_slot(b)).map(|b| b.chord.clone())
}

impl HotkeyEditor {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        bindings: &mut Vec<HotkeyBinding>,
        step: &mut u8,
        service: &HotkeyService,
        slots: &[Slot],
    ) {
        let status = service.status();
        if self.show_slots(ui, bindings, step, &status, slots)
            && let Err(error) = service.configure(bindings, *step)
        {
            self.error = Some(error);
        }
    }

    /// The editor for the global actions only.
    #[cfg(test)]
    pub(crate) fn show_with_status(
        &mut self,
        ui: &mut egui::Ui,
        bindings: &mut Vec<HotkeyBinding>,
        step: &mut u8,
        status: &HotkeyStatus,
    ) -> bool {
        let slots: Vec<Slot> = crate::platform::hotkey_model::HotkeyAction::ALL
            .iter()
            .map(|&action| Slot { action, target: None })
            .collect();
        self.show_slots(ui, bindings, step, status, &slots)
    }

    /// Returns whether Retry was clicked.
    pub fn show_slots(
        &mut self,
        ui: &mut egui::Ui,
        bindings: &mut Vec<HotkeyBinding>,
        step: &mut u8,
        status: &HotkeyStatus,
        slots: &[Slot],
    ) -> bool {
        let validated = validate_bindings(bindings);
        for slot in slots {
            if self.initialized.insert(slot.clone())
                && let Some(chord) = primary_chord_for_slot(slot, bindings, &validated, status)
            {
                self.drafts.entry(slot.clone()).or_insert(chord);
            }
        }
        let mut retry_clicked = false;
        ui.label("No shortcuts are assigned by default. Type a chord such as Ctrl+Alt+M, then Bind. Windows may reject keys used by another app.");
        ui.horizontal(|ui| {
            let label = ui.label("Volume step");
            crate::ui::widgets::scroll_on_focus(ui.add(egui::DragValue::new(step).range(1..=20).suffix("%")))
                .labelled_by(label.id)
                .on_hover_text("Volume change for each volume shortcut, 1 to 20 percent");
        });
        let (global, targeted): (Vec<&Slot>, Vec<&Slot>) = slots.iter().partition(|s| s.target.is_none());
        for slot in global {
            self.slot_row(ui, bindings, status, &validated, slot);
        }
        if !targeted.is_empty() {
            let header =
                egui::CollapsingHeader::new(format!("App and profile shortcuts ({})", targeted.len())).show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("Pin an app in the Mixer or save a profile to add its shortcuts.")
                                .small(),
                        )
                        .wrap(),
                    );
                    for slot in targeted {
                        self.slot_row(ui, bindings, status, &validated, slot);
                    }
                });
            crate::ui::widgets::scroll_on_focus(header.header_response);
        }
        self.footer(ui, status, &mut retry_clicked);
        retry_clicked
    }

    fn slot_row(
        &mut self,
        ui: &mut egui::Ui,
        bindings: &mut Vec<HotkeyBinding>,
        status: &HotkeyStatus,
        validated: &Validated,
        slot: &Slot,
    ) {
        let in_slot = |b: &HotkeyBinding| same_slot(&Slot::of(b), slot);
        let title = slot.label();
        {
            ui.push_id(slot, |ui| {
                let current = primary_chord_for_slot(slot, bindings, validated, status).unwrap_or_else(|| "Off".into());
                let label = ui.label(format!("{title} — {current}"));
                if current != "Off" {
                    let is_active = status.active.iter().any(|b| in_slot(b) && b.chord == current);
                    let mut consumed_primary_failure = false;
                    let state = if is_active {
                        "Active".to_owned()
                    } else if let Some(failure) =
                        status.failed.iter().find(|f| same_slot(&f.slot(), slot) && f.chord == current)
                    {
                        consumed_primary_failure = true;
                        format!("Not active: {}", failure.reason)
                    } else if status.pending {
                        "Registering…".to_owned()
                    } else {
                        "Not active".to_owned()
                    };
                    ui.add(egui::Label::new(egui::RichText::new(state).small()).wrap());
                    let mut skipped_primary = false;
                    for extra in status.failed.iter().filter(|f| same_slot(&f.slot(), slot)) {
                        if consumed_primary_failure && !skipped_primary && extra.chord == current {
                            skipped_primary = true;
                            continue;
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("Ignored duplicate ({}): {}", extra.chord, extra.reason))
                                    .small(),
                            )
                            .wrap(),
                        );
                    }
                }
                ui.horizontal(|ui| {
                    let draft = self.drafts.entry(slot.clone()).or_default();
                    crate::ui::widgets::scroll_on_focus(
                        ui.add(egui::TextEdit::singleline(draft).hint_text("Ctrl+Alt+M").desired_width(140.0)),
                    )
                    .labelled_by(label.id)
                    .on_hover_text(format!("Shortcut for {title}"));
                    // Short visible text keeps the row narrow; the accessible name names the action.
                    let bind = crate::ui::widgets::scroll_on_focus(ui.button("Bind"));
                    crate::ui::widgets::set_name(ui, &bind, &format!("Bind {title}"));
                    if bind.on_hover_text(format!("Bind {title}")).clicked() {
                        let mut proposed = bindings.clone();
                        proposed.retain(|b| !in_slot(b));
                        proposed.push(HotkeyBinding {
                            action: slot.action,
                            chord: draft.trim().to_string(),
                            target: slot.target.clone(),
                        });
                        // Another action's invalid entry must not block this one.
                        match validate_bindings(&proposed).rejected.into_iter().find(|f| same_slot(&f.slot(), slot)) {
                            None => {
                                *bindings = proposed;
                                self.error = None;
                            }
                            Some(failure) => self.error = Some(failure.message()),
                        }
                    }
                    let clear = crate::ui::widgets::scroll_on_focus(
                        ui.add_enabled(current != "Off", egui::Button::new("Clear")),
                    );
                    crate::ui::widgets::set_name(ui, &clear, &format!("Clear {title}"));
                    if clear.on_hover_text(format!("Clear {title}")).clicked() {
                        bindings.retain(|b| !in_slot(b));
                        self.drafts.remove(slot);
                        self.error = None;
                    }
                });
            });
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui, status: &HotkeyStatus, retry_clicked: &mut bool) {
        if let Some(error) = &self.error {
            ui.label(format!("Shortcut error: {error}"));
        }
        if status.pending {
            ui.label("Registering shortcuts with Windows…");
        }
        for error in &status.errors {
            ui.label(format!("Shortcut error: {error}"));
        }
        if !status.failed.is_empty() {
            ui.add(
                egui::Label::new(format!(
                    "{} shortcut entry(ies) are not active. Valid non-conflicting shortcuts remain active. If another app held a key, release it and click Retry failed shortcuts; use Bind or Clear to fix invalid or duplicate entries.",
                    status.failed.len()
                ))
                .wrap(),
            );
            if crate::ui::widgets::scroll_on_focus(
                ui.add_enabled(!status.pending, egui::Button::new("Retry failed shortcuts")),
            )
            .on_hover_text("Try to register every shortcut that is not active")
            .clicked()
            {
                *retry_clicked = true;
            }
        }
        ui.label(format!("{} active shortcut(s)", status.active.len()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::hotkey_model::{FailedBinding, HotkeyAction};

    fn binding(action: HotkeyAction, chord: &str) -> HotkeyBinding {
        HotkeyBinding { action, chord: chord.into(), target: None }
    }

    #[test]
    fn editor_presentation_handles_active_invalid_conflicting_duplicate_pending_and_cleared_states() {
        use crate::settings::Density;
        use crate::ui::test_support::Harness;

        let harness = Harness::new(Density::Comfortable);
        let mut editor = HotkeyEditor::default();
        // Hand-edited settings with:
        // - valid-first + duplicate-second for VolumeUp (Ctrl+Alt+F23 then Ctrl+Alt+F22),
        // - invalid-first + valid-second for ToggleWindow (garbage then Ctrl+F1),
        // - invalid VolumeDown (Ctrl+Alt+nonsense),
        // - conflicting ToggleOutputMute (Ctrl+Alt+M).
        let mut bindings = vec![
            binding(HotkeyAction::VolumeUp, "Ctrl+Alt+F23"),
            binding(HotkeyAction::VolumeUp, "Ctrl+Alt+F22"),
            binding(HotkeyAction::ToggleWindow, "garbage"),
            binding(HotkeyAction::ToggleWindow, "Ctrl+F1"),
            binding(HotkeyAction::VolumeDown, "Ctrl+Alt+nonsense"),
            binding(HotkeyAction::ToggleOutputMute, "Ctrl+Alt+M"),
        ];
        let mut step = 5_u8;
        let checked = validate_bindings(&bindings);
        let mut failed = checked.rejected;
        failed.push(FailedBinding {
            action: HotkeyAction::ToggleOutputMute,
            target: None,
            chord: "Ctrl+Alt+M".into(),
            reason: "Windows could not register this shortcut; another app may already use it.".into(),
        });
        let status = HotkeyStatus {
            active: vec![
                binding(HotkeyAction::VolumeUp, "Ctrl+Alt+F23"),
                binding(HotkeyAction::ToggleWindow, "Ctrl+F1"),
            ],
            failed,
            pending: false,
            errors: Vec::new(),
        };

        let frame = harness.run(360.0, 720.0, |ui| {
            let _ = editor.show_with_status(ui, &mut bindings, &mut step, &status);
        });
        let labels: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        // Valid-first duplicate binding (Ctrl+Alt+F23) is shown as header AND draft AND Active.
        assert!(labels.contains(&"Volume up — Ctrl+Alt+F23"), "primary duplicate must be shown: {labels:?}");
        assert!(
            labels.iter().filter(|&&l| l == "Active").count() == 2,
            "both active bindings (valid-first VolumeUp and invalid-first ToggleWindow) must show Active: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.starts_with("Ignored duplicate (Ctrl+Alt+F22):")),
            "rejected duplicate for the same action must be reported separately: {labels:?}"
        );
        // Invalid-first + valid-second (ToggleWindow: garbage then Ctrl+F1) selects Ctrl+F1 as
        // primary and reports garbage as an ignored duplicate entry.
        assert!(
            labels.contains(&"Show / hide mixer — Ctrl+F1"),
            "retained valid/active binding must be selected even when preceded by an invalid entry: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.starts_with("Ignored duplicate (garbage):")),
            "rejected invalid first entry must still be displayed alongside the active second entry: {labels:?}"
        );
        let vol_up_input = frame
            .widgets
            .iter()
            .find(|w| w.role == egui::accesskit::Role::TextInput && w.name == "Volume up — Ctrl+Alt+F23")
            .expect("Volume up text input");
        assert_eq!(
            vol_up_input.value.as_deref(),
            Some("Ctrl+Alt+F23"),
            "draft input must initialize with the first (active) binding, not the ignored duplicate"
        );
        let toggle_win_input = frame
            .widgets
            .iter()
            .find(|w| w.role == egui::accesskit::Role::TextInput && w.name == "Show / hide mixer — Ctrl+F1")
            .expect("Show / hide mixer text input");
        assert_eq!(
            toggle_win_input.value.as_deref(),
            Some("Ctrl+F1"),
            "draft input for invalid-first duplicate must initialize with the retained valid/active binding"
        );
        assert!(
            labels.iter().any(|l| l.contains("Not active: Use a letter, digit")),
            "invalid binding reason must be shown: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("Not active: Windows could not register this shortcut")),
            "conflicting binding reason must be shown: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("use Bind or Clear to fix invalid or duplicate entries")),
            "summary banner must distinguish retryable conflicts from invalid/duplicate entries: {labels:?}"
        );

        let pending_status =
            HotkeyStatus { active: Vec::new(), failed: status.failed.clone(), pending: true, errors: Vec::new() };
        let pending_frame = harness.run(360.0, 720.0, |ui| {
            let _ = editor.show_with_status(ui, &mut bindings, &mut step, &pending_status);
        });
        assert!(pending_frame.widgets.iter().any(|w| w.name == "Registering shortcuts with Windows…"));
        let retry_btn =
            pending_frame.widgets.iter().find(|w| w.name == "Retry failed shortcuts").expect("Retry button");
        assert!(retry_btn.disabled, "Retry button must be disabled while registration is pending");

        bindings.clear();
        let cleared_status = HotkeyStatus::default();
        let cleared_frame = harness.run(360.0, 720.0, |ui| {
            let _ = editor.show_with_status(ui, &mut bindings, &mut step, &cleared_status);
        });
        let clear_vol_up =
            cleared_frame.widgets.iter().find(|w| w.name == "Clear Volume up").expect("Clear Volume up button");
        assert!(clear_vol_up.disabled, "Clear button must be disabled when action is Off");
    }
}
