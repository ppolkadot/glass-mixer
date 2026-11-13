//! Application lifecycle: wires the audio engine, tray, backdrop, settings and
//! views together, and owns the single shutdown path.

use crate::audio::{AudioHandle, NoticeLevel};
use crate::platform::backdrop::{self, Applied, Support};
use crate::platform::exit_request::ExitListener;
use crate::platform::fonts;
use crate::platform::foreground::ForegroundWatch;
use crate::platform::hotkeys::{HotkeyEditor, HotkeyEvent, HotkeyService};
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

/// Meters repaint the window on every change. A software renderer rasterizes
/// each frame on the CPU, so at meter rates the controls themselves would lag.
fn meters_wanted(shown: bool, show_meters: bool, software_renderer: bool) -> bool {
    shown && show_meters && !software_renderer
}

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
    expiry: ToastExpiry,
}

/// Visible display time for a toast. The countdown runs only while the window
/// is shown, so a toast queued while hidden is displayed in full later.
struct ToastExpiry {
    remaining: Duration,
    /// When the toast should disappear; `None` while the countdown is paused.
    due: Option<Instant>,
}

impl ToastExpiry {
    fn new(visible_for: Duration) -> Self {
        Self { remaining: visible_for, due: None }
    }

    /// Tracks a visibility change; repeated calls with the same state have no effect.
    fn tick(&mut self, shown: bool, now: Instant) {
        if shown {
            if self.due.is_none() {
                self.due = Some(now + self.remaining);
            }
        } else if let Some(due) = self.due.take() {
            self.remaining = due.saturating_duration_since(now);
        }
    }

    fn due(&self) -> Option<Instant> {
        self.due
    }

    fn expired(&self, now: Instant) -> bool {
        self.due.is_some_and(|due| due <= now)
    }
}

/// Logs the renderer and graphics adapter in use. Returns whether frames are
/// rendered in software (WARP, the "Microsoft Basic Render Driver"), and a
/// description for the diagnostic report.
fn log_renderer(cc: &eframe::CreationContext<'_>) -> (bool, String) {
    if let Some(state) = &cc.wgpu_render_state {
        let info = state.adapter.get_info();
        log::info!(
            "renderer: wgpu fallback, {:?} on \"{}\" ({:?}, driver {} {})",
            info.backend,
            info.name,
            info.device_type,
            info.driver,
            info.driver_info
        );
        for adapter in &state.available_adapters {
            let a = adapter.get_info();
            log::debug!("available wgpu adapter: {:?} \"{}\" ({:?})", a.backend, a.name, a.device_type);
        }
        let software = info.device_type == eframe::egui_wgpu::wgpu::DeviceType::Cpu;
        if software {
            log::info!("software rendering: level meters are paused");
        }
        (software, format!("wgpu fallback, {:?} on \"{}\" ({:?})", info.backend, info.name, info.device_type))
    } else {
        if cc.gl.is_some() {
            log::info!("renderer: OpenGL (glow)");
        }
        (false, "OpenGL (glow)".to_owned())
    }
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
    save_error_shown: bool,
    prefs: SystemPrefs,
    prefs_checked: Instant,
    support: Support,
    backdrop_key: Option<(Option<Backdrop>, bool)>,
    applied: Option<Applied>,
    backdrop_rejection: BackdropRejection,
    hwnd: Option<Hwnd>,
    tray: Option<Tray>,
    tray_rx: Receiver<TrayAction>,
    exit_listener: Option<ExitListener>,
    view: View,
    mixer: mixer::MixerState,
    login: LoginState,
    toasts: Vec<Toast>,
    confirm_route_reset: bool,
    foreground: Option<ForegroundWatch>,
    transfer_ui: settings_view::TransferUi,
    /// Settings from a confirmed import, applied at the start of the next frame so
    /// every part of the app picks up the change the way it does for any setting.
    import_to_apply: Option<Settings>,
    exiting: bool,
    has_icon_font: bool,
    look: Look,
    style_key: Option<Look>,
    shut_down: bool,
    /// Started with `--minimized`: hide once eframe has shown the (cloaked) window.
    pending_start_hide: bool,
    force_opaque: bool,
    software_renderer: bool,
    renderer: String,
    /// Warnings and errors shown to the user, for the diagnostic report.
    recent_notices: std::collections::VecDeque<crate::diagnostics::RecentNotice>,
    #[cfg(debug_assertions)]
    dev_exit_at: Option<Instant>,
}

impl MixerApp {
    pub fn new(cc: &eframe::CreationContext<'_>, opts: StartOptions) -> Self {
        let (software_renderer, renderer) = log_renderer(cc);
        let ctx = cc.egui_ctx.clone();
        let has_icon_font = fonts::install(&ctx, opts.settings.typeface);
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
        send_audio_preferences(&audio, &opts.settings);
        {
            let ctx = ctx.clone();
            audio.add_listener(move || ctx.request_repaint());
        }
        let icons = {
            let ctx = ctx.clone();
            IconService::start(move || ctx.request_repaint())
        };
        let (tray_tx, tray_rx) = crossbeam_channel::unbounded();
        // `--exit` (used by the uninstaller) takes the same path as the tray's Exit.
        let exit_listener = {
            let (tx, ctx) = (tray_tx.clone(), ctx.clone());
            ExitListener::start(move || {
                let _ = tx.send(TrayAction::Exit);
                ctx.request_repaint();
            })
        };
        let mut toasts = Vec::new();
        let tray = match hwnd {
            Some(h) => match Tray::create(h, audio.clone(), tray_tx, ctx.clone()) {
                Ok(t) => Some(t),
                Err(e) => {
                    log::error!("tray icon unavailable: {e}");
                    toasts.push(Toast {
                        level: NoticeLevel::Error,
                        text: "The tray icon couldn't be created, so closing the window will exit.".into(),
                        expiry: ToastExpiry::new(Duration::from_secs(10)),
                    });
                    None
                }
            },
            None => None,
        };
        // The show/hide shortcut must never hide the only way back to the UI.
        let hotkeys = if tray.is_some() {
            hwnd.and_then(|_| match HotkeyService::start(audio.clone(), ctx.clone()) {
                Ok(service) => {
                    // A start-hidden window is cloaked until its first frame; a show
                    // request before then would be lost, so register after hiding.
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
            tracking: opts.settings.typeface == crate::settings::Typeface::Polaris,
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
            save_error_shown: false,
            prefs,
            prefs_checked: Instant::now(),
            support,
            backdrop_key: None,
            applied: None,
            backdrop_rejection: BackdropRejection::default(),
            hwnd,
            tray,
            tray_rx,
            exit_listener,
            view: View::Mixer,
            mixer: mixer::MixerState::default(),
            login: startup::state(),
            toasts,
            confirm_route_reset: false,
            foreground: None,
            transfer_ui: settings_view::TransferUi::default(),
            import_to_apply: None,
            exiting: false,
            has_icon_font,
            look,
            style_key: None,
            shut_down: false,
            pending_start_hide: start_hidden,
            force_opaque: opts.force_opaque,
            software_renderer,
            renderer,
            recent_notices: std::collections::VecDeque::new(),
            #[cfg(debug_assertions)]
            dev_exit_at: std::env::var_os("GLASS_MIXER_DEV_PROFILE")
                .and_then(|_| std::env::var("GLASS_MIXER_DEV_EXIT_AFTER_MS").ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(|ms| Instant::now() + Duration::from_millis(ms)),
        };
        if let Some(err) = opts.load_error {
            app.toast(NoticeLevel::Warning, err);
        }
        app.refresh_look(&ctx);
        app.update_foreground_watch();
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
            tracking: self.settings.typeface == crate::settings::Typeface::Polaris,
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
        let (wait, error) = flush_save_if_due(
            &self.store,
            &self.settings,
            &mut self.dirty_since,
            &mut self.save_error_shown,
            Instant::now(),
            force,
        );
        if let Some(delay) = wait
            && !force
        {
            ctx.request_repaint_after(delay);
        }
        if let Some(msg) = error {
            self.toast(NoticeLevel::Error, msg);
        }
    }
}

fn flush_save_if_due(
    store: &SettingsStore,
    settings: &Settings,
    dirty_since: &mut Option<Instant>,
    save_error_shown: &mut bool,
    now: Instant,
    force: bool,
) -> (Option<Duration>, Option<String>) {
    let Some(since) = *dirty_since else { return (None, None) };
    let due = since + SAVE_DEBOUNCE;
    if !force && now < due {
        return (Some(due - now), None);
    }
    *dirty_since = None;
    match store.save(settings) {
        Ok(()) => {
            *save_error_shown = false;
            (None, None)
        }
        Err(e) => {
            log::error!("saving settings failed: {e}");
            *dirty_since = Some(now);
            let msg = (!std::mem::replace(save_error_shown, true)).then(|| format!("Couldn't save settings: {e}"));
            (Some(SAVE_DEBOUNCE), msg)
        }
    }
}

impl MixerApp {
    fn toast(&mut self, level: NoticeLevel, text: String) {
        self.notify(crate::audio::Notice { level, text, announce: false });
    }

    /// Shows a notice in the window, and in the tray while hidden.
    fn notify(&mut self, notice: crate::audio::Notice) {
        let crate::audio::Notice { level, text, announce } = notice;
        if level != NoticeLevel::Info {
            self.recent_notices.push_back(crate::diagnostics::RecentNotice { level, text: text.clone() });
            while self.recent_notices.len() > crate::diagnostics::MAX_NOTICES {
                self.recent_notices.pop_front();
            }
        }
        if self.settings.hidden_notifications
            && self.hwnd.is_some_and(|h| !window::is_shown(h))
            && let Some(tray) = self.tray.as_mut()
        {
            if announce {
                tray.announce(&text);
            } else {
                tray.notify(level, &text);
            }
        }
        let secs = match level {
            NoticeLevel::Info => 4,
            NoticeLevel::Warning => 7,
            NoticeLevel::Error => 9,
        };
        self.toasts.retain(|t| t.text != text);
        self.toasts.push(Toast { level, text, expiry: ToastExpiry::new(Duration::from_secs(secs)) });
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

    /// Show/hide shortcut. Ignored once exit has begun.
    fn toggle_window(&mut self) {
        if self.exiting || self.shut_down {
            return;
        }
        let Some(h) = self.hwnd else {
            return;
        };
        if window::is_shown(h) && window::is_foreground(h) {
            self.hide_to_tray();
        } else {
            window::show(h);
        }
    }

    fn unix_now() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs())
    }

    /// Writes the preferences to a file in the exports folder and shows the folder.
    fn export_settings(&mut self) {
        let dir = crate::transfer::exports_dir(self.store.path());
        let text = crate::transfer::export(&self.settings, env!("CARGO_PKG_VERSION"));
        match crate::transfer::save(&dir, crate::transfer::EXPORT_PREFIX, Self::unix_now(), &text) {
            Ok(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if let Err(e) = window::open_folder(&dir) {
                    log::warn!("couldn't open the exports folder: {e}");
                }
                self.toast(NoticeLevel::Info, format!("Settings exported: {name}"));
            }
            Err(e) => self.toast(NoticeLevel::Error, format!("Couldn't export the settings: {e}")),
        }
    }

    /// Reads and checks the file named in the Settings page; shows what it holds.
    fn check_import(&mut self) {
        let path = std::path::PathBuf::from(self.transfer_ui.path.trim().trim_matches('"'));
        self.transfer_ui.error = None;
        self.transfer_ui.candidate = None;
        match crate::transfer::read_import_file(&path) {
            Ok(import) => {
                let mut lines = crate::transfer::summary(&import.settings);
                lines.extend(import.notes.iter().map(|note| format!("Not read: {note}")));
                self.transfer_ui.candidate = Some((import.settings, lines));
            }
            Err(e) => self.transfer_ui.error = Some(e),
        }
    }

    /// Confirmed: backs up the current settings, then queues the import. Without a
    /// backup, nothing is replaced.
    fn confirm_import(&mut self) {
        let Some((imported, _)) = self.transfer_ui.candidate.clone() else { return };
        let dir = crate::transfer::exports_dir(self.store.path());
        let backup = crate::transfer::export(&self.settings, env!("CARGO_PKG_VERSION"));
        match crate::transfer::save(&dir, crate::transfer::BACKUP_PREFIX, Self::unix_now(), &backup) {
            Ok(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.import_to_apply = Some(crate::transfer::merge_keeping_this_pc(&self.settings, imported));
                self.transfer_ui.candidate = None;
                self.transfer_ui.path.clear();
                self.toast(
                    NoticeLevel::Info,
                    format!("Settings imported. Your previous settings were saved as {name}."),
                );
            }
            Err(e) => self.toast(
                NoticeLevel::Error,
                format!("Couldn't save a backup of your current settings, so nothing was imported: {e}"),
            ),
        }
    }

    fn diagnostic_report(&self) -> String {
        let snapshot = self.audio.snapshot();
        let hotkeys = self.hotkeys.as_ref().map(HotkeyService::status);
        let notices: Vec<_> = self.recent_notices.iter().cloned().collect();
        let log_tail = crate::diagnostics::read_log_tail(crate::diagnostics::log_lines());
        let unix_seconds =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs());
        crate::diagnostics::build(&crate::diagnostics::Inputs {
            settings: &self.settings,
            snapshot: &snapshot,
            prefs: &self.prefs,
            renderer: &self.renderer,
            software_renderer: self.software_renderer,
            hotkeys: hotkeys.as_ref(),
            notices: &notices,
            log_tail: &log_tail,
            unix_seconds,
        })
    }

    /// Writes the diagnostic report beside the logs and shows the folder.
    fn save_diagnostic_report(&mut self) {
        let Some(dir) = crate::diagnostics::reports_dir() else {
            return self.toast(NoticeLevel::Error, "Couldn't find a folder for the diagnostic report.".into());
        };
        let report = self.diagnostic_report();
        let unix_seconds =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs());
        match crate::diagnostics::save(&report, &dir, unix_seconds) {
            Ok(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if let Err(e) = window::open_folder(&dir) {
                    log::warn!("couldn't open the diagnostics folder: {e}");
                }
                self.toast(NoticeLevel::Info, format!("Diagnostic report saved: {name}"));
            }
            Err(e) => self.toast(NoticeLevel::Error, format!("Couldn't save the diagnostic report: {e}")),
        }
    }

    /// The foreground watcher runs only while a focus rule exists.
    fn update_foreground_watch(&mut self) {
        let wanted = !self.settings.focus_rules.is_empty() && !self.shut_down;
        if wanted == self.foreground.is_some() {
            return;
        }
        self.foreground = if wanted {
            let audio = self.audio.clone();
            match ForegroundWatch::start(move |pid| audio.send(crate::audio::Command::ForegroundChanged { pid })) {
                Ok(watch) => Some(watch),
                Err(e) => {
                    self.toast(NoticeLevel::Error, format!("{e} Background lowering is unavailable."));
                    None
                }
            }
        } else {
            None
        };
    }

    fn activate_profile(&mut self, name: &str) {
        match self.settings.presets.iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
            Some(profile) => self.audio.send(crate::audio::Command::ActivateProfile(profile.clone())),
            None => self.toast(
                NoticeLevel::Warning,
                format!("The profile \"{name}\" no longer exists, so its shortcut does nothing."),
            ),
        }
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
        self.exit_listener = None;
        if self.dirty_since.take().is_some()
            && let Err(e) = self.store.save(&self.settings)
        {
            log::error!("saving settings on exit failed: {e}");
        }
        self.tray = None;
        self.hotkeys = None;
        self.foreground = None;
        self.audio.shutdown();
        self.icons.shutdown();
        log::info!("shutdown complete");
    }

    fn track_window(&mut self, ctx: &egui::Context, shown: bool) {
        if !shown {
            return;
        }
        let (outer, inner, maximized) = ctx.input(|i| {
            let v = i.viewport();
            (v.outer_rect, v.inner_rect, v.maximized.unwrap_or(false))
        });
        let outer_pos = outer.zip(self.hwnd).and_then(|(_, h)| window::outer_position(h));
        let inner_size = inner.map(|inner| (inner.width(), inner.height()));
        if record_window_geometry(&mut self.settings, shown, maximized, outer_pos, inner_size) {
            self.mark_dirty();
        }
    }

    fn draw_toasts(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        // Toasts queued during `ui` start counting down here.
        for toast in &mut self.toasts {
            toast.expiry.tick(true, now);
        }
        self.toasts.retain(|t| !t.expiry.expired(now));
        if let Some(next) = self.toasts.iter().filter_map(|t| t.expiry.due()).min() {
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

/// Preferences the engine acts on by itself (shortcuts reach it directly).
fn send_audio_preferences(audio: &AudioHandle, settings: &Settings) {
    use crate::audio::{Command, MicDucking, attenuation::Reduction, automation::MicGateConfig};
    audio.send(Command::SetPinnedOutputs(settings.pinned_devices.iter().map(|d| d.id.clone()).collect()));
    audio.send(Command::SetSoloOptions { factor: settings.solo.factor(), mute: settings.solo.mute });
    let keep = |percent: u8| f32::from(percent.min(90)) / 100.0;
    audio.send(Command::SetFocusRules(
        settings
            .focus_rules
            .iter()
            .map(|rule| (rule.key.clone(), Reduction { factor: keep(rule.keep_percent), mute: rule.mute }))
            .collect(),
    ));
    let mic = &settings.mic_ducking;
    audio.send(Command::SetMicDucking((mic.enabled && !mic.targets.is_empty()).then(|| MicDucking {
        gate: MicGateConfig {
            threshold: f32::from(mic.threshold_percent.clamp(1, 60)) / 100.0,
            attack: Duration::from_millis(u64::from(mic.attack_ms.clamp(20, 2000))),
            release: Duration::from_millis(u64::from(mic.release_ms.clamp(100, 5000))),
        },
        reduction: Reduction { factor: keep(mic.keep_percent), mute: false },
        targets: mic.targets.clone(),
    })));
}

fn record_window_geometry(
    settings: &mut Settings,
    shown: bool,
    maximized: bool,
    outer_pos: Option<(f32, f32)>,
    inner_size: Option<(f32, f32)>,
) -> bool {
    if !shown || maximized {
        return false;
    }
    let (Some((x, y)), Some((width, height))) = (outer_pos, inner_size) else {
        return false;
    };
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    let g = WindowGeometry { x, y, width, height };
    let changed = crate::settings::geometry_changed(settings.window, settings.window_position_physical, g);
    if changed {
        settings.window = Some(g);
        settings.window_position_physical = true;
    }
    changed
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

        // The shortcut thread only queues window actions; the UI thread performs
        // them, so exit never waits on a cross-thread window call.
        let hotkey_events = self.hotkeys.as_ref().map(HotkeyService::take_events).unwrap_or_default();
        for event in hotkey_events {
            match event {
                HotkeyEvent::ToggleWindow => self.toggle_window(),
                HotkeyEvent::ActivateProfile(name) => self.activate_profile(&name),
            }
        }

        let shown = self.hwnd.is_none_or(window::is_shown);
        self.audio.set_meters_wanted(meters_wanted(shown, self.settings.show_meters, self.software_renderer));

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
            self.notify(n);
        }
        if let Some(service) = &self.hotkeys {
            let errors = service.status().messages();
            if errors != self.last_hotkey_errors {
                self.last_hotkey_errors = errors.clone();
                for error in errors {
                    self.toast(NoticeLevel::Warning, error);
                }
            }
        }

        let toast_now = Instant::now();
        for toast in &mut self.toasts {
            toast.expiry.tick(shown, toast_now);
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
            // Windows doesn't notify eframe of theme or transparency changes, so
            // re-read them while visible.
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
        if let Some(imported) = self.import_to_apply.take() {
            self.settings = imported;
            self.mixer.forget_drafts();
            self.hotkey_editor = HotkeyEditor::default();
            ui.ctx().request_repaint();
        }
        let mut errors = Vec::new();
        let (mut save_report, mut copy_report) = (false, false);
        let (mut export_now, mut check_now, mut import_now) = (false, false, false);

        egui::Frame::NONE.inner_margin(Margin::same(look.m.pad as i8)).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                let look = &self.look;
                let menu_text =
                    |text| crate::ui::typography::rich(text, crate::ui::typography::Role::CalloutStrong, look);
                ui.selectable_value(&mut self.view, View::Mixer, menu_text("Mixer"));
                if ui.selectable_value(&mut self.view, View::Devices, menu_text("Devices")).clicked() {
                    self.audio.send(crate::audio::Command::LoadDevices);
                }
                ui.selectable_value(&mut self.view, View::Presets, menu_text("Presets"));
                if ui.selectable_value(&mut self.view, View::Settings, menu_text("Settings")).clicked() {
                    // Task Manager can change the launch-at-login state behind our back.
                    self.login = startup::state();
                }
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
                    if let mixer::Outcome::OpenDevices = mixer::show(ui, &mut self.mixer, &mut cx) {
                        self.audio.send(crate::audio::Command::LoadDevices);
                        self.view = View::Devices;
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
                        software_renderer: self.software_renderer,
                        can_switch_default: snapshot.can_switch_default,
                        can_route_apps: snapshot.can_route_apps,
                        confirm_route_reset: &mut self.confirm_route_reset,
                        transfer: &mut self.transfer_ui,
                        audio: &self.audio,
                        icons: &mut self.icons,
                        settings_path: self.store.path(),
                        hotkeys: self.hotkeys.as_ref(),
                        hotkey_editor: &mut self.hotkey_editor,
                        errors: &mut errors,
                    };
                    match settings_view::show(ui, &mut cx) {
                        settings_view::Outcome::Back => self.view = View::Mixer,
                        settings_view::Outcome::SaveDiagnostics => save_report = true,
                        settings_view::Outcome::CopyDiagnostics => copy_report = true,
                        settings_view::Outcome::ExportSettings => export_now = true,
                        settings_view::Outcome::CheckImport => check_now = true,
                        settings_view::Outcome::ApplyImport => import_now = true,
                        settings_view::Outcome::None => {}
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
            if self.settings.typeface != before.typeface {
                fonts::install(ui.ctx(), self.settings.typeface);
            }
            if self.settings.pinned_devices != before.pinned_devices
                || self.settings.solo != before.solo
                || self.settings.focus_rules != before.focus_rules
                || self.settings.mic_ducking != before.mic_ducking
            {
                send_audio_preferences(&self.audio, &self.settings);
                self.update_foreground_watch();
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
        if save_report {
            self.save_diagnostic_report();
        }
        if copy_report {
            ui.ctx().copy_text(self.diagnostic_report());
            self.toast(NoticeLevel::Info, "Diagnostic report copied.".into());
        }
        if export_now {
            self.export_settings();
        }
        if check_now {
            self.check_import();
        }
        if import_now {
            self.confirm_import();
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
    fn hidden_toast_time_is_frozen_and_restarts_when_first_shown() {
        let created = Instant::now();

        // Queued while hidden: the full duration starts at the reveal.
        let mut queued_while_hidden = ToastExpiry::new(Duration::from_secs(7));
        queued_while_hidden.tick(true, created + Duration::from_secs(3600));
        assert!(
            !queued_while_hidden.expired(created + Duration::from_secs(3600 + 6)),
            "a warning queued while hidden must survive a long idle and be displayable on reveal"
        );
        assert!(queued_while_hidden.expired(created + Duration::from_secs(3600 + 8)));

        // Hidden mid-countdown: the remaining time resumes on reveal.
        let mut frozen = ToastExpiry::new(Duration::from_secs(7));
        frozen.tick(true, created);
        frozen.tick(false, created + Duration::from_secs(3));
        frozen.tick(true, created + Duration::from_secs(3600));
        assert!(!frozen.expired(created + Duration::from_secs(3600 + 3)), "7s minus 3s shown = 4s left");
        assert!(frozen.expired(created + Duration::from_secs(3600 + 5)));
    }

    #[test]
    fn meters_run_while_shown_except_with_a_software_renderer() {
        assert!(meters_wanted(true, true, false));
        assert!(!meters_wanted(false, true, false), "hidden");
        assert!(!meters_wanted(true, false, false), "meters turned off");
        assert!(!meters_wanted(true, true, true), "software rendering");
    }

    #[test]
    fn repeated_visible_passes_do_not_extend_a_toast() {
        let created = Instant::now();
        let mut expiry = ToastExpiry::new(Duration::from_secs(4));
        expiry.tick(true, created);
        expiry.tick(true, created + Duration::from_secs(2));
        assert!(!expiry.expired(created + Duration::from_millis(3900)));
        assert!(expiry.expired(created + Duration::from_secs(4)));
    }

    #[test]
    fn a_toast_expired_while_visible_stays_gone_across_hide_and_reveal() {
        let created = Instant::now();
        let mut expiry = ToastExpiry::new(Duration::from_secs(4));
        expiry.tick(true, created);
        assert!(expiry.expired(created + Duration::from_secs(4)));
        expiry.tick(false, created + Duration::from_secs(5));
        expiry.tick(true, created + Duration::from_secs(100));
        assert!(expiry.expired(created + Duration::from_secs(100)));
    }

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

    #[test]
    fn window_tracking_ignores_hidden_or_empty_frames_and_marks_real_changes() {
        let mut settings = Settings {
            window: Some(WindowGeometry { x: 100.0, y: 120.0, width: 480.0, height: 720.0 }),
            window_position_physical: false,
            ..Default::default()
        };
        assert!(
            !record_window_geometry(&mut settings, false, false, Some((200.0, 220.0)), Some((480.0, 720.0))),
            "hidden windows must not overwrite saved geometry"
        );
        assert!(
            !record_window_geometry(&mut settings, true, true, Some((0.0, 0.0)), Some((1920.0, 1080.0))),
            "maximized frames must not overwrite saved normal geometry"
        );
        assert!(
            !record_window_geometry(&mut settings, true, false, Some((200.0, 220.0)), Some((0.0, 720.0))),
            "minimized/zero-width inner rect must be ignored"
        );
        assert!(
            record_window_geometry(&mut settings, true, false, Some((100.0, 120.0)), Some((480.0, 720.0))),
            "legacy logical coordinates upgrade to physical once"
        );
        assert!(settings.window_position_physical);
        assert!(
            !record_window_geometry(&mut settings, true, false, Some((100.2, 120.1)), Some((480.0, 720.0))),
            "sub-pixel jitter does not dirty settings"
        );
        assert!(
            record_window_geometry(&mut settings, true, false, Some((140.0, 160.0)), Some((500.0, 740.0))),
            "real move/resize updates geometry"
        );
        assert_eq!(settings.window, Some(WindowGeometry { x: 140.0, y: 160.0, width: 500.0, height: 740.0 }));
    }

    #[test]
    fn failed_save_keeps_dirty_state_for_retry_and_toasts_once_per_streak() {
        let root = std::env::temp_dir().join(format!("glass-mixer-save-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&root);
        // A regular file where a directory is expected makes SettingsStore::save fail.
        std::fs::write(&root, b"not a directory").unwrap();
        let store = SettingsStore::new(&root);
        let settings = Settings::default();
        let t0 = Instant::now();
        let mut dirty_since = Some(t0);
        let mut error_shown = false;

        let (wait, err) =
            flush_save_if_due(&store, &settings, &mut dirty_since, &mut error_shown, t0 + SAVE_DEBOUNCE, false);
        assert!(wait.is_some());
        assert!(err.is_some(), "first failure surfaces an error toast");
        assert_eq!(dirty_since, Some(t0 + SAVE_DEBOUNCE), "dirty_since is restored for debounce retry and shutdown");
        assert!(error_shown);

        // Second consecutive failure retries after debounce but does not re-toast.
        let t1 = t0 + SAVE_DEBOUNCE * 2;
        let (_wait, err2) = flush_save_if_due(&store, &settings, &mut dirty_since, &mut error_shown, t1, false);
        assert!(err2.is_none(), "repeated failure suppresses duplicate toast");
        assert_eq!(dirty_since, Some(t1));

        // Removing the obstacle lets the next retry succeed and clears both flags.
        std::fs::remove_file(&root).unwrap();
        let (wait, err3) = flush_save_if_due(&store, &settings, &mut dirty_since, &mut error_shown, t1, true);
        assert!(wait.is_none());
        assert!(err3.is_none());
        assert!(dirty_since.is_none());
        assert!(!error_shown);
        let _ = std::fs::remove_dir_all(&root);
    }
}
