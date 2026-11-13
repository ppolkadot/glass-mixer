//! Typography roles, sizes and weight mapping.

use super::Look;
use crate::platform::fonts;
use egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, RichText, TextFormat, text::LayoutJob};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    LargeTitle,
    Title,
    Headline,
    Body,
    Callout,
    CalloutStrong,
    Footnote,
    Caption,
    Numeric,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Semibold,
    Bold,
}

impl Role {
    pub const ALL: [Self; 9] = [
        Self::LargeTitle,
        Self::Title,
        Self::Headline,
        Self::Body,
        Self::Callout,
        Self::CalloutStrong,
        Self::Footnote,
        Self::Caption,
        Self::Numeric,
    ];
}

/// `(size in points, weight)`. The single table that decides the typographic hierarchy.
pub const fn spec(role: Role) -> (f32, Weight) {
    match role {
        Role::LargeTitle => (22.0, Weight::Bold),
        Role::Title => (17.0, Weight::Bold),
        Role::Headline => (14.0, Weight::Semibold),
        Role::Body => (14.0, Weight::Regular),
        Role::Callout => (13.0, Weight::Regular),
        Role::CalloutStrong => (13.0, Weight::Semibold),
        Role::Footnote => (12.0, Weight::Regular),
        Role::Caption => (11.0, Weight::Regular),
        Role::Numeric => (13.0, Weight::Semibold),
    }
}

pub fn font(role: Role) -> FontId {
    let (size, weight) = spec(role);
    let family = match weight {
        Weight::Regular => FontFamily::Proportional,
        Weight::Semibold => FontFamily::Name(fonts::SEMIBOLD.into()),
        Weight::Bold => FontFamily::Name(fonts::BOLD.into()),
    };
    FontId::new(size, family)
}

/// The "Normal" track of Polaris's `trak` table: `(point size, font units per 2048-unit em)`.
const POLARIS_TRACK: [(f32, f32); 14] = [
    (9.0, 130.0),
    (10.0, 116.0),
    (11.0, 104.0),
    (12.0, 92.0),
    (13.0, 80.0),
    (14.0, 70.0),
    (15.0, 60.0),
    (16.0, 52.0),
    (17.0, 45.0),
    (20.0, 37.0),
    (22.0, 32.0),
    (28.0, 26.0),
    (32.0, 24.0),
    (36.0, 22.0),
];

fn tracking_units(size: f32) -> f32 {
    if size <= POLARIS_TRACK[0].0 {
        return POLARIS_TRACK[0].1;
    }
    for pair in POLARIS_TRACK.windows(2) {
        let (s0, u0) = pair[0];
        let (s1, u1) = pair[1];
        if size <= s1 {
            let t = (size - s0) / (s1 - s0);
            return u0 + (u1 - u0) * t;
        }
    }
    POLARIS_TRACK[POLARIS_TRACK.len() - 1].1
}

/// Extra letter spacing in points for `size`, linearly interpolated and clamped at both ends.
pub fn polaris_tracking(size: f32) -> f32 {
    tracking_units(size) / 2048.0 * size
}

pub fn format(role: Role, color: Color32, look: &Look) -> TextFormat {
    let (size, _) = spec(role);
    TextFormat {
        font_id: font(role),
        color,
        extra_letter_spacing: if look.tracking { polaris_tracking(size) } else { 0.0 },
        ..Default::default()
    }
}

pub fn rich(text: impl Into<String>, role: Role, look: &Look) -> RichText {
    let (size, _) = spec(role);
    let spacing = if look.tracking { polaris_tracking(size) } else { 0.0 };
    RichText::new(text).font(font(role)).extra_letter_spacing(spacing)
}

pub fn paint(
    painter: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    role: Role,
    color: Color32,
    look: &Look,
) -> Rect {
    let mut job = LayoutJob::default();
    job.append(text, 0.0, format(role, color, look));
    let galley = painter.layout_job(job);
    let rect = anchor.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Typeface;

    #[test]
    fn every_role_maps_to_a_registered_font_family() {
        for face in [Typeface::Polaris, Typeface::System] {
            let (defs, _) = fonts::definitions(face, fonts::SystemFonts::default());
            for role in Role::ALL {
                let id = font(role);
                assert!(defs.families.contains_key(&id.family), "missing family {:?} for {face:?}", id.family);
            }
        }
    }

    #[test]
    fn polaris_tracking_interpolates_and_clamps() {
        let approx = |a: f32, b: f32| assert!((a - b).abs() < 1e-3, "{a} != {b}");
        approx(polaris_tracking(12.0), 0.539);
        approx(polaris_tracking(14.0), 0.479);
        approx(polaris_tracking(17.0), 0.374);

        let mid = polaris_tracking(18.5);
        let at_17 = polaris_tracking(17.0);
        let at_20 = polaris_tracking(20.0);
        assert!(mid < at_17 && mid > at_20, "expected {mid} between {at_20} and {at_17}");

        approx(polaris_tracking(4.0), POLARIS_TRACK[0].1 / 2048.0 * 4.0);
        approx(polaris_tracking(100.0), POLARIS_TRACK[POLARIS_TRACK.len() - 1].1 / 2048.0 * 100.0);
    }

    #[test]
    fn format_zeroes_tracking_when_disabled() {
        let mut look = Look {
            p: crate::ui::theme::palette(true, false, crate::settings::Accent::Violet, false),
            m: crate::ui::theme::metrics(crate::settings::Density::Comfortable),
            icons: true,
            tracking: true,
        };
        assert!(format(Role::Body, Color32::WHITE, &look).extra_letter_spacing > 0.0);
        look.tracking = false;
        assert_eq!(format(Role::Body, Color32::WHITE, &look).extra_letter_spacing, 0.0);
    }
}
