//! Spike: verify that a transparent eframe/glow window can show a native
//! Windows 11 system backdrop (Acrylic / Mica) behind egui content.
//!
//! Env vars: GM_BACKDROP = acrylic | mica | none, GM_EXTEND = 1 | 0, GM_DARK = 1 | 0
#![windows_subsystem = "windows"]

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::UI::Controls::MARGINS;

struct Spike;

impl eframe::App for Spike {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::NONE.fill(egui::Color32::from_rgba_unmultiplied(20, 20, 28, 110)).inner_margin(16).show(
            ui,
            |ui| {
                ui.set_min_size(ui.available_size());
                ui.heading("Glass spike");
                ui.label("Text over the system backdrop should stay legible.");
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 14))
                    .corner_radius(10)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.label("A translucent card");
                        let mut v = 0.6f32;
                        ui.add(egui::Slider::new(&mut v, 0.0..=1.0));
                    });
            },
        );
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Glass spike")
            .with_inner_size([420.0, 320.0])
            .with_position([60.0, 60.0])
            .with_transparent(true),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "glass-spike",
        options,
        Box::new(|cc| {
            if let Ok(handle) = cc.window_handle()
                && let RawWindowHandle::Win32(h) = handle.as_raw()
            {
                let hwnd = HWND(h.hwnd.get() as *mut _);
                apply(hwnd);
            }
            Ok(Box::new(Spike))
        }),
    )
}

fn apply(hwnd: HWND) {
    let backdrop = std::env::var("GM_BACKDROP").unwrap_or_else(|_| "acrylic".into());
    let extend = std::env::var("GM_EXTEND").map(|v| v == "1").unwrap_or(true);
    let dark = std::env::var("GM_DARK").map(|v| v == "1").unwrap_or(true);
    unsafe {
        let dark_flag: i32 = dark as i32;
        let r1 = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark_flag as *const _ as _, 4);
        if extend {
            let m = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
            let r = DwmExtendFrameIntoClientArea(hwnd, &m);
            eprintln!("extend: {r:?}");
        }
        let kind = match backdrop.as_str() {
            "mica" => DWMSBT_MAINWINDOW,
            "none" => DWMSBT_NONE,
            _ => DWMSBT_TRANSIENTWINDOW,
        };
        let r2 = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &kind as *const _ as _, 4);
        eprintln!("dark: {r1:?}, backdrop {backdrop}: {r2:?}");
    }
}
