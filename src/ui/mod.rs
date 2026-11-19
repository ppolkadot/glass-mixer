//! User interface: design system, widgets and views.

pub mod mixer;
pub mod presets;
pub mod settings_view;
#[cfg(test)]
pub(crate) mod test_support;
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
    RendererFallback,
}

impl GlassStatus {
    pub fn description(self) -> &'static str {
        match self {
            GlassStatus::Active => "Translucent window using the Windows backdrop",
            GlassStatus::TurnedOff => "Using a solid background",
            GlassStatus::TransparencyOff => "Windows transparency is off; using a solid background",
            GlassStatus::HighContrast => "High contrast is on; using a solid background",
            GlassStatus::Unsupported => "Unsupported on this version of Windows",
            GlassStatus::RendererFallback => "Fallback renderer active; using a solid background",
        }
    }
}
