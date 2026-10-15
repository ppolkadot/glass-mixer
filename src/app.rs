//! Application lifecycle: wires the audio engine, tray, backdrop, settings and
//! views together, and owns the single shutdown path.

use crate::audio::{AudioHandle, NoticeLevel};
use crate::platform::backdrop::{self, Applied, Support};
use crate::platform::fonts;
use crate::platform::icons::IconService;
use crate::platform::startup::{self, LoginState};
use crate::platform::sysprefs::SystemPrefs;
use crate::platform::tray::{Tray, TrayAction};
use crate::platform::window::{self, Hwnd};
use crate::settings::{Backdrop, Settings, SettingsStore, ThemeMode, WindowGeometry};
use crate::ui::{GlassStatus, Look, mixer, settings_view, theme};
use crossbeam_channel::Receiver;
use egui::{Align2, Color32, CornerRadius, FontId, Margin, RichText, ViewportCommand};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::time::{Duration, Instant};

const SAVE_DEBOUNCE: Duration = Duration::from_millis(800);
const PREFS_INTERVAL: Duration = Duration::from_secs(2);

pub struct StartOptions {
    pub store: SettingsStore,
    pub settings: Settings,
    pub load_error: Option<String>,
    /// Start with only the tray icon.
    pub hidden: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Mixer,
    Settings,
}

struct Toast {
    level: NoticeLevel,
    text: String,
    until: Instant,
}

pub struct MixerApp {
    audio: AudioHandle,
    icons: IconService,
    store: SettingsStore,
    settings: Settings,
    dirty_since: Option<Instant>,
    prefs: SystemPrefs,
    prefs_checked: Instant,
    support: Support,
    backdrop_key: Option<(Option<Backdrop>, bool)>,
    applied: Option<Applied>,
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
}

impl MixerApp {
    pub fn new(cc: &eframe::CreationContext<'_>, opts: StartOptions) -> Self {
        let ctx = cc.egui_ctx.clone();
        let has_icon_font = fonts::install(&ctx);
        let hwnd = cc.window_handle().ok().and_then(|h| match h.as_raw() {
            RawWindowHandle::Win32(w) => Some(Hwnd(w.hwnd.get())),
            _ => None,
        });

        let audio = AudioHandle::start();
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
            store: opts.store,
            settings: opts.settings,
            dirty_since: None,
            prefs,
            prefs_checked: Instant::now(),
            support,
            backdrop_key: None,
            applied: None,
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

    fn glass_status(&self) -> GlassStatus {
        if !self.settings.glass {
            GlassStatus::TurnedOff
        } else if self.prefs.high_contrast {
            GlassStatus::HighContrast
        } else if !self.prefs.transparency {
            GlassStatus::TransparencyOff
        } else if self.support == Support::None
            || self.applied.is_some_and(|a| !a.glass && self.backdrop_key.is_some_and(|k| k.0.is_some()))
        {
            GlassStatus::Unsupported
        } else {
            GlassStatus::Active
        }
    }

    /// Applies the backdrop and egui style when anything affecting them changed.
    fn refresh_look(&mut self, ctx: &egui::Context) {
        let dark = self.effective_dark() || self.prefs.high_contrast;
        let want_glass = matches!(self.glass_status(), GlassStatus::Active);
        let requested = want_glass.then_some(self.settings.backdrop);
        if let Some(h) = self.hwnd
            && self.backdrop_key != Some((requested, dark))
        {
            let applied = backdrop::apply(h, self.support, requested, dark);
            log::info!("backdrop applied: {applied:?}");
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
        self.audio.shutdown();
        self.icons.shutdown();
        log::info!("shutdown complete");
    }

    fn track_window(&mut self, ctx: &egui::Context, shown: bool) {
        if !shown {
            return;
        }
        let rects = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        if let (Some(outer), Some(inner)) = rects {
            let g = WindowGeometry { x: outer.min.x, y: outer.min.y, width: inner.width(), height: inner.height() };
            let changed = self.settings.window.is_none_or(|old| {
                (old.x - g.x).abs() > 0.5
                    || (old.y - g.y).abs() > 0.5
                    || (old.width - g.width).abs() > 0.5
                    || (old.height - g.height).abs() > 0.5
            });
            if changed && g.width > 0.0 && g.height > 0.0 {
                self.settings.window = Some(g);
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
        if self.pending_start_hide
            && let Some(h) = self.hwnd
        {
            if window::is_visible(h) {
                self.pending_start_hide = false;
                self.hide_to_tray();
                window::set_cloaked(h, false);
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
        if let Some(tray) = self.tray.as_mut() {
            tray.update(&snapshot);
        }
        for n in self.audio.take_notices() {
            self.toast(n.level, n.text);
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
            match self.view {
                View::Mixer => {
                    let mut cx = mixer::MixerCtx {
                        audio: &self.audio,
                        snap: &snapshot,
                        meters: &meters,
                        look: &look,
                        settings: &self.settings,
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
                        settings_path: self.store.path(),
                        errors: &mut errors,
                    };
                    if let settings_view::Outcome::Back = settings_view::show(ui, &mut cx) {
                        self.view = View::Mixer;
                    }
                }
            }
        });

        for e in errors {
            self.toast(NoticeLevel::Error, e);
        }
        if self.settings != before {
            self.mark_dirty();
            let ctx = ui.ctx().clone();
            self.refresh_look(&ctx);
            ctx.request_repaint();
        }
        self.draw_toasts(ui);
        let _ = FontId::default();
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
