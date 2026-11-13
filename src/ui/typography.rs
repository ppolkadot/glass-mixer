//! Typography roles, sizes and weight mapping.

use crate::platform::fonts;
use egui::{FontFamily, FontId};

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
}
