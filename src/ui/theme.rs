//! Design tokens and egui style setup.
//!
//! Dark and light palettes are designed separately (light is not an inverted
//! dark). Glass variants use translucent surfaces over the DWM backdrop;
//! opaque variants are used when glass is off or unsupported.

use crate::settings::{Accent, Density};
use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Vec2, Visuals};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub glass: bool,
    pub high_contrast: bool,
    /// Window background: a translucent tint over the backdrop, or opaque.
    pub base: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    /// Opaque-enough fill for popups, inputs and the selected segment.
    pub surface_strong: Color32,
    pub border: Color32,
    /// Hairlines inside grouped lists.
    pub separator: Color32,
    /// Recessed control background: slider track, toggle off, segmented trough, search field.
    pub fill: Color32,
    /// Selected segment pill surface.
    pub raised: Color32,
    /// Slider thumb and toggle knob fill.
    pub knob: Color32,
    /// Soft shadow under knobs, raised segments, and toasts.
    pub shadow: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub track: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub playing: Color32,
    pub focus: Color32,
}

/// (dark-theme value, light-theme value). Light values are darker so they keep
/// at least 4.5:1 contrast against light surfaces.
pub fn accent_color(accent: Accent, dark: bool) -> Color32 {
    let (d, l) = match accent {
        Accent::Violet => (0x9b7bff, 0x6d28d9),
        Accent::Blue => (0x5b9dff, 0x1d4ed8),
        Accent::Teal => (0x2dd4bf, 0x0f766e),
        Accent::Green => (0x4ade80, 0x15803d),
        Accent::Amber => (0xfbbf24, 0xb45309),
        Accent::Coral => (0xfb7185, 0xbe123c),
        Accent::Pink => (0xf472b6, 0xbe185d),
    };
    hex(if dark { d } else { l })
}

fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

pub fn palette(dark: bool, glass: bool, accent: Accent, high_contrast: bool) -> Palette {
    if high_contrast {
        // Opaque, maximum-contrast palette; colour is never the only signal.
        let accent = hex(0xffd60a);
        return Palette {
            dark: true,
            glass: false,
            high_contrast: true,
            base: Color32::BLACK,
            surface: Color32::BLACK,
            surface_hover: hex(0x1a1a1a),
            surface_strong: hex(0x101010),
            border: Color32::WHITE,
            separator: Color32::WHITE,
            fill: hex(0x808080),
            raised: accent,
            knob: Color32::WHITE,
            shadow: Color32::TRANSPARENT,
            text: Color32::WHITE,
            text_muted: hex(0xe0e0e0),
            track: hex(0x808080),
            accent,
            accent_soft: alpha(accent, 60),
            on_accent: Color32::BLACK,
            danger: hex(0xff6b6b),
            playing: hex(0x3ff23f),
            focus: Color32::WHITE,
        };
    }
    let accent_c = accent_color(accent, dark);
    if dark {
        Palette {
            dark,
            glass,
            high_contrast: false,
            base: if glass { Color32::from_rgba_unmultiplied(16, 16, 22, 96) } else { hex(0x1b1b22) },
            surface: if glass { Color32::from_rgba_unmultiplied(255, 255, 255, 14) } else { hex(0x24242d) },
            surface_hover: if glass { Color32::from_rgba_unmultiplied(255, 255, 255, 24) } else { hex(0x2d2d38) },
            surface_strong: hex(0x2b2b35),
            border: Color32::from_rgba_unmultiplied(255, 255, 255, if glass { 22 } else { 18 }),
            separator: Color32::from_rgba_unmultiplied(255, 255, 255, 26),
            fill: Color32::from_rgba_unmultiplied(120, 120, 128, 92),
            raised: Color32::from_rgb(99, 99, 102),
            knob: Color32::WHITE,
            shadow: Color32::from_rgba_unmultiplied(0, 0, 0, 90),
            text: hex(0xf4f4f8),
            text_muted: hex(0xb4b4c3),
            track: Color32::from_rgba_unmultiplied(255, 255, 255, 40),
            accent: accent_c,
            accent_soft: alpha(accent_c, 48),
            on_accent: hex(0x101014),
            danger: hex(0xff8a8a),
            playing: hex(0x4ade80),
            focus: accent_c,
        }
    } else {
        Palette {
            dark,
            glass,
            high_contrast: false,
            base: if glass { Color32::from_rgba_unmultiplied(250, 250, 253, 110) } else { hex(0xf2f2f6) },
            surface: if glass { Color32::from_rgba_unmultiplied(255, 255, 255, 150) } else { hex(0xffffff) },
            surface_hover: if glass { Color32::from_rgba_unmultiplied(255, 255, 255, 200) } else { hex(0xf7f7fb) },
            surface_strong: hex(0xffffff),
            border: Color32::from_rgba_unmultiplied(0, 0, 0, if glass { 20 } else { 24 }),
            separator: Color32::from_rgba_unmultiplied(0, 0, 0, 26),
            fill: Color32::from_rgba_unmultiplied(120, 120, 128, 51),
            raised: Color32::WHITE,
            knob: Color32::WHITE,
            shadow: Color32::from_rgba_unmultiplied(0, 0, 0, 40),
            text: hex(0x16161d),
            text_muted: hex(0x52525e),
            track: Color32::from_rgba_unmultiplied(0, 0, 0, 34),
            accent: accent_c,
            accent_soft: alpha(accent_c, 36),
            on_accent: hex(0xffffff),
            danger: hex(0xc81e1e),
            playing: hex(0x15803d),
            focus: accent_c,
        }
    }
}

/// Size tokens derived from the density setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub pad: f32,
    pub gap: f32,
    pub row_height: f32,
    pub icon: f32,
    pub button: f32,
    pub radius: u8,
    pub control_radius: u8,
}

pub fn metrics(density: Density) -> Metrics {
    match density {
        Density::Comfortable => {
            Metrics { pad: 14.0, gap: 10.0, row_height: 52.0, icon: 28.0, button: 32.0, radius: 12, control_radius: 8 }
        }
        Density::Compact => {
            Metrics { pad: 10.0, gap: 6.0, row_height: 40.0, icon: 22.0, button: 28.0, radius: 10, control_radius: 7 }
        }
    }
}

pub fn apply(ctx: &egui::Context, p: &Palette, m: &Metrics, animations: bool) {
    let mut style = egui::Style {
        text_styles: [
            (TextStyle::Heading, super::typography::font(super::typography::Role::Title)),
            (TextStyle::Body, super::typography::font(super::typography::Role::Body)),
            (TextStyle::Button, super::typography::font(super::typography::Role::CalloutStrong)),
            (TextStyle::Small, super::typography::font(super::typography::Role::Footnote)),
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        ]
        .into(),
        ..Default::default()
    };
    style.animation_time = if animations { 0.12 } else { 0.0 };
    style.spacing.item_spacing = Vec2::new(8.0, m.gap * 0.6);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.interact_size = Vec2::new(40.0, m.button - 4.0);
    style.spacing.combo_height = 300.0;
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    style.interaction.selectable_labels = false;

    let r = CornerRadius::same(m.radius);
    let small = CornerRadius::same(m.control_radius);
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.text_muted);
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = p.surface_strong;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = r;
    v.menu_corner_radius = r;
    v.window_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: p.shadow };
    v.popup_shadow = Shadow { offset: [0, 4], blur: 12, spread: 0, color: p.shadow };
    v.extreme_bg_color = p.surface_strong;
    v.text_edit_bg_color = Some(p.surface);
    v.faint_bg_color = p.surface;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(1.5, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.error_fg_color = p.danger;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = small;
    for (wv, fill, stroke) in [
        (&mut w.inactive, p.surface, p.border),
        (&mut w.hovered, p.surface_hover, p.border),
        (&mut w.active, p.surface_hover, p.border),
        (&mut w.open, p.surface_hover, p.accent),
    ] {
        wv.bg_fill = fill;
        wv.weak_bg_fill = fill;
        wv.bg_stroke = Stroke::new(1.0, stroke);
        wv.fg_stroke = Stroke::new(1.5, p.text);
        wv.corner_radius = small;
        wv.expansion = 0.0;
    }
    style.visuals = v;
    // Same style for egui's dark and light slots: the theme is chosen by us, and the
    // native title bar is themed through DWM in `platform::backdrop`.
    ctx.options_mut(|o| o.sync_window_theme = false);
    ctx.all_styles_mut(|s| *s = style.clone());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative_luminance(c: Color32) -> f32 {
        let channel = |v: u8| {
            let s = f32::from(v) / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (l1, l2) = (relative_luminance(a), relative_luminance(b));
        let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn palette_tokens_meet_wcag_contrast_thresholds() {
        for dark in [true, false] {
            for accent in Accent::ALL {
                let p = palette(dark, false, accent, false);
                let check = |name: &str, fg: Color32, bg: Color32, min: f32| {
                    let ratio = contrast(fg, bg);
                    assert!(ratio >= min, "dark={dark} {accent:?} {name}: {ratio:.2} < {min}");
                };
                check("text on base", p.text, p.base, 7.0);
                check("text_muted on surface", p.text_muted, p.surface, 4.5);
                check("text on raised", p.text, p.raised, 4.5);
                check("accent on base", p.accent, p.base, 3.0);
                check("on_accent on accent", p.on_accent, p.accent, 4.5);
            }
        }
    }
}
