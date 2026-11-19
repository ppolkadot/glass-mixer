//! A neutral preview of the interface typeface at the sizes the mixer uses, with
//! the glyphs it needs. Uses the app's own font installation, so it shows what the
//! Typeface setting does. Shows no app, device or user data.
//! cargo run --example font_preview

use egui::{Color32, FontFamily, FontId, RichText};
use glass_mixer::{
    platform::fonts::{self, SEMIBOLD},
    settings::Typeface,
};

struct Preview {
    face: Typeface,
}

impl eframe::App for Preview {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.horizontal(|ui| {
            for (face, label) in [(Typeface::Polaris, "Polaris"), (Typeface::System, "Windows")] {
                if ui.selectable_value(&mut self.face, face, label).clicked() {
                    fonts::install(ui.ctx(), face);
                }
            }
        });
        ui.visuals_mut().override_text_color = Some(Color32::from_gray(235));
        let semibold = |size: f32| FontId::new(size, FontFamily::Name(SEMIBOLD.into()));
        let regular = |size: f32| FontId::proportional(size);
        ui.separator();
        ui.label(RichText::new("Settings   Devices   Presets   Mixer").font(semibold(18.0)));
        ui.label(RichText::new("Output device").font(semibold(14.0)));
        ui.label(RichText::new("APPEARANCE").font(semibold(13.0)).color(Color32::from_gray(150)));
        for (text, size) in [
            ("Configuração de áudio — Ação, são, coração, você, ônibus, pingüim", 14.0),
            ("ÀÁÂÃÇÉÊÍÓÔÕÚÜ àáâãçéêíóôõúü ñ ß", 14.0),
            ("Volume 100%  45%  7%   R$ 1.234,56   € 9,90   £ 3   ¥ 120   0123456789", 14.0),
            ("“Quotes” ‘single’ « » ¿Qué? ¡Hola! … · • – — ← → ↑ ↓ ✓  (a) [b] {c} @ # & / \\ | ~", 13.0),
            ("Playing · 2 streams · lowered while you talk · Muted · Idle", 12.0),
        ] {
            ui.label(RichText::new(text).font(regular(size)));
        }
        ui.label(RichText::new("Same line in semibold: Ação, €, “aspas”, 100%").font(semibold(14.0)));
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 330.0]).with_title("Font preview"),
        ..Default::default()
    };
    eframe::run_native(
        "Font preview",
        options,
        Box::new(|cc| {
            fonts::install(&cc.egui_ctx, Typeface::Polaris);
            Ok(Box::new(Preview { face: Typeface::Polaris }))
        }),
    )
}
