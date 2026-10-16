//! Settings page: appearance, behaviour, audio and about.

use super::widgets::{self, card, glyph, icon_button, segmented, semibold, toggle_row};
use super::{GlassStatus, Look, theme};
use crate::audio::{AudioHandle, Command};
use crate::platform::backdrop::Support;
use crate::platform::hotkeys::{HotkeyEditor, HotkeyService};
use crate::platform::icons::IconService;
use crate::platform::startup::{self, LoginState};
use crate::settings::{Accent, Backdrop, DEFAULT_HEIGHT, DEFAULT_WIDTH, Density, Settings, ThemeMode};
use egui::{Align, Align2, CornerRadius, Key, Layout, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, ViewportCommand};
use std::path::Path;

pub enum Outcome {
    None,
    Back,
}

pub struct SettingsCtx<'a> {
    pub look: &'a Look,
    pub settings: &'a mut Settings,
    pub login: &'a mut LoginState,
    pub glass: GlassStatus,
    pub support: Support,
    pub can_switch_default: bool,
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
                cx.hotkey_editor.show(ui, &mut cx.settings.hotkeys, &mut cx.settings.hotkey_step, service);
            } else {
                ui.label("Global shortcuts are unavailable in this session.");
            }
        });
        section(ui, &look, "About", |ui| about(ui, cx));
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
    labeled(ui, &look, "Density", |ui| {
        segmented(
            ui,
            &look,
            &mut s.density,
            &[(Density::Comfortable, "Comfortable"), (Density::Compact, "Compact")],
            210.0,
        );
    });
    toggle_row(ui, &look, &mut s.show_meters, "Level meters", Some("Show live audio levels under the sliders"));
    if ui.button("Use recommended window size (480 × 720)").clicked() {
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
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(14),
            Stroke::new(2.0, look.p.focus),
            StrokeKind::Outside,
        );
    }
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
    ui.horizontal(|ui| {
        if ui.button("Refresh audio devices").on_hover_text("Reconnect to all devices and audio sessions").clicked() {
            cx.icons.retry_failed();
            cx.audio.send(Command::Refresh);
        }
    });
    ui.label(
        RichText::new("Live volumes come from Windows. Presets store app levels only when you save one.")
            .small()
            .color(look.p.text_muted),
    );
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

fn about(ui: &mut Ui, cx: &mut SettingsCtx) {
    let look = *cx.look;
    ui.label(RichText::new(format!("Glass Mixer {}", env!("CARGO_PKG_VERSION"))).font(semibold(14.0)));
    ui.label(
        RichText::new("Works offline. No telemetry, analytics or network access.").small().color(look.p.text_muted),
    );
    let dir = cx.settings_path.parent().map(Path::to_path_buf);
    ui.horizontal(|ui| {
        ui.label(RichText::new(cx.settings_path.display().to_string()).small().color(look.p.text_muted));
    });
    if let Some(dir) = dir
        && ui.button("Open settings folder").clicked()
    {
        let _ = std::fs::create_dir_all(&dir);
        if let Err(e) = std::process::Command::new("explorer.exe").arg(&dir).spawn() {
            cx.errors.push(format!("Couldn't open the folder: {e}"));
        }
    }
}
