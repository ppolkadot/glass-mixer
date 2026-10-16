//! Application lifecycle: wires the audio engine, tray, backdrop, settings and
//! views together, and owns the single shutdown path.

use crate::audio::{AudioHandle, NoticeLevel};
use crate::platform::backdrop::{self, Applied, Support};
use crate::platform::fonts;
use crate::platform::hotkeys::{HotkeyEditor, HotkeyService};
use crate::platform::icons::IconService;
use crate::platform::startup::{self, LoginState};
use crate::platform::sysprefs::SystemPrefs;
use crate::platform::tray::{Tray, TrayAction};
use crate::platform::window::{self, Hwnd};
use crate::settings::{Backdrop, Settings, SettingsStore, ThemeMode, WindowGeometry};
use crate::ui::{GlassStatus, Look, mixer, settings_view, theme};
use crossbeam_channel::Receiver;
use egui::{Align2, Color32, CornerRadius, Margin, RichText, ViewportCommand};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::time::{Duration, Instant};

const SAVE_DEBOUNCE: Duration = Duration::from_millis(800);
const PREFS_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct StartOptions {
    pub store: SettingsStore,
    pub settings: Settings,
    pub load_error: Option<String>,
    /// Start with only the tray icon.
    pub hidden: bool,
    pub restored_position: Option<(i32, i32)>,
    /// The fallback renderer uses a solid surface when OpenGL initialization fails.
    pub force_opaque: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Mixer,
    Devices,
    Presets,
    Settings,
}

struct Toast {
    level: NoticeLevel,
    text: String,
    until: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BackdropRetryKey {
    glass: bool,
    backdrop: Backdrop,
    theme: ThemeMode,
    dark: bool,
    high_contrast: bool,
    build: u32,
}

#[derive(Default)]
struct BackdropRejection {
    failed_for: Option<BackdropRetryKey>,
}

impl BackdropRejection {
    fn reset_for(&mut self, key: BackdropRetryKey) {
        if self.failed_for.is_some_and(|failed| failed != key) {
            self.failed_for = None;
        }
    }

    fn may_try(&self, key: BackdropRetryKey) -> bool {
        self.failed_for != Some(key)
    }

    fn record(&mut self, key: BackdropRetryKey, applied: Applied) {
        if !applied.glass {
            self.failed_for = Some(key);
        }
    }
}

pub struct MixerApp {
    audio: AudioHandle,
    icons: IconService,
    hotkeys: Option<HotkeyService>,
    hotkey_editor: HotkeyEditor,
    preset_editor: crate::ui::presets::PresetEditor,
    last_hotkey_errors: Vec<String>,
    store: SettingsStore,
    settings: Settings,
    dirty_since: Option<Instant>,
    prefs: SystemPrefs,
    prefs_checked: Instant,
    support: Support,
    backdrop_key: Option<(Option<Backdrop>, bool)>,
    applied: Option<Applied>,
    backdrop_rejection: BackdropRejection,
    hwnd: Option<Hwnd>,
    tray: Option<Tray>,
    tray_rx: Receiver<TrayAction>,
    view: View,
    mixer: mixer::MixerState,
    login: LoginState,
    toasts: Vec<Toast>,
    exiting: bool,
    has_icon_font: bool,
    look: Look,
    style_key: Option<Look>,
    shut_down: bool,
    /// Started with `--minimized`: hide once eframe has shown the (cloaked) window.
    pending_start_hide: bool,
    force_opaque: bool,
    #[cfg(debug_assertions)]
    dev_exit_at: Option<Instant>,
}

impl MixerApp {
    pub fn new(cc: &eframe::CreationContext<'_>, opts: StartOptions) -> Self {
        let ctx = cc.egui_ctx.clone();
        let has_icon_font = fonts::install(&ctx);
        let hwnd = cc.window_handle().ok().and_then(|h| match h.as_raw() {
            RawWindowHandle::Win32(w) => Some(Hwnd(w.hwnd.get())),
            _ => None,
        });
        if let (Some(h), Some((x, y))) = (hwnd, opts.restored_position)
            && let Err(e) = window::set_outer_position(h, x, y)
        {
            log::warn!("couldn't restore the window position: {e}");
        }

        let audio = AudioHandle::start();
        audio.send(crate::audio::Command::SetPresetRules(opts.settings.presets.clone()));
        {
            let ctx = ctx.clone();
            audio.add_listener(move || ctx.request_repaint());
        }
        let icons = {
            let ctx = ctx.clone();
            IconService::start(move || ctx.request_repaint())
        };
        let (tray_tx, tray_rx) = crossbeam_channel::unbounded();
        let mut toasts = Vec::new();
        let tray = match hwnd {
            Some(h) => match Tray::create(h, audio.clone(), tray_tx, ctx.clone()) {
                Ok(t) => Some(t),
                Err(e) => {
                    log::error!("tray icon unavailable: {e}");
                    toasts.push(Toast {
                        level: NoticeLevel::Error,
                        text: "The tray icon couldn't be created, so closing the window will exit.".into(),
                        until: Instant::now() + Duration::from_secs(10),
                    });
                    None
                }
            },
            None => None,
        };
        // The show/hide shortcut must never hide the only way back to the UI.
        let hotkeys = if tray.is_some() {
            hwnd.and_then(|h| match HotkeyService::start(audio.clone(), h, ctx.clone()) {
                Ok(service) => {
                    // A start-hidden window is cloaked until its first frame.
                    // Registering the show/hide chord before that handoff can
                    // consume a show request while the window is still cloaked.
                    if !opts.hidden
                        && let Err(e) = service.configure(&opts.settings.hotkeys, opts.settings.hotkey_step)
                    {
                        log::warn!("global shortcut setup: {e}");
                    }
                    Some(service)
                }
                Err(e) => {
                    log::warn!("global shortcuts unavailable: {e}");
                    None
                }
            })
        } else {
            None
        };
        if let Some(err) = opts.load_error {
            toasts.push(Toast {
                level: NoticeLevel::Warning,
                text: err,
                until: Instant::now() + Duration::from_secs(10),
            });
        }
        // Never strand the user with no window and no tray icon.
        let start_hidden = opts.hidden && tray.is_some();
        if opts.hidden
            && tray.is_none()
            && let Some(h) = hwnd
        {
            window::show(h);
        }
        if start_hidden && let Some(h) = hwnd {
            window::set_cloaked(h, true);
        }

        let prefs = SystemPrefs::read();
        let support = Support::for_build(prefs.build);
        log::info!(
            "Windows build {}, backdrop support {:?}, dark={}, transparency={}, animations={}, high contrast={}",
            prefs.build,
            support,
            prefs.dark,
            prefs.transparency,
            prefs.animations,
            prefs.high_contrast
        );
        let look = Look {
            p: theme::palette(true, false, opts.settings.accent, false),
            m: theme::metrics(opts.settings.density),
            icons: has_icon_font,
        };
        let mut app = Self {
            audio,
            icons,
            hotkeys,
            hotkey_editor: HotkeyEditor::default(),
            preset_editor: crate::ui::presets::PresetEditor::default(),
            last_hotkey_errors: Vec::new(),
            store: opts.store,
            settings: opts.settings,
            dirty_since: None,
            prefs,
            prefs_checked: Instant::now(),
            support,
            backdrop_key: None,
            applied: None,
            backdrop_rejection: BackdropRejection::default(),
            hwnd,
            tray,
            tray_rx,
            view: View::Mixer,
            mixer: mixer::MixerState::default(),
            login: startup::state(),
            toasts,
            exiting: false,
            has_icon_font,
            look,
            style_key: None,
            shut_down: false,
            pending_start_hide: start_hidden,
            force_opaque: opts.force_opaque,
            #[cfg(debug_assertions)]
            dev_exit_at: std::env::var_os("GLASS_MIXER_DEV_PROFILE")
                .and_then(|_| std::env::var("GLASS_MIXER_DEV_EXIT_AFTER_MS").ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(|ms| Instant::now() + Duration::from_millis(ms)),
        };
        app.refresh_look(&ctx);
        app
    }

    fn effective_dark(&self) -> bool {
        match self.settings.theme {
            ThemeMode::System => self.prefs.dark,
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        }
    }

    fn backdrop_retry_key(&self) -> BackdropRetryKey {
        BackdropRetryKey {
            glass: self.settings.glass,
            backdrop: self.settings.backdrop,
            theme: self.settings.theme,
            dark: self.prefs.dark,
            high_contrast: self.prefs.high_contrast,
            build: self.prefs.build,
        }
    }

    fn glass_status(&self) -> GlassStatus {
        if !self.settings.glass {
            GlassStatus::TurnedOff
        } else if self.force_opaque {
            GlassStatus::RendererFallback
        } else if self.prefs.high_contrast {
            GlassStatus::HighContrast
        } else if !self.prefs.transparency {
            GlassStatus::TransparencyOff
        } else if self.support == Support::None || !self.backdrop_rejection.may_try(self.backdrop_retry_key()) {
            GlassStatus::Unsupported
        } else {
            GlassStatus::Active
        }
    }

    /// Applies the backdrop and egui style when anything affecting them changed.
    fn refresh_look(&mut self, ctx: &egui::Context) {
        self.backdrop_rejection.reset_for(self.backdrop_retry_key());
        let dark = self.effective_dark() || self.prefs.high_contrast;
        let want_glass = matches!(self.glass_status(), GlassStatus::Active);
        let requested = want_glass.then_some(self.settings.backdrop);
        if let Some(h) = self.hwnd
            && self.backdrop_key != Some((requested, dark))
        {
            let applied = backdrop::apply(h, self.support, requested, dark);
            log::info!("backdrop applied: {applied:?}");
            if requested.is_some() {
                self.backdrop_rejection.record(self.backdrop_retry_key(), applied);
            }
            self.applied = Some(applied);
            self.backdrop_key = Some((requested, dark));
        }
        let glass = self.applied.is_some_and(|a| a.glass);
        let look = Look {
            p: theme::palette(dark, glass, self.settings.accent, self.prefs.high_contrast),
            m: theme::metrics(self.settings.density),
            icons: self.has_icon_font,
        };
        if self.style_key != Some(look) {
            theme::apply(ctx, &look.p, &look.m, self.prefs.animations);
            self.style_key = Some(look);
        }
        self.look = look;
    }

    fn mark_dirty(&mut self) {
        if self.dirty_since.is_none() {
            self.dirty_since = Some(Instant::now());
        }
    }

    fn save_if_due(&mut self, ctx: &egui::Context, force: bool) {
        let Some(since) = self.dirty_since else { return };
        let due = since + SAVE_DEBOUNCE;
        if !force && Instant::now() < due {
            ctx.request_repaint_after(due - Instant::now());
            return;
        }
        self.dirty_since = None;
        if let Err(e) = self.store.save(&self.settings) {
            log::error!("saving settings failed: {e}");
            self.toast(NoticeLevel::Error, format!("Couldn't save settings: {e}"));
        }
    }

    fn toast(&mut self, level: NoticeLevel, text: String) {
        if self.settings.hidden_notifications
            && self.hwnd.is_some_and(|h| !window::is_shown(h))
            && let Some(tray) = self.tray.as_mut()
        {
            tray.notify(level, &text);
        }
        let secs = match level {
            NoticeLevel::Info => 4,
            NoticeLevel::Warning => 7,
            NoticeLevel::Error => 9,
        };
        self.toasts.retain(|t| t.text != text);
        self.toasts.push(Toast { level, text, until: Instant::now() + Duration::from_secs(secs) });
        while self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }

    fn hide_to_tray(&mut self) {
        if let Some(h) = self.hwnd {
            window::hide(h);
        }
        self.audio.set_meters_wanted(false);
    }

    fn request_exit(&mut self, ctx: &egui::Context) {
        log::info!("exit requested");
        self.exiting = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
        ctx.request_repaint();
    }

    /// Single shutdown path: persist settings, remove the tray icon, stop threads.
    fn shutdown(&mut self) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;
        if self.dirty_since.take().is_some()
            && let Err(e) = self.store.save(&self.settings)
        {
            log::error!("saving settings on exit failed: {e}");
        }
        self.tray = None;
        self.hotkeys = None;
        self.audio.shutdown();
        self.icons.shutdown();
        log::info!("shutdown complete");
    }

    fn track_window(&mut self, ctx: &egui::Context, shown: bool) {
        if !shown {
            return;
        }
        let rects = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        if let (Some(_outer), Some(inner), Some(h)) = (rects.0, rects.1, self.hwnd)
            && let Some((x, y)) = window::outer_position(h)
        {
            let g = WindowGeometry { x, y, width: inner.width(), height: inner.height() };
            let changed = !self.settings.window_position_physical
                || self.settings.window.is_none_or(|old| {
                    (old.x - g.x).abs() > 0.5
                        || (old.y - g.y).abs() > 0.5
                        || (old.width - g.width).abs() > 0.5
                        || (old.height - g.height).abs() > 0.5
                });
            if changed && g.width > 0.0 && g.height > 0.0 {
                self.settings.window = Some(g);
                self.settings.window_position_physical = true;
                self.mark_dirty();
            }
        }
    }

    fn draw_toasts(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        self.toasts.retain(|t| t.until > now);
        if let Some(next) = self.toasts.iter().map(|t| t.until).min() {
            ui.ctx().request_repaint_after(next - now);
        }
        if self.toasts.is_empty() {
            return;
        }
        let p = self.look.p;
        let mut dismiss = None;
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(Align2::CENTER_BOTTOM, [0.0, -12.0])
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.set_max_width(360.0);
                for (i, t) in self.toasts.iter().enumerate() {
                    let accent = match t.level {
                        NoticeLevel::Info => p.accent,
                        NoticeLevel::Warning => Color32::from_rgb(0xf5, 0x9e, 0x0b),
                        NoticeLevel::Error => p.danger,
                    };
                    let prefix = match t.level {
                        NoticeLevel::Info => "",
                        NoticeLevel::Warning => "Warning: ",
                        NoticeLevel::Error => "Error: ",
                    };
                    let resp = egui::Frame::NONE
                        .fill(p.surface_strong)
                        .stroke(egui::Stroke::new(1.0, accent))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(12, 8))
                        .shadow(egui::Shadow {
                            offset: [0, 4],
                            blur: 14,
                            spread: 0,
                            color: Color32::from_black_alpha(80),
                        })
                        .show(ui, |ui| {
                            ui.set_width(336.0);
                            ui.label(RichText::new(format!("{prefix}{}", t.text)).color(p.text));
                        })
                        .response
                        .interact(egui::Sense::click())
                        .on_hover_text("Click to dismiss");
                    if resp.clicked() {
                        dismiss = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = dismiss {
            self.toasts.remove(i);
        }
    }
}

impl eframe::App for MixerApp {
    /// Runs before every frame and also while the window is hidden (on repaint
    /// requests from the audio engine, tray or icon threads).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(debug_assertions)]
        if let Some(deadline) = self.dev_exit_at {
            if Instant::now() >= deadline {
                self.dev_exit_at = None;
                self.request_exit(ctx);
            } else {
                ctx.request_repaint_after(deadline - Instant::now());
            }
        }
        if self.pending_start_hide
            && let Some(h) = self.hwnd
        {
            if window::is_visible(h) {
                self.pending_start_hide = false;
                self.hide_to_tray();
                window::set_cloaked(h, false);
                if let Some(service) = &self.hotkeys
                    && let Err(e) = service.configure(&self.settings.hotkeys, self.settings.hotkey_step)
                {
                    log::warn!("global shortcut setup after hidden start: {e}");
                }
            } else {
                ctx.request_repaint();
            }
        }
        for action in self.tray_rx.try_iter().collect::<Vec<_>>() {
            match action {
                TrayAction::Exit => self.request_exit(ctx),
            }
        }

        let (close_requested, minimized) =
            ctx.input(|i| (i.viewport().close_requested(), i.viewport().minimized.unwrap_or(false)));
        if close_requested && !self.exiting {
            if self.settings.close_to_tray && self.tray.is_some() {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.hide_to_tray();
            } else {
                self.exiting = true;
            }
        }
        if minimized
            && self.settings.minimize_to_tray
            && self.tray.is_some()
            && let Some(h) = self.hwnd
            && unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(h.get()) }.as_bool()
        {
            self.hide_to_tray();
        }

        let shown = self.hwnd.is_none_or(window::is_shown);
        self.audio.set_meters_wanted(shown && self.settings.show_meters);

        let snapshot = self.audio.snapshot();
        self.icons.sync_live_apps(
            snapshot
                .apps
                .iter()
                .map(|app| app.key.clone())
                .chain(self.settings.pinned_apps.iter().map(|p| p.key.clone())),
        );
        if let Some(tray) = self.tray.as_mut() {
            tray.update(&snapshot);
        }
        for n in self.audio.take_notices() {
            self.toast(n.level, n.text);
        }
        if let Some(service) = &self.hotkeys {
            let errors = service.status().errors;
            if errors != self.last_hotkey_errors {
                self.last_hotkey_errors = errors.clone();
                for error in errors {
                    self.toast(NoticeLevel::Warning, error);
                }
            }
        }

        if shown {
            if self.prefs_checked.elapsed() >= PREFS_INTERVAL {
                self.prefs_checked = Instant::now();
                let prefs = SystemPrefs::read();
                if prefs != self.prefs {
                    log::info!("system preferences changed: {prefs:?}");
                    self.prefs = prefs;
                }
            }
            // Pick up Windows theme/transparency changes while visible; idle cost is one
            // registry read every two seconds, and nothing at all while hidden.
            ctx.request_repaint_after(PREFS_INTERVAL);
        }
        self.track_window(ctx, shown);
        self.refresh_look(ctx);
        self.save_if_due(ctx, false);

        let ready = self.icons.take_ready();
        if !ready.is_empty() {
            self.mixer.ingest_icons(ctx, ready);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let look = self.look;
        ui.painter().rect_filled(ui.max_rect(), CornerRadius::ZERO, look.p.base);
        let snapshot = self.audio.snapshot();
        let meters = self.audio.meters();
        let before = self.settings.clone();
        let mut errors = Vec::new();

        egui::Frame::NONE.inner_margin(Margin::same(look.m.pad as i8)).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.view, View::Mixer, "Mixer");
                if ui.selectable_value(&mut self.view, View::Devices, "Devices").clicked() {
                    self.audio.send(crate::audio::Command::LoadDevices);
                }
                ui.selectable_value(&mut self.view, View::Settings, "Settings");
                ui.selectable_value(&mut self.view, View::Presets, "Presets");
            });
            ui.add_space(look.m.gap);
            match self.view {
                View::Devices => {
                    let mut cx = mixer::MixerCtx {
                        audio: &self.audio,
                        snap: &snapshot,
                        meters: &meters,
                        look: &look,
                        settings: &mut self.settings,
                        icons: &mut self.icons,
                    };
                    mixer::show_devices(ui, &mut self.mixer, &mut cx);
                }
                View::Mixer => {
                    let mut cx = mixer::MixerCtx {
                        audio: &self.audio,
                        snap: &snapshot,
                        meters: &meters,
                        look: &look,
                        settings: &mut self.settings,
                        icons: &mut self.icons,
                    };
                    if let mixer::Outcome::OpenSettings = mixer::show(ui, &mut self.mixer, &mut cx) {
                        self.login = startup::state();
                        self.view = View::Settings;
                    }
                }
                View::Settings => {
                    let glass = self.glass_status();
                    let mut cx = settings_view::SettingsCtx {
                        look: &look,
                        settings: &mut self.settings,
                        login: &mut self.login,
                        glass,
                        support: self.support,
                        can_switch_default: snapshot.can_switch_default,
                        audio: &self.audio,
                        icons: &mut self.icons,
                        settings_path: self.store.path(),
                        hotkeys: self.hotkeys.as_ref(),
                        hotkey_editor: &mut self.hotkey_editor,
                        errors: &mut errors,
                    };
                    if let settings_view::Outcome::Back = settings_view::show(ui, &mut cx) {
                        self.view = View::Mixer;
                    }
                }
                View::Presets => crate::ui::presets::show(
                    ui,
                    &mut self.preset_editor,
                    &mut self.settings,
                    &snapshot,
                    &self.audio,
                    &mut errors,
                ),
            }
        });

        if self.settings != before {
            if self.settings.presets != before.presets {
                self.audio.send(crate::audio::Command::SetPresetRules(self.settings.presets.clone()));
            }
            if (self.settings.hotkeys != before.hotkeys || self.settings.hotkey_step != before.hotkey_step)
                && let Some(service) = &self.hotkeys
                && let Err(e) = service.configure(&self.settings.hotkeys, self.settings.hotkey_step)
            {
                errors.push(e);
            }
            self.mark_dirty();
            let ctx = ui.ctx().clone();
            self.refresh_look(&ctx);
            ctx.request_repaint();
        }
        for e in errors {
            self.toast(NoticeLevel::Error, e);
        }
        self.draw_toasts(ui);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.look.p.glass { [0.0, 0.0, 0.0, 0.0] } else { self.look.p.base.to_normalized_gamma_f32() }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }
}

impl Drop for MixerApp {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_backdrop_stays_rejected_until_appearance_or_build_changes() {
        let key = BackdropRetryKey {
            glass: true,
            backdrop: Backdrop::Acrylic,
            theme: ThemeMode::Dark,
            dark: true,
            high_contrast: false,
            build: 26200,
        };
        let mut rejection = BackdropRejection::default();
        assert!(rejection.may_try(key));
        rejection.record(key, Applied { glass: false, dark: true, kind: None });
        assert!(!rejection.may_try(key));
        assert!(rejection.may_try(BackdropRetryKey { backdrop: Backdrop::Mica, ..key }));
        assert!(rejection.may_try(BackdropRetryKey { theme: ThemeMode::Light, ..key }));
        assert!(rejection.may_try(BackdropRetryKey { build: 26201, ..key }));
        rejection.reset_for(BackdropRetryKey { backdrop: Backdrop::Mica, ..key });
        assert!(rejection.may_try(key), "returning to the earlier setting should retry");
    }
}
