//! Settings page: appearance, behaviour, audio and about.

use super::widgets::{self, card, glyph, icon_button, segmented, semibold, toggle_row};
use super::{GlassStatus, Look, theme};
use crate::audio::{AudioHandle, Command};
use crate::platform::backdrop::Support;
use crate::platform::hotkeys::{HotkeyEditor, HotkeyService};
use crate::platform::icons::IconService;
use crate::platform::startup::{self, LoginState};
use crate::settings::{Accent, Backdrop, DEFAULT_HEIGHT, DEFAULT_WIDTH, Density, Settings, ThemeMode, Typeface};
use egui::{Align, Align2, Key, Layout, RichText, Sense, Stroke, Ui, Vec2, ViewportCommand};
use std::path::Path;

pub enum Outcome {
    None,
    Back,
    /// Write the diagnostic report to a file and show the folder.
    SaveDiagnostics,
    CopyDiagnostics,
    ExportSettings,
    /// Read and check the file named in [`TransferUi::path`].
    CheckImport,
    /// The checked import in [`TransferUi::candidate`] was confirmed.
    ApplyImport,
}

/// What the Backup and restore section shows: the path typed, and a checked import
/// waiting for confirmation (the settings it holds, and lines describing them).
#[derive(Default)]
pub struct TransferUi {
    pub path: String,
    pub candidate: Option<(Settings, Vec<String>)>,
    pub error: Option<String>,
}

pub struct SettingsCtx<'a> {
    pub look: &'a Look,
    pub settings: &'a mut Settings,
    pub login: &'a mut LoginState,
    pub glass: GlassStatus,
    pub support: Support,
    /// Frames are drawn by the CPU, so level meters are paused.
    pub software_renderer: bool,
    pub can_switch_default: bool,
    pub can_route_apps: bool,
    /// The "reset all app devices" confirmation is showing.
    pub confirm_route_reset: &'a mut bool,
    pub transfer: &'a mut TransferUi,
    pub audio: &'a AudioHandle,
    pub icons: &'a mut IconService,
    pub settings_path: &'a Path,
    pub hotkeys: Option<&'a HotkeyService>,
    pub hotkey_editor: &'a mut HotkeyEditor,
    /// Error messages to show as notices.
    pub errors: &'a mut Vec<String>,
}

pub fn show(ui: &mut Ui, cx: &mut SettingsCtx) -> Outcome {
    let look = *cx.look;
    let mut outcome = Outcome::None;
    ui.horizontal(|ui| {
        if icon_button(ui, &look, glyph::BACK, "<", "Back to mixer (Esc)", None).clicked() {
            outcome = Outcome::Back;
        }
        ui.label(RichText::new("Settings").font(semibold(18.0)));
    });
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        outcome = Outcome::Back;
    }
    ui.add_space(look.m.gap);

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        section(ui, &look, "Appearance", |ui| appearance(ui, cx));
        section(ui, &look, "Behavior", |ui| behavior(ui, cx));
        section(ui, &look, "Audio", |ui| audio(ui, cx));
        section(ui, &look, "Global shortcuts", |ui| {
            if let Some(service) = cx.hotkeys {
                let slots = crate::platform::hotkeys::slots(
                    &cx.settings.pinned_apps,
                    &cx.settings.app_names,
                    &cx.settings.presets,
                    &cx.settings.hotkeys,
                );
                cx.hotkey_editor.show(ui, &mut cx.settings.hotkeys, &mut cx.settings.hotkey_step, service, &slots);
            } else {
                ui.label("Global shortcuts are unavailable in this session.");
            }
        });
        section(ui, &look, "Backup and restore", |ui| backup(ui, cx, &mut outcome));
        section(ui, &look, "About", |ui| about(ui, cx, &mut outcome));
    });
    outcome
}

fn section(ui: &mut Ui, look: &Look, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.label(RichText::new(title).font(semibold(13.0)).color(look.p.text_muted));
    ui.add_space(2.0);
    card(look).show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui);
    });
    ui.add_space(look.m.gap * 1.4);
}

fn labeled(ui: &mut Ui, look: &Look, label: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(34.0);
        ui.label(RichText::new(label).color(if ui.is_enabled() { look.p.text } else { look.p.text_muted }));
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
}

fn appearance(ui: &mut Ui, cx: &mut SettingsCtx) {
    let look = *cx.look;
    let s = &mut *cx.settings;
    labeled(ui, &look, "Theme", |ui| {
        segmented(
            ui,
            &look,
            &mut s.theme,
            &[(ThemeMode::System, "System"), (ThemeMode::Light, "Light"), (ThemeMode::Dark, "Dark")],
            210.0,
        );
    });
    toggle_row(ui, &look, &mut s.glass, "Glass effect", Some(cx.glass.description()));
    ui.add_enabled_ui(s.glass && cx.glass == GlassStatus::Active && cx.support == Support::Full, |ui| {
        labeled(ui, &look, "Backdrop", |ui| {
            segmented(ui, &look, &mut s.backdrop, &[(Backdrop::Acrylic, "Acrylic"), (Backdrop::Mica, "Mica")], 160.0);
        });
    });
    labeled(ui, &look, "Accent", |ui| {
        for accent in Accent::ALL.iter().rev() {
            swatch(ui, &look, &mut s.accent, *accent);
        }
    });
    labeled(ui, &look, "Typeface", |ui| {
        segmented(ui, &look, &mut s.typeface, &[(Typeface::Polaris, "Polaris"), (Typeface::System, "Windows")], 210.0);
    });
    ui.label(
        RichText::new("Polaris is the bundled Salted Flutters Polaris. Windows uses Segoe UI.")
            .small()
            .color(look.p.text_muted),
    );
    labeled(ui, &look, "Density", |ui| {
        segmented(
            ui,
            &look,
            &mut s.density,
            &[(Density::Comfortable, "Comfortable"), (Density::Compact, "Compact")],
            210.0,
        );
    });
    let meters = if cx.software_renderer {
        "Paused: without graphics acceleration, live levels would use a lot of CPU"
    } else {
        "Show live audio levels under the sliders"
    };
    toggle_row(ui, &look, &mut s.show_meters, "Level meters", Some(meters));
    if widgets::scroll_on_focus(ui.button("Use recommended window size (480 × 720)")).clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(DEFAULT_WIDTH, DEFAULT_HEIGHT)));
    }
}

fn swatch(ui: &mut Ui, look: &Look, current: &mut Accent, accent: Accent) {
    let size = 24.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let selected = *current == accent;
    if response.clicked() {
        *current = accent;
    }
    let color = theme::accent_color(accent, look.p.dark);
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size / 2.0 - 3.0, color);
    if selected {
        painter.circle_stroke(rect.center(), size / 2.0 - 0.5, Stroke::new(2.0, look.p.text));
        if look.icons {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                glyph::CHECK,
                widgets::icon_font(10.0),
                look.p.on_accent,
            );
        } else {
            painter.circle_filled(rect.center(), 3.0, look.p.on_accent);
        }
    } else if response.hovered() {
        painter.circle_stroke(rect.center(), size / 2.0 - 0.5, Stroke::new(1.5, look.p.text_muted));
    }
    widgets::focus_ring(ui, rect, &response, &look.p, 14);
    let label = accent.label();
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, format!("{label} accent"))
    });
    response.on_hover_text(label);
}

fn behavior(ui: &mut Ui, cx: &mut SettingsCtx) {
    let look = *cx.look;
    let mut login = *cx.login != LoginState::Disabled;
    let desc = match *cx.login {
        LoginState::BlockedByUser => "Disabled in Task Manager. Turn off and on again to re-enable.",
        _ => "Start in the tray when you sign in to Windows",
    };
    if toggle_row(ui, &look, &mut login, "Launch at login", Some(desc)).changed() {
        if let Err(e) = startup::set_enabled(login) {
            cx.errors.push(e);
        }
        *cx.login = startup::state();
    }
    let s = &mut *cx.settings;
    toggle_row(
        ui,
        &look,
        &mut s.start_minimized,
        "Start minimized",
        Some("Open only the tray icon when Glass Mixer starts"),
    );
    toggle_row(
        ui,
        &look,
        &mut s.close_to_tray,
        "Keep running when closed",
        Some("The close button hides to the tray; use Exit in the tray menu to quit"),
    );
    toggle_row(ui, &look, &mut s.minimize_to_tray, "Minimize to tray", Some("Hide from the taskbar when minimized"));
    toggle_row(
        ui,
        &look,
        &mut s.hidden_notifications,
        "Tray notifications",
        Some("Warnings and errors while hidden; Windows notification settings apply"),
    );
    toggle_row(
        ui,
        &look,
        &mut s.show_inactive_apps,
        "Show idle apps",
        Some("List apps that have an audio session but aren't playing"),
    );
}

fn audio(ui: &mut Ui, cx: &mut SettingsCtx) {
    let look = *cx.look;
    let s = &mut *cx.settings;
    // Preset steps, or "Custom" for any other value the field below sets.
    let mut preset = if [1, 2, 5].contains(&s.wheel_step) { s.wheel_step } else { 0 };
    labeled(ui, &look, "Mouse wheel step", |ui| {
        let custom = ui.add_enabled(preset == 0, egui::DragValue::new(&mut s.wheel_step).range(1..=20).suffix("%"));
        widgets::set_name(ui, &custom, "Custom mouse wheel step");
        widgets::scroll_on_focus(custom).on_hover_text("Any step from 1 to 20 percent");
        if segmented(ui, &look, &mut preset, &[(1, "1%"), (2, "2%"), (5, "5%"), (0, "Custom")], 200.0) {
            s.wheel_step = if preset == 0 { 3 } else { preset };
        }
    });
    ui.label(RichText::new("Volume change per wheel notch over a volume slider.").small().color(look.p.text_muted));
    let mut keep = s.solo.keep_percent.min(90);
    labeled(ui, &look, "Solo keeps other apps at", |ui| {
        let field = ui.add_enabled(!s.solo.mute, egui::DragValue::new(&mut keep).range(0..=90).suffix("%"));
        widgets::set_name(ui, &field, "Solo keeps other apps at this percent of their volume");
        widgets::scroll_on_focus(field);
    });
    s.solo.keep_percent = keep;
    toggle_row(
        ui,
        &look,
        &mut s.solo.mute,
        "Solo mutes other apps",
        Some(
            "Mute other apps instead of lowering them. Their own levels come back when Solo ends, unless you changed them meanwhile.",
        ),
    );
    ui.horizontal(|ui| {
        if widgets::scroll_on_focus(ui.button("Refresh audio devices"))
            .on_hover_text("Reconnect to all devices and audio sessions")
            .clicked()
        {
            cx.icons.retry_failed();
            cx.audio.send(Command::Refresh);
        }
    });
    ui.label(
        RichText::new("Live volumes come from Windows. Presets store app levels only when you save one.")
            .small()
            .color(look.p.text_muted),
    );
    mic_ducking(ui, cx);
    if cx.can_route_apps {
        app_device_reset(ui, cx);
    }
    if !cx.can_switch_default {
        ui.label(
            RichText::new(
                "Default device switching isn't available on this system. Use Windows Settings > System > Sound.",
            )
            .small()
            .color(look.p.text_muted),
        );
    }
}

/// Opt-in: lower chosen pinned apps while the microphone picks up speech.
fn mic_ducking(ui: &mut Ui, cx: &mut SettingsCtx) {
    let look = *cx.look;
    let s = &mut *cx.settings;
    let mic = &mut s.mic_ducking;
    toggle_row(
        ui,
        &look,
        &mut mic.enabled,
        "Microphone ducking",
        Some(
            "Lower the apps chosen below while you talk. Only the microphone level is read; nothing is recorded. It works while an app is using the microphone.",
        ),
    );
    if !mic.enabled {
        return;
    }
    let field = |ui: &mut Ui, label: &str, value: &mut u16, range: std::ops::RangeInclusive<u16>, suffix: &str| {
        labeled(ui, &look, label, |ui| {
            let drag = ui.add(egui::DragValue::new(value).range(range).suffix(suffix));
            widgets::set_name(ui, &drag, label);
            widgets::scroll_on_focus(drag);
        });
    };
    let mut threshold = u16::from(mic.threshold_percent);
    field(ui, "Speech level", &mut threshold, 1..=60, "%");
    mic.threshold_percent = threshold as u8;
    field(ui, "Start after", &mut mic.attack_ms, 20..=2000, " ms");
    field(ui, "Restore after silence of", &mut mic.release_ms, 100..=5000, " ms");
    let mut keep = u16::from(mic.keep_percent);
    field(ui, "Lowered apps keep", &mut keep, 0..=90, "%");
    mic.keep_percent = keep as u8;
    if s.pinned_apps.is_empty() {
        ui.label(
            RichText::new("Pin apps in the Mixer to choose which ones are lowered.").small().color(look.p.text_muted),
        );
    }
    for pin in &s.pinned_apps {
        let mut chosen = mic.targets.contains(&pin.key);
        let check = widgets::scroll_on_focus(ui.checkbox(&mut chosen, format!("Lower {} while you talk", pin.name)));
        if check.changed() {
            if chosen {
                mic.targets.push(pin.key.clone());
            } else {
                mic.targets.retain(|k| *k != pin.key);
            }
        }
    }
}

/// Backup and restore: export the preferences to a file, or check a file and import it.
/// Nothing is replaced without the user confirming what the file holds.
fn backup(ui: &mut Ui, cx: &mut SettingsCtx, outcome: &mut Outcome) {
    let look = *cx.look;
    ui.add(
        egui::Label::new(
            RichText::new(
                "Export your preferences (not the window position) to a file, and import them on this or another PC. An import is checked first and shows what it contains, and your current settings are saved to a backup file before they are replaced.",
            )
            .small()
            .color(look.p.text_muted),
        )
        .wrap(),
    );
    if widgets::scroll_on_focus(ui.button("Export settings")).clicked() {
        *outcome = Outcome::ExportSettings;
    }
    ui.horizontal(|ui| {
        let label = ui.label("Import from");
        let width = (ui.available_width() - 90.0).clamp(80.0, 260.0);
        let field = ui.add(
            egui::TextEdit::singleline(&mut cx.transfer.path)
                .hint_text("Path of an exported .json file")
                .desired_width(width),
        );
        widgets::set_name(ui, &field, "Path of the settings file to import");
        widgets::scroll_on_focus(field).labelled_by(label.id);
        let ready = !cx.transfer.path.trim().is_empty();
        if widgets::scroll_on_focus(ui.add_enabled(ready, egui::Button::new("Check file"))).clicked() {
            *outcome = Outcome::CheckImport;
        }
    });
    if let Some(error) = &cx.transfer.error {
        ui.add(egui::Label::new(RichText::new(error).color(look.p.danger)).wrap());
    }
    let mut cancel = false;
    if let Some((_, lines)) = &cx.transfer.candidate {
        ui.label(RichText::new("This file contains").font(semibold(13.0)));
        for line in lines {
            ui.add(egui::Label::new(RichText::new(line).small()).wrap());
        }
        ui.horizontal_wrapped(|ui| {
            if widgets::scroll_on_focus(ui.button("Import and replace my settings")).clicked() {
                *outcome = Outcome::ApplyImport;
            }
            cancel = widgets::scroll_on_focus(ui.button("Cancel")).clicked();
        });
    }
    if cancel {
        cx.transfer.candidate = None;
    }
}

/// Windows can only reset every app's device choice at once, so this is a
/// separate, confirmed action; a single app is reset from its row in the Mixer.
fn app_device_reset(ui: &mut Ui, cx: &mut SettingsCtx) {
    let look = *cx.look;
    if !*cx.confirm_route_reset {
        if widgets::scroll_on_focus(ui.button("Reset all app devices…"))
            .on_hover_text("Make every app follow the system default output and input again")
            .clicked()
        {
            *cx.confirm_route_reset = true;
        }
        return;
    }
    ui.add(
        egui::Label::new(
            RichText::new(
                "This resets the device choice of every app on this PC, including choices made in Windows Settings. To reset one app, choose System default in its row instead.",
            )
            .color(look.p.text),
        )
        .wrap(),
    );
    ui.horizontal_wrapped(|ui| {
        let confirm = widgets::scroll_on_focus(ui.button("Reset all apps"));
        widgets::set_name(ui, &confirm, "Confirm: reset the device choice of all apps");
        if confirm.clicked() {
            cx.audio.send(Command::ResetAllAppDevices);
            *cx.confirm_route_reset = false;
        }
        if widgets::scroll_on_focus(ui.button("Cancel")).clicked() {
            *cx.confirm_route_reset = false;
        }
    });
}

fn about(ui: &mut Ui, cx: &mut SettingsCtx, outcome: &mut Outcome) {
    let look = *cx.look;
    ui.label(RichText::new(format!("Glass Mixer {}", env!("CARGO_PKG_VERSION"))).font(semibold(14.0)));
    ui.label(
        RichText::new("Works offline. No telemetry, analytics or network access.").small().color(look.p.text_muted),
    );
    ui.add(
        egui::Label::new(
            RichText::new(
                "For a bug report: a diagnostic report lists versions, states, counts and recent warnings. App, device and profile names and your user folder are hidden, and nothing is sent anywhere.",
            )
            .small()
            .color(look.p.text_muted),
        )
        .wrap(),
    );
    ui.horizontal_wrapped(|ui| {
        if widgets::scroll_on_focus(ui.button("Save diagnostic report")).clicked() {
            *outcome = Outcome::SaveDiagnostics;
        }
        if widgets::scroll_on_focus(ui.button("Copy report")).clicked() {
            *outcome = Outcome::CopyDiagnostics;
        }
    });
    let dir = cx.settings_path.parent().map(Path::to_path_buf);
    // A long profile path must wrap inside the card, not run past the window edge.
    ui.add(
        egui::Label::new(RichText::new(cx.settings_path.display().to_string()).small().color(look.p.text_muted)).wrap(),
    );
    if let Some(dir) = dir
        && widgets::scroll_on_focus(ui.button("Open settings folder")).clicked()
    {
        let _ = std::fs::create_dir_all(&dir);
        if let Err(e) = crate::platform::window::open_folder(&dir) {
            cx.errors.push(format!("Couldn't open the folder: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::hotkeys::{HotkeyAction, HotkeyBinding};
    use crate::settings::MIN_WIDTH;
    use crate::ui::test_support::{Harness, past_right_edge, unnamed};
    use std::time::{Duration, Instant};

    /// Renders Settings at `window_width`, with one failed shortcut binding so the
    /// "Not active: <reason>" line (the longest text in the editor) is present.
    fn render(window_width: f32) -> (f32, crate::ui::test_support::Frame) {
        let harness = Harness::new(Density::Comfortable);
        let look = harness.look;
        let width = window_width - 2.0 * look.m.pad;
        let service = HotkeyService::start_with(|_| {}, harness.ctx.clone()).unwrap();
        let mut settings = Settings {
            hotkeys: vec![HotkeyBinding {
                action: HotkeyAction::ToggleOutputMute,
                chord: "Ctrl+Alt+nonsense".into(),
                target: None,
            }],
            ..Default::default()
        };
        service.configure(&settings.hotkeys, settings.hotkey_step).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while service.status().pending || service.status().failed.is_empty() {
            assert!(Instant::now() < deadline, "the shortcut service never reported: {:?}", service.status());
            std::thread::sleep(Duration::from_millis(5));
        }
        let (audio, mut icons, mut editor) =
            (AudioHandle::detached(), IconService::detached(), HotkeyEditor::default());
        let (mut login, mut errors) = (LoginState::Disabled, Vec::new());
        let path = std::path::PathBuf::from(r"C:\Users\Someone\AppData\Roaming\GlassMixer\settings.json");
        let frame = harness.run(width, 3000.0, |ui| {
            let mut cx = SettingsCtx {
                look: &look,
                settings: &mut settings,
                login: &mut login,
                glass: GlassStatus::TransparencyOff,
                support: Support::Full,
                software_renderer: false,
                can_switch_default: true,
                can_route_apps: true,
                confirm_route_reset: &mut false,
                transfer: &mut TransferUi::default(),
                audio: &audio,
                icons: &mut icons,
                settings_path: &path,
                hotkeys: Some(&service),
                hotkey_editor: &mut editor,
                errors: &mut errors,
            };
            show(ui, &mut cx);
        });
        (width, frame)
    }

    #[test]
    fn resetting_every_app_device_needs_a_confirmation_that_fits() {
        let harness = crate::ui::test_support::Harness::new(Density::Comfortable);
        let look = harness.look;
        let width = MIN_WIDTH - 2.0 * look.m.pad;
        let (audio, mut icons, mut editor) =
            (AudioHandle::detached(), IconService::detached(), HotkeyEditor::default());
        let (mut login, mut errors, mut settings) = (LoginState::Disabled, Vec::new(), Settings::default());
        let path = std::path::PathBuf::from(r"C:\settings.json");
        let mut confirm = false;
        let mut render = |confirm: &mut bool, settings: &mut Settings| {
            harness.run(width, 3000.0, |ui| {
                let mut cx = SettingsCtx {
                    look: &look,
                    settings,
                    login: &mut login,
                    glass: GlassStatus::TransparencyOff,
                    support: Support::Full,
                    software_renderer: false,
                    can_switch_default: true,
                    can_route_apps: true,
                    confirm_route_reset: &mut *confirm,
                    transfer: &mut TransferUi::default(),
                    audio: &audio,
                    icons: &mut icons,
                    settings_path: &path,
                    hotkeys: None,
                    hotkey_editor: &mut editor,
                    errors: &mut errors,
                };
                show(ui, &mut cx);
            })
        };
        let names = |f: &crate::ui::test_support::Frame| f.widgets.iter().map(|w| w.name.clone()).collect::<Vec<_>>();
        let first = render(&mut confirm, &mut settings);
        assert!(names(&first).iter().any(|n| n == "Reset all app devices…"));
        assert!(!names(&first).iter().any(|n| n.starts_with("Confirm")), "nothing is reset without a second step");
        confirm = true;
        settings.mic_ducking.enabled = true;
        settings.pinned_apps.push(crate::audio::preferences::PinnedApp {
            key: crate::audio::AppKey::Exe("music.exe".into()),
            name: "Music".into(),
            exe_path: None,
            aumid: None,
        });
        let second = render(&mut confirm, &mut settings);
        for expected in [
            "Speech level",
            "Start after",
            "Restore after silence of",
            "Lowered apps keep",
            "Lower Music while you talk",
        ] {
            assert!(names(&second).iter().any(|n| n == expected), "missing {expected}: {:?}", names(&second));
        }
        assert!(
            names(&second).iter().any(|n| n == "Confirm: reset the device choice of all apps"),
            "{:?}",
            names(&second)
        );
        let clipped = crate::ui::test_support::past_right_edge(&second.widgets, width);
        assert!(clipped.is_empty(), "{clipped:?}");
    }

    #[test]
    fn the_diagnostic_buttons_ask_for_a_saved_or_copied_report() {
        let harness = crate::ui::test_support::Harness::new(Density::Comfortable);
        let look = harness.look;
        let width = MIN_WIDTH - 2.0 * look.m.pad;
        let (audio, mut icons, mut editor) =
            (AudioHandle::detached(), IconService::detached(), HotkeyEditor::default());
        let (mut login, mut errors, mut settings) = (LoginState::Disabled, Vec::new(), Settings::default());
        let path = std::path::PathBuf::from(r"C:\settings.json");
        let seen = std::cell::RefCell::new(Vec::<&str>::new());
        let mut render = |events: Vec<egui::Event>| {
            harness.run_with_events(width, 3000.0, events, |ui| {
                let mut cx = SettingsCtx {
                    look: &look,
                    settings: &mut settings,
                    login: &mut login,
                    glass: GlassStatus::TransparencyOff,
                    support: Support::Full,
                    software_renderer: false,
                    can_switch_default: true,
                    can_route_apps: true,
                    confirm_route_reset: &mut false,
                    transfer: &mut TransferUi::default(),
                    audio: &audio,
                    icons: &mut icons,
                    settings_path: &path,
                    hotkeys: None,
                    hotkey_editor: &mut editor,
                    errors: &mut errors,
                };
                match show(ui, &mut cx) {
                    Outcome::SaveDiagnostics => seen.borrow_mut().push("save"),
                    Outcome::CopyDiagnostics => seen.borrow_mut().push("copy"),
                    Outcome::Back => seen.borrow_mut().push("back"),
                    Outcome::ExportSettings | Outcome::CheckImport | Outcome::ApplyImport => {
                        seen.borrow_mut().push("transfer")
                    }
                    Outcome::None => {}
                }
            })
        };
        let frame = render(Vec::new());
        assert!(unnamed(&frame.widgets).is_empty());
        let click = |name: &str, frame: &crate::ui::test_support::Frame| {
            let bounds = frame.widgets.iter().find(|w| w.name == name).and_then(|w| w.bounds).expect(name);
            bounds.center()
        };
        for (name, expected) in [("Save diagnostic report", "save"), ("Copy report", "copy")] {
            let pos = click(name, &frame);
            for pressed in [true, false] {
                render(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            assert_eq!(seen.borrow().last().copied(), Some(expected), "clicking {name}: {:?}", seen.borrow());
        }
        assert_eq!(*seen.borrow(), ["save", "copy"], "nothing else was triggered");
    }

    #[test]
    fn the_backup_section_exports_checks_confirms_and_cancels_with_named_buttons_that_fit() {
        let harness = crate::ui::test_support::Harness::new(Density::Comfortable);
        let look = harness.look;
        let width = MIN_WIDTH - 2.0 * look.m.pad;
        let (audio, mut icons, mut editor) =
            (AudioHandle::detached(), IconService::detached(), HotkeyEditor::default());
        let (mut login, mut errors, mut settings) = (LoginState::Disabled, Vec::new(), Settings::default());
        let path = std::path::PathBuf::from(r"C:\settings.json");
        let transfer = std::cell::RefCell::new(TransferUi {
            path: r"C:\Users\Someone\Documents\glass-mixer-settings-20261106-120000.json".into(),
            candidate: Some((
                Settings::default(),
                vec![
                    "2 pinned app(s), 1 renamed app(s), 1 pinned output(s)".into(),
                    "Not read: wheel_step: the value isn't valid, so the default is used.".into(),
                ],
            )),
            error: Some("The file isn't valid JSON (expected value at line 1 column 1).".into()),
        });
        let seen = std::cell::RefCell::new(Vec::<&str>::new());
        let mut render = |events: Vec<egui::Event>| {
            harness.run_with_events(width, 3000.0, events, |ui| {
                let mut cx = SettingsCtx {
                    look: &look,
                    settings: &mut settings,
                    login: &mut login,
                    glass: GlassStatus::TransparencyOff,
                    support: Support::Full,
                    software_renderer: false,
                    can_switch_default: true,
                    can_route_apps: true,
                    confirm_route_reset: &mut false,
                    transfer: &mut transfer.borrow_mut(),
                    audio: &audio,
                    icons: &mut icons,
                    settings_path: &path,
                    hotkeys: None,
                    hotkey_editor: &mut editor,
                    errors: &mut errors,
                };
                match show(ui, &mut cx) {
                    Outcome::ExportSettings => seen.borrow_mut().push("export"),
                    Outcome::CheckImport => seen.borrow_mut().push("check"),
                    Outcome::ApplyImport => seen.borrow_mut().push("apply"),
                    _ => {}
                }
            })
        };
        let frame = render(Vec::new());
        assert!(unnamed(&frame.widgets).is_empty(), "{:?}", unnamed(&frame.widgets));
        let names: Vec<&str> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        for expected in [
            "Export settings",
            "Check file",
            "Path of the settings file to import",
            "Import and replace my settings",
            "Cancel",
        ] {
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
        // The text inside the path field is clipped by the field; only real controls must stay inside.
        let outside: Vec<_> = past_right_edge(&frame.widgets, width)
            .into_iter()
            .filter(|w| w.role != egui::accesskit::Role::TextRun)
            .collect();
        assert!(outside.is_empty(), "{outside:?}");
        assert!(frame.content_right <= width + 0.5, "content reaches {}", frame.content_right);

        let press = |render: &mut dyn FnMut(Vec<egui::Event>) -> crate::ui::test_support::Frame, pos: egui::Pos2| {
            for pressed in [true, false] {
                render(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
        };
        let centre =
            |name: &str| frame.widgets.iter().find(|w| w.name == name).and_then(|w| w.bounds).expect(name).center();
        for (name, expected) in
            [("Export settings", "export"), ("Check file", "check"), ("Import and replace my settings", "apply")]
        {
            press(&mut render, centre(name));
            assert_eq!(seen.borrow().last().copied(), Some(expected), "clicking {name}: {:?}", seen.borrow());
        }
        assert!(transfer.borrow().candidate.is_some(), "only Cancel discards a checked import");
        press(&mut render, centre("Cancel"));
        assert!(transfer.borrow().candidate.is_none(), "Cancel discards it");
        assert_eq!(*seen.borrow(), ["export", "check", "apply"], "nothing else was triggered");
    }

    #[test]
    fn settings_controls_are_named_and_the_shortcut_editor_fits_at_minimum_width() {
        let (width, frame) = render(MIN_WIDTH);
        let nameless: Vec<_> = unnamed(&frame.widgets).iter().map(|w| format!("{:?} {:?}", w.role, w.bounds)).collect();
        assert!(nameless.is_empty(), "controls without an accessible name: {nameless:#?}");
        let clipped: Vec<_> = past_right_edge(&frame.widgets, width)
            .iter()
            .map(|w| format!("{:?} '{}' {:?}", w.role, w.name, w.bounds))
            .collect();
        assert!(clipped.is_empty(), "controls past the right edge ({width}): {clipped:#?}");
        let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        for expected in ["Volume step", "Bind Toggle output mute", "Clear Toggle output mute", "Retry failed shortcuts"]
        {
            assert!(names.iter().any(|n| n.starts_with(expected)), "missing '{expected}' in {names:?}");
        }
        assert!(
            frame.content_right <= width + 0.5,
            "settings content reaches {} in a {width}-wide window",
            frame.content_right
        );
    }

    #[test]
    fn toggle_descriptions_and_the_glass_status_are_exposed_to_assistive_technology() {
        let (_, frame) = render(480.0);
        // The status text explaining why glass is off is only painted; it must also be a description.
        let glass = frame.widgets.iter().find(|w| w.name == "Glass effect").expect("Glass effect toggle");
        assert!(glass.has_description, "the Glass effect toggle has no description for screen readers");
        let meters = frame.widgets.iter().find(|w| w.name == "Level meters").expect("Level meters toggle");
        assert!(meters.has_description);
    }

    #[test]
    fn focused_offscreen_controls_scroll_into_the_settings_viewport() {
        use crate::ui::test_support::is_interactive;

        for density in [Density::Comfortable, Density::Compact] {
            for (window_width, height) in [(MIN_WIDTH, 440.0), (480.0, 720.0)] {
                let harness = Harness::new(density);
                let look = harness.look;
                let width = window_width - 2.0 * look.m.pad;
                let mut service = HotkeyService::start_with(|_| {}, harness.ctx.clone()).unwrap();
                let (audio, mut icons, mut editor) =
                    (AudioHandle::detached(), IconService::detached(), HotkeyEditor::default());
                let (mut settings, mut login, mut errors) = (Settings::default(), LoginState::Disabled, Vec::new());
                let path = std::path::PathBuf::from(r"C:\Users\Someone\AppData\Roaming\GlassMixer\settings.json");

                let initial = harness.run(width, height, |ui| {
                    let mut cx = SettingsCtx {
                        look: &look,
                        settings: &mut settings,
                        login: &mut login,
                        glass: GlassStatus::TransparencyOff,
                        support: Support::Full,
                        software_renderer: false,
                        can_switch_default: true,
                        can_route_apps: true,
                        confirm_route_reset: &mut false,
                        transfer: &mut TransferUi::default(),
                        audio: &audio,
                        icons: &mut icons,
                        settings_path: &path,
                        hotkeys: Some(&service),
                        hotkey_editor: &mut editor,
                        errors: &mut errors,
                    };
                    show(ui, &mut cx);
                });
                let initial_bottom = initial
                    .widgets
                    .iter()
                    .find(|w| w.name == "Open settings folder")
                    .and_then(|w| w.bounds)
                    .expect("Open settings folder bounds")
                    .max
                    .y;
                assert!(
                    initial_bottom > height,
                    "expected Open settings folder to start off-screen at {width}x{height} ({density:?}, y={initial_bottom})"
                );

                // Tab forward to Open settings folder; every focused control must stay inside the viewport.
                let mut reached_open_folder = false;
                for _ in 0..90 {
                    let frame = harness.send_key(width, height, Key::Tab, egui::Modifiers::NONE, |ui| {
                        let mut cx = SettingsCtx {
                            look: &look,
                            settings: &mut settings,
                            login: &mut login,
                            glass: GlassStatus::TransparencyOff,
                            support: Support::Full,
                            software_renderer: false,
                            can_switch_default: true,
                            can_route_apps: true,
                            confirm_route_reset: &mut false,
                            transfer: &mut TransferUi::default(),
                            audio: &audio,
                            icons: &mut icons,
                            settings_path: &path,
                            hotkeys: Some(&service),
                            hotkey_editor: &mut editor,
                            errors: &mut errors,
                        };
                        show(ui, &mut cx);
                    });
                    if let Some(widget) = frame.widgets.iter().find(|w| w.focused && is_interactive(w.role)) {
                        if let Some(b) = widget.bounds {
                            assert!(
                                b.min.y >= -1.0 && b.max.y <= height + 1.0,
                                "Settings focused control '{}' off-screen at {b:?} in {width}x{height} ({density:?})",
                                widget.name
                            );
                        }
                        if widget.name == "Open settings folder" {
                            reached_open_folder = true;
                            break;
                        }
                    }
                }
                assert!(reached_open_folder, "Tab traversal in Settings must reach Open settings folder");

                // While Open settings folder remains focused, scrolling the wheel upward must not be
                // snapped back on subsequent frames because scroll_on_focus uses gained_focus().
                let before_wheel = harness
                    .run(width, height, |ui| {
                        let mut cx = SettingsCtx {
                            look: &look,
                            settings: &mut settings,
                            login: &mut login,
                            glass: GlassStatus::TransparencyOff,
                            support: Support::Full,
                            software_renderer: false,
                            can_switch_default: true,
                            can_route_apps: true,
                            confirm_route_reset: &mut false,
                            transfer: &mut TransferUi::default(),
                            audio: &audio,
                            icons: &mut icons,
                            settings_path: &path,
                            hotkeys: Some(&service),
                            hotkey_editor: &mut editor,
                            errors: &mut errors,
                        };
                        show(ui, &mut cx);
                    })
                    .widgets
                    .into_iter()
                    .find(|w| w.name == "Open settings folder")
                    .and_then(|w| w.bounds)
                    .unwrap()
                    .min
                    .y;
                let after_wheel = harness
                    .run_with_events(
                        width,
                        height,
                        vec![
                            egui::Event::PointerMoved(egui::pos2(width * 0.5, height * 0.5)),
                            egui::Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Point,
                                delta: egui::vec2(0.0, 120.0),
                                modifiers: egui::Modifiers::NONE,
                                phase: egui::TouchPhase::Move,
                            },
                        ],
                        |ui| {
                            let mut cx = SettingsCtx {
                                look: &look,
                                settings: &mut settings,
                                login: &mut login,
                                glass: GlassStatus::TransparencyOff,
                                support: Support::Full,
                                software_renderer: false,
                                can_switch_default: true,
                                can_route_apps: true,
                                confirm_route_reset: &mut false,
                                transfer: &mut TransferUi::default(),
                                audio: &audio,
                                icons: &mut icons,
                                settings_path: &path,
                                hotkeys: Some(&service),
                                hotkey_editor: &mut editor,
                                errors: &mut errors,
                            };
                            show(ui, &mut cx);
                        },
                    )
                    .widgets
                    .into_iter()
                    .find(|w| w.name == "Open settings folder")
                    .and_then(|w| w.bounds)
                    .unwrap()
                    .min
                    .y;
                assert!(
                    after_wheel > before_wheel + 20.0,
                    "manual wheel scroll while focused must not be fought by scroll_on_focus (before={before_wheel}, after={after_wheel})"
                );

                // Let the wheel's smooth-scroll animation finish (the harness advances 10 ms per
                // pass), so the first Shift+Tab is judged on focus scrolling alone.
                for _ in 0..30 {
                    harness.run(width, height, |ui| {
                        let mut cx = SettingsCtx {
                            look: &look,
                            settings: &mut settings,
                            login: &mut login,
                            glass: GlassStatus::TransparencyOff,
                            support: Support::Full,
                            software_renderer: false,
                            can_switch_default: true,
                            can_route_apps: true,
                            confirm_route_reset: &mut false,
                            transfer: &mut TransferUi::default(),
                            audio: &audio,
                            icons: &mut icons,
                            settings_path: &path,
                            hotkeys: Some(&service),
                            hotkey_editor: &mut editor,
                            errors: &mut errors,
                        };
                        show(ui, &mut cx);
                    });
                }

                // Shift+Tab backward all the way to the top Theme control.
                let mut reached_system_theme = false;
                for _ in 0..90 {
                    let frame = harness.send_key(width, height, Key::Tab, egui::Modifiers::SHIFT, |ui| {
                        let mut cx = SettingsCtx {
                            look: &look,
                            settings: &mut settings,
                            login: &mut login,
                            glass: GlassStatus::TransparencyOff,
                            support: Support::Full,
                            software_renderer: false,
                            can_switch_default: true,
                            can_route_apps: true,
                            confirm_route_reset: &mut false,
                            transfer: &mut TransferUi::default(),
                            audio: &audio,
                            icons: &mut icons,
                            settings_path: &path,
                            hotkeys: Some(&service),
                            hotkey_editor: &mut editor,
                            errors: &mut errors,
                        };
                        show(ui, &mut cx);
                    });
                    if let Some(widget) = frame.widgets.iter().find(|w| w.focused && is_interactive(w.role)) {
                        if let Some(b) = widget.bounds {
                            assert!(
                                b.min.y >= -1.0 && b.max.y <= height + 1.0,
                                "Settings Shift+Tab focused control '{}' off-screen at {b:?} in {width}x{height} ({density:?})",
                                widget.name
                            );
                        }
                        if widget.name == "System" {
                            reached_system_theme = true;
                            break;
                        }
                    }
                }
                assert!(reached_system_theme, "Shift+Tab in Settings must return to the top System theme button");
                service.shutdown();
            }
        }
    }
}
