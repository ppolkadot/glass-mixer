//! User interface: design system, widgets and views.

pub mod mixer;
pub mod settings_view;
pub mod theme;
pub mod widgets;

/// Everything a view needs to know about the current appearance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub p: theme::Palette,
    pub m: theme::Metrics,
    /// Whether the Segoe icon font is available (otherwise text fallbacks are drawn).
    pub icons: bool,
}

/// Why the glass effect is or isn't showing; explained in Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlassStatus {
    Active,
    TurnedOff,
    TransparencyOff,
    HighContrast,
    Unsupported,
}

impl GlassStatus {
    pub fn description(self) -> &'static str {
        match self {
            GlassStatus::Active => "Translucent window using the Windows backdrop",
            GlassStatus::TurnedOff => "Using a solid background",
            GlassStatus::TransparencyOff => "Windows transparency effects are off, so a solid background is used",
            GlassStatus::HighContrast => "High contrast is on, so a solid background is used",
            GlassStatus::Unsupported => "Not supported on this version of Windows; using a solid background",
        }
    }
}
