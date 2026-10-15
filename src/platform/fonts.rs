//! Loads the Windows UI fonts (Segoe UI) and the system icon font
//! (Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on Windows 10).

use egui::{FontData, FontDefinitions, FontFamily};
use std::{path::PathBuf, sync::Arc};

pub const SEMIBOLD: &str = "semibold";
pub const ICONS: &str = "icons";

fn fonts_dir() -> PathBuf {
    std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("Fonts")
}

fn load(name: &str) -> Option<Vec<u8>> {
    std::fs::read(fonts_dir().join(name)).ok()
}

/// Installs fonts; returns whether an icon font is available.
pub fn install(ctx: &egui::Context) -> bool {
    let mut defs = FontDefinitions::default();

    if let Some(regular) = load("segoeui.ttf") {
        defs.font_data.insert("segoe".into(), Arc::new(FontData::from_owned(regular)));
        defs.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    let semibold = load("seguisb.ttf").map(|b| ("segoe-sb", b));
    let mut semibold_family = Vec::new();
    if let Some((name, bytes)) = semibold {
        defs.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
        semibold_family.push(name.to_string());
    }
    semibold_family.extend(defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    defs.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold_family);

    let icon_font = load("SegoeIcons.ttf").or_else(|| load("segmdl2.ttf"));
    let has_icons = icon_font.is_some();
    let mut icon_family = Vec::new();
    if let Some(bytes) = icon_font {
        defs.font_data.insert("icons".into(), Arc::new(FontData::from_owned(bytes)));
        icon_family.push("icons".to_string());
    }
    icon_family.extend(defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    defs.families.insert(FontFamily::Name(ICONS.into()), icon_family);

    ctx.set_fonts(defs);
    if !has_icons {
        log::warn!("no Segoe icon font found; using text fallbacks");
    }
    has_icons
}
