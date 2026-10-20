//! Settings page: appearance, behaviour, audio and about.

use super::widgets::{self, card, glyph, icon_button, segmented, semibold, toggle_row};
use super::{GlassStatus, Look, theme};
use crate::audio::{AudioHandle, Command};
use crate::platform::backdrop::Support;
use crate::platform::hotkeys::{HotkeyEditor, HotkeyService};
use crate::platform::icons::IconService;
use crate::platform::startup::{self, LoginState};
use crate::settings::{Accent, Backdrop, DEFAULT_HEIGHT, DEFAULT_WIDTH, Density, Settings, ThemeMode};
use egui::{Align, Align2, Key, Layout, RichText, Sense, Stroke, Ui, Vec2, ViewportCommand};
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
    /// Frames are drawn by the CPU, so level meters are paused.
    pub software_renderer: bool,
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
            hotkeys: vec![HotkeyBinding { action: HotkeyAction::ToggleOutputMute, chord: "Ctrl+Alt+nonsense".into() }],
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
                for _ in 0..45 {
                    let frame = harness.send_key(width, height, Key::Tab, egui::Modifiers::NONE, |ui| {
                        let mut cx = SettingsCtx {
                            look: &look,
                            settings: &mut settings,
                            login: &mut login,
                            glass: GlassStatus::TransparencyOff,
                            support: Support::Full,
                            software_renderer: false,
                            can_switch_default: true,
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

                // Shift+Tab backward all the way to the top Theme control.
                let mut reached_system_theme = false;
                for _ in 0..45 {
                    let frame = harness.send_key(width, height, Key::Tab, egui::Modifiers::SHIFT, |ui| {
                        let mut cx = SettingsCtx {
                            look: &look,
                            settings: &mut settings,
                            login: &mut login,
                            glass: GlassStatus::TransparencyOff,
                            support: Support::Full,
                            software_renderer: false,
                            can_switch_default: true,
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
