//! UI typefaces and the icon font.
//!
//! The default typeface is Salted Flutters Polaris (Regular and Semibold),
//! embedded in the executable: nothing is installed and no file is read for it.
//! Segoe UI stays behind it as the fallback for any glyph Polaris lacks, and is
//! the whole typeface when the user chooses "Windows". Icons come from the system
//! icon font (Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on Windows 10).

use crate::settings::Typeface;
use egui::{FontData, FontDefinitions, FontFamily};
use std::{path::PathBuf, sync::Arc};

pub const SEMIBOLD: &str = "semibold";
pub const ICONS: &str = "icons";

/// Embedded unmodified; its license (`assets/fonts/LICENSE.txt`) ships with every package.
static POLARIS_REGULAR: &[u8] = include_bytes!("../../assets/fonts/SaltedFluttersPolaris-Regular.otf");
static POLARIS_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/SaltedFluttersPolaris-Semibold.otf");

/// Fonts read from the Windows Fonts folder; any of them can be missing.
#[derive(Default)]
struct SystemFonts {
    regular: Option<Vec<u8>>,
    semibold: Option<Vec<u8>>,
    icons: Option<Vec<u8>>,
}

impl SystemFonts {
    fn load() -> Self {
        let dir =
            std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("Fonts");
        let read = |name: &str| std::fs::read(dir.join(name)).ok();
        Self {
            regular: read("segoeui.ttf"),
            semibold: read("seguisb.ttf"),
            icons: read("SegoeIcons.ttf").or_else(|| read("segmdl2.ttf")),
        }
    }
}

/// Installs the fonts for `face`; returns whether an icon font is available.
pub fn install(ctx: &egui::Context, face: Typeface) -> bool {
    let (defs, has_icons) = definitions(face, SystemFonts::load());
    ctx.set_fonts(defs);
    if !has_icons {
        log::warn!("no Segoe icon font found; using text fallbacks");
    }
    has_icons
}

/// The font families for `face`. Each chain ends with egui's built-in fonts, so
/// text stays readable even if every preferred font is missing.
fn definitions(face: Typeface, system: SystemFonts) -> (FontDefinitions, bool) {
    let mut defs = FontDefinitions::default();
    let add = |defs: &mut FontDefinitions, key: &str, data: FontData| {
        defs.font_data.insert(key.into(), Arc::new(data));
        key.to_owned()
    };

    let mut regular: Vec<String> = Vec::new();
    let mut semibold: Vec<String> = Vec::new();
    if face == Typeface::Polaris {
        regular.push(add(&mut defs, "polaris-regular", FontData::from_static(POLARIS_REGULAR)));
        semibold.push(add(&mut defs, "polaris-semibold", FontData::from_static(POLARIS_SEMIBOLD)));
    }
    // A missing glyph in a semibold run falls back to a semibold glyph first.
    if let Some(bytes) = system.semibold {
        semibold.push(add(&mut defs, "segoe-sb", FontData::from_owned(bytes)));
    }
    if let Some(bytes) = system.regular {
        regular.push(add(&mut defs, "segoe", FontData::from_owned(bytes)));
    }

    let builtin = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let proportional: Vec<String> = regular.into_iter().chain(builtin).collect();
    let semibold_chain: Vec<String> = semibold.into_iter().chain(proportional.iter().cloned()).collect();

    let has_icons = system.icons.is_some();
    let mut icon_chain = Vec::new();
    if let Some(bytes) = system.icons {
        icon_chain.push(add(&mut defs, ICONS, FontData::from_owned(bytes)));
    }
    icon_chain.extend(proportional.iter().cloned());

    defs.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold_chain);
    defs.families.insert(FontFamily::Name(ICONS.into()), icon_chain);
    defs.families.insert(FontFamily::Proportional, proportional);
    (defs, has_icons)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::FontId;

    fn fake_system() -> SystemFonts {
        SystemFonts { regular: Some(vec![1]), semibold: Some(vec![2]), icons: Some(vec![3]) }
    }

    fn chain(defs: &FontDefinitions, family: FontFamily) -> Vec<&str> {
        defs.families[&family].iter().map(String::as_str).collect()
    }

    #[test]
    fn polaris_leads_each_chain_with_segoe_and_the_builtin_fonts_behind_it() {
        let (defs, has_icons) = definitions(Typeface::Polaris, fake_system());
        assert!(has_icons);
        let regular = chain(&defs, FontFamily::Proportional);
        assert_eq!(&regular[..2], ["polaris-regular", "segoe"]);
        assert!(regular.len() > 2, "egui's built-in fonts stay as the last fallback: {regular:?}");
        let semibold = chain(&defs, FontFamily::Name(SEMIBOLD.into()));
        assert_eq!(&semibold[..3], ["polaris-semibold", "segoe-sb", "polaris-regular"]);
        let icons = chain(&defs, FontFamily::Name(ICONS.into()));
        assert_eq!(&icons[..2], ["icons", "polaris-regular"], "icons first, text fallbacks behind");
    }

    #[test]
    fn the_windows_typeface_leaves_polaris_out_entirely() {
        let (defs, _) = definitions(Typeface::System, fake_system());
        assert!(!defs.font_data.keys().any(|k| k.starts_with("polaris")));
        assert_eq!(&chain(&defs, FontFamily::Proportional)[..1], ["segoe"]);
        assert_eq!(&chain(&defs, FontFamily::Name(SEMIBOLD.into()))[..2], ["segoe-sb", "segoe"]);
    }

    #[test]
    fn missing_system_fonts_fall_back_to_the_builtin_ones_and_report_no_icons() {
        let (defs, has_icons) = definitions(Typeface::Polaris, SystemFonts::default());
        assert!(!has_icons);
        assert_eq!(&chain(&defs, FontFamily::Proportional)[..1], ["polaris-regular"]);
        let (defs, _) = definitions(Typeface::System, SystemFonts::default());
        let builtin = FontDefinitions::default().families[&FontFamily::Proportional].clone();
        assert_eq!(defs.families[&FontFamily::Proportional], builtin, "nothing but egui's fonts remains");
    }

    #[test]
    fn the_embedded_fonts_are_opentype_files_that_keep_the_required_notice() {
        let notice = "Salted Flutters Polaris - Created by ppoolar";
        for (name, bytes) in [("Regular", POLARIS_REGULAR), ("Semibold", POLARIS_SEMIBOLD)] {
            assert_eq!(&bytes[..4], b"OTTO", "{name} is not a CFF OpenType file");
            let found = |needle: Vec<u8>| bytes.windows(needle.len()).any(|w| w == needle.as_slice());
            assert!(
                found(notice.as_bytes().to_vec()) || found(notice.encode_utf16().flat_map(u16::to_be_bytes).collect()),
                "{name} lost the metadata line its license requires"
            );
        }
        let license = include_str!("../../assets/fonts/LICENSE.txt");
        assert!(license.starts_with("SALTED FLUTTERS POLARIS FONT LICENSE"), "the license text was altered");
    }

    /// What the interface draws, per script: Portuguese, currency, punctuation and symbols.
    const SAMPLE: &str = "Configuração de áudio — Ação, são, coração, você, ônibus, pingüim \u{c0}\u{c1}\u{c2}\u{c3}\u{c7}\u{c9}\u{ca}\u{cd}\u{d3}\u{d4}\u{d5}\u{da}\u{dc} \
        R$ 1.234,56 € £ ¥ ¢ 0123456789 100% “aspas” ‘simples’ «» ¿¡ … · • – ← → ↑ ↓ ✓ @#&/|~";

    #[test]
    fn polaris_covers_the_interface_text_and_is_the_font_actually_used() {
        let width_of = |face: Typeface| {
            let ctx = egui::Context::default();
            install(&ctx, face);
            let mut result = None;
            for _ in 0..3 {
                let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let regular = FontId::proportional(14.0);
                    let semibold = FontId::new(14.0, FontFamily::Name(SEMIBOLD.into()));
                    let (covered_regular, covered_semibold) =
                        ui.ctx().fonts_mut(|f| (f.has_glyphs(&regular, SAMPLE), f.has_glyphs(&semibold, SAMPLE)));
                    let galley = ui.painter().layout_no_wrap(SAMPLE.to_owned(), regular, egui::Color32::WHITE);
                    result = Some((covered_regular, covered_semibold, galley.size().x));
                });
                output.drop_without_applying_deltas();
            }
            result.expect("a frame ran")
        };
        let (regular, semibold, polaris_width) = width_of(Typeface::Polaris);
        assert!(regular && semibold, "a glyph the interface uses is missing from the Polaris chains");
        let (_, _, system_width) = width_of(Typeface::System);
        assert!(polaris_width > 100.0);
        assert!(
            (polaris_width - system_width).abs() > 1.0,
            "both typefaces laid out identically ({polaris_width}); Polaris is not being used"
        );
    }
}
