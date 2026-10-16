#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use glass_mixer::app::{MixerApp, StartOptions};
use glass_mixer::icon_art::render_app_icon;
use glass_mixer::platform::{single_instance, startup, window};
use glass_mixer::settings::{
    DEFAULT_HEIGHT, DEFAULT_WIDTH, LoadOutcome, MIN_HEIGHT, MIN_WIDTH, SettingsStore, WindowGeometry,
};
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
    let geometry = settings.window.and_then(WindowGeometry::sanitized);
    let restored_position = geometry.and_then(|g| on_screen(g, settings.window_position_physical));
    let geometry = geometry.filter(|_| restored_position.is_some());

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
        Some(g) => viewport.with_inner_size([g.width, g.height]),
        None => viewport.with_inner_size([DEFAULT_WIDTH, DEFAULT_HEIGHT]),
    };
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        centered: restored_position.is_none(),
        persist_window: false,
        ..Default::default()
    };

    let result = eframe::run_native(
        window::WINDOW_TITLE,
        options,
        Box::new(move |cc| {
            Ok(Box::new(MixerApp::new(cc, StartOptions { store, settings, load_error, hidden, restored_position })))
        }),
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

#[derive(Clone, Copy)]
struct MonitorArea {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    scale: f32,
}

/// Version 1 positions were egui points divided by the *source monitor's* DPI.
/// Try every monitor. An ambiguous legacy position is centered rather than
/// guessed; version 2 positions are stored as physical screen pixels.
fn restore_position(g: WindowGeometry, physical: bool, monitors: &[MonitorArea]) -> Option<(i32, i32)> {
    let mut candidates = monitors.iter().filter_map(|m| {
        let scale = if physical { 1.0 } else { m.scale };
        let x = (g.x * scale).round() as i32;
        let y = (g.y * scale).round() as i32;
        let check_x = ((g.x + 40.0) * scale).round() as i32;
        let check_y = ((g.y + 20.0) * scale).round() as i32;
        (check_x >= m.left && check_x < m.right && check_y >= m.top && check_y < m.bottom).then_some((x, y))
    });
    let first = candidates.next()?;
    if candidates.next().is_some() { None } else { Some(first) }
}

fn on_screen(g: WindowGeometry, physical: bool) -> Option<(i32, i32)> {
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::core::BOOL;

    unsafe extern "system" fn visit(monitor: HMONITOR, _dc: HDC, _rect: *mut RECT, data: LPARAM) -> BOOL {
        let monitors = unsafe { &mut *(data.0 as *mut Vec<MonitorArea>) };
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            let mut x_dpi = 0;
            let mut y_dpi = 0;
            if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x_dpi, &mut y_dpi) }.is_ok() {
                let r = info.rcMonitor;
                monitors.push(MonitorArea {
                    left: r.left,
                    top: r.top,
                    right: r.right,
                    bottom: r.bottom,
                    scale: x_dpi as f32 / 96.0,
                });
            }
        }
        true.into()
    }

    let mut monitors = Vec::new();
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(visit), LPARAM(&mut monitors as *mut _ as isize)) };
    restore_position(g, physical, &monitors)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_position_uses_the_target_monitors_dpi() {
        let monitors = [
            MonitorArea { left: 0, top: 0, right: 1920, bottom: 1080, scale: 1.0 },
            MonitorArea { left: 1920, top: 0, right: 4480, bottom: 1440, scale: 2.0 },
        ];
        let g = WindowGeometry { x: 2000.0, y: 100.0, width: 420.0, height: 640.0 };
        assert_eq!(restore_position(g, false, &monitors), Some((4000, 200)));
        assert_eq!(restore_position(WindowGeometry { x: 1100.0, ..g }, false, &monitors), None);
        assert_eq!(restore_position(WindowGeometry { x: 2200.0, y: 200.0, ..g }, true, &monitors), Some((2200, 200)));
        assert_eq!(restore_position(WindowGeometry { x: 5000.0, ..g }, true, &monitors), None);
    }
}
