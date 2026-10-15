#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use glass_mixer::app::{MixerApp, StartOptions};
use glass_mixer::icon_art::render_app_icon;
use glass_mixer::platform::{single_instance, startup, window};
use glass_mixer::settings::{LoadOutcome, MIN_HEIGHT, MIN_WIDTH, SettingsStore, WindowGeometry};
use std::{path::PathBuf, process::ExitCode, sync::Arc};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    init_logging();
    install_panic_hook();
    log::info!("Glass Mixer {} starting (args: {:?})", env!("CARGO_PKG_VERSION"), &args[1..]);

    let Some(_instance) = single_instance::acquire() else {
        log::info!("another instance is running; activating it");
        window::activate_existing_instance();
        return ExitCode::SUCCESS;
    };

    let dir = SettingsStore::default_dir().unwrap_or_else(|| PathBuf::from("."));
    let store = SettingsStore::new(&dir);
    let outcome = store.load();
    let load_error = match &outcome {
        LoadOutcome::Recovered { error, .. } => Some(error.clone()),
        _ => None,
    };
    if let LoadOutcome::FirstRun(_) = outcome {
        log::info!("first run: using default settings");
    }
    let settings = outcome.into_settings();
    startup::repair_path_if_enabled();

    let hidden = args.iter().any(|a| a == startup::MINIMIZED_ARG) || settings.start_minimized;
    let geometry = settings.window.and_then(WindowGeometry::sanitized).filter(|g| on_screen(*g));

    let icon = egui::IconData { rgba: render_app_icon(64, false), width: 64, height: 64 };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(window::WINDOW_TITLE)
        .with_app_id("GlassMixer")
        .with_icon(Arc::new(icon))
        .with_min_inner_size([MIN_WIDTH, MIN_HEIGHT])
        // Always created visible: eframe shows the window after its first frame anyway.
        // When starting hidden, the app cloaks it and hides it once shown (no flash).
        .with_transparent(true);
    viewport = match geometry {
        Some(g) => viewport.with_position([g.x, g.y]).with_inner_size([g.width, g.height]),
        None => viewport.with_inner_size([420.0, 640.0]),
    };
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        centered: geometry.is_none(),
        persist_window: false,
        ..Default::default()
    };

    let result = eframe::run_native(
        window::WINDOW_TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(MixerApp::new(cc, StartOptions { store, settings, load_error, hidden })))),
    );
    match result {
        Ok(()) => {
            log::info!("exited normally");
            ExitCode::SUCCESS
        }
        Err(e) => {
            log::error!("failed to start the user interface: {e}");
            show_error(&format!(
                "Glass Mixer couldn't start its window.\n\n{e}\n\nThis usually means OpenGL isn't available \
                 (for example in some remote desktop or virtual machine sessions)."
            ));
            ExitCode::FAILURE
        }
    }
}

/// Rejects saved positions that are no longer on any monitor (e.g. a disconnected display).
fn on_screen(g: WindowGeometry) -> bool {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromPoint};
    use windows::Win32::UI::HiDpi::GetDpiForSystem;
    let scale = unsafe { GetDpiForSystem() } as f32 / 96.0;
    let pt = POINT { x: ((g.x + 40.0) * scale) as i32, y: ((g.y + 20.0) * scale) as i32 };
    !unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONULL) }.is_invalid()
}

fn log_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("GlassMixer")
}

fn init_logging() {
    use simplelog::{ConfigBuilder, LevelFilter, WriteLogger};
    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("glass-mixer.log");
    // Keep the log small: start fresh when it grows past 1 MB.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = std::fs::rename(&path, dir.join("glass-mixer.old.log"));
    }
    let config = ConfigBuilder::new()
        .add_filter_allow_str("glass_mixer")
        .set_time_offset_to_local()
        .unwrap_or_else(|b| b)
        .build();
    let level = if cfg!(debug_assertions) { LevelFilter::Debug } else { LevelFilter::Info };
    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = WriteLogger::init(level, config, file);
    }
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default(info);
    }));
}

fn show_error(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::HSTRING;
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(window::WINDOW_TITLE), MB_OK | MB_ICONERROR);
    }
}
