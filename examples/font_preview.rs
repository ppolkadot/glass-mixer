//! A neutral preview of the interface typeface at the sizes the mixer uses, with
//! the glyphs it needs. Uses the app's own font installation, so it shows what the
//! Typeface setting does. Shows no app, device or user data.
//! cargo run --example font_preview

use egui::{Color32, RichText};
use glass_mixer::{
    platform::fonts,
    settings::Typeface,
    ui::typography::{Role, font, spec},
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
        ui.separator();
        for (role, sample) in [
            (Role::LargeTitle, "LargeTitle — Settings · Devices · Profiles · Mixer"),
            (Role::Title, "Title — Apps · Outputs · Inputs"),
            (Role::Headline, "Headline — Firefox · Output device"),
            (Role::Body, "Body — Show audio meters and controls"),
            (Role::Callout, "Callout — Secondary control or combo item"),
            (Role::CalloutStrong, "CalloutStrong — Save profile · Mixer"),
            (Role::Footnote, "Footnote — Playing · 2 streams · lowered while you talk"),
            (Role::Caption, "Caption — C:\\Program Files\\Glass Mixer\\settings.json"),
            (Role::Numeric, "Numeric — 100%  45%  7%  0123456789"),
        ] {
            let (size, weight) = spec(role);
            ui.label(RichText::new(format!("{sample} ({size:.0} pt, {weight:?})")).font(font(role)));
        }
        ui.separator();
        for (text, role) in [
            ("Configuração de áudio — Ação, são, coração, você, ônibus, pingüim", Role::Body),
            ("ÀÁÂÃÇÉÊÍÓÔÕÚÜ àáâãçéêíóôõúü ñ ß", Role::Body),
            (
                "R$ 1.234,56   € 9,90   £ 3   ¥ 120   “Quotes” ‘single’ « » ¿Qué? ¡Hola! … · • – — ← → ↑ ↓ ✓",
                Role::Callout,
            ),
            ("Same line in semibold and bold: Ação, €, “aspas”, 100%", Role::Title),
        ] {
            ui.label(RichText::new(text).font(font(role)));
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 460.0]).with_title("Font preview"),
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
